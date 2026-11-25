mod bindings {
    wasmtime_testing_helper::bindgen!("zip-world");

    wasmtime_testing_helper::setup!(ZipWorld);
}

use std::io::{Cursor, Read};
use std::sync::{Arc, Mutex};

use base64::{Engine, engine::general_purpose::STANDARD};
use wasmtime_testing_helper::http::hyper;

use bindings::betty_blocks_types::data_api::data_api::HelperContext;
use bindings::betty_blocks_types::types::types::{BettyModel, BettyProperty};
use bindings::betty_blocks_types::upload_file::upload_file::{Input, UploadResult};

const UPLOAD: &str = "betty-blocks-types:upload-file/upload-file@4.0.0";

fn helper_context() -> HelperContext {
    HelperContext {
        application_id: String::from("app"),
        action_id: String::from("action"),
        log_id: String::from("log"),
        encrypted_configurations: None,
        jwt: None,
    }
}

fn property(name: &str) -> Vec<BettyProperty> {
    vec![BettyProperty {
        name: String::from(name),
    }]
}

/// Serves `/<name>` with the bytes of that file, and 404 for anything else. With
/// `claimed_length`, every file is sent with that `Content-Length` header instead of its own.
fn serve(
    files: Vec<(&'static str, Vec<u8>)>,
    claimed_length: Option<u64>,
) -> wasmtime_testing_helper::http::HttpHandler {
    Box::new(move |request, _config| {
        let path = request.uri().path().trim_start_matches('/').to_string();
        let found = files
            .iter()
            .find(|(name, _)| *name == path)
            .map(|(_, bytes)| bytes.clone());
        Box::pin(async move {
            // Read the request body first, as a real server would; the client waits for that.
            request.into_body().await?;
            Ok(match found {
                Some(bytes) => {
                    let mut response = hyper::Response::builder();
                    if let Some(length) = claimed_length {
                        response = response.header("content-length", length);
                    }
                    response.body(hyper::body::Bytes::from(bytes)).unwrap()
                }
                None => hyper::Response::builder()
                    .status(404)
                    .body(hyper::body::Bytes::new())
                    .unwrap(),
            })
        })
    })
}

/// Runs the step against the served files and returns its result and every upload it made.
fn run_zip(
    records: Vec<serde_json::Value>,
    files: Vec<(&'static str, Vec<u8>)>,
    file_name: Option<&str>,
) -> (Result<String, String>, Vec<Input>) {
    run_zip_with(records, files, file_name, None)
}

fn run_zip_with(
    records: Vec<serde_json::Value>,
    files: Vec<(&'static str, Vec<u8>)>,
    file_name: Option<&str>,
    claimed_length: Option<u64>,
) -> (Result<String, String>, Vec<Input>) {
    let uploads = Arc::new(Mutex::new(Vec::new()));
    let captured = uploads.clone();
    let mut harness = bindings::harness();
    harness.mock_http_handler(serve(files, claimed_length));
    harness.mock(
        UPLOAD,
        "upload",
        move |_context, (_helper, input): (HelperContext, Input)| {
            let size = STANDARD.decode(&input.file_base64).unwrap().len() as u64;
            captured.lock().unwrap().push(input);
            Ok((Ok::<_, String>(UploadResult {
                reference: String::from("reference-1"),
                file_size: size,
                message: None,
            }),))
        },
    );
    let mut component = bindings::instantiate(harness);
    // How a Value option holding a collection is expected to arrive: { data: [record, ...] }.
    let collection = serde_json::json!({ "data": records }).to_string();
    let result = component
        .component
        .betty_blocks_zip_zip()
        .call_zip(
            &mut component.store,
            &helper_context(),
            &collection,
            &property("file"),
            &BettyModel {
                name: String::from("Export"),
            },
            &property("archive"),
            file_name,
        )
        .unwrap();
    let uploads = std::mem::take(&mut *uploads.lock().unwrap());
    (result, uploads)
}

fn entries(input: &Input) -> Vec<(String, Vec<u8>)> {
    let bytes = STANDARD.decode(&input.file_base64).unwrap();
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    (0..archive.len())
        .map(|index| {
            let mut entry = archive.by_index(index).unwrap();
            let mut content = Vec::new();
            entry.read_to_end(&mut content).unwrap();
            (entry.name().unwrap().to_string(), content)
        })
        .collect()
}

fn file(name: &str, path: &str) -> serde_json::Value {
    serde_json::json!({ "name": name, "url": format!("https://assets.example/{path}?signature=x") })
}

#[test]
fn zips_the_files_of_a_collection_and_uploads_the_archive() {
    let xml = "<Document>Grüße</Document>".repeat(1000);
    let (result, uploads) = run_zip(
        vec![
            serde_json::json!({ "id": 1, "file": file("invoice.xml", "a") }),
            serde_json::json!({ "id": 2, "file": null }),
            serde_json::json!({ "id": 3, "file": file("invoice.xml", "b") }),
            serde_json::json!({ "id": 4, "file": file("photo.png", "c") }),
        ],
        vec![
            ("a", xml.clone().into_bytes()),
            ("b", b"second".to_vec()),
            ("c", vec![0, 159, 146, 150]),
        ],
        Some("payments"),
    );
    assert_eq!(result.unwrap(), "reference-1");
    assert_eq!(uploads.len(), 1);
    let upload = &uploads[0];
    assert_eq!(upload.full_filename, "payments.zip");
    assert_eq!(upload.model.name, "Export");
    assert_eq!(upload.property.name, "archive");
    assert_eq!(
        entries(upload),
        [
            (String::from("invoice.xml"), xml.into_bytes()),
            (String::from("invoice_1.xml"), b"second".to_vec()),
            (String::from("photo.png"), vec![0, 159, 146, 150]),
        ]
    );
}

#[test]
fn entries_have_the_fixed_date_and_permissions() {
    let (_, uploads) = run_zip(
        vec![serde_json::json!({ "id": 1, "file": file("a.txt", "a") })],
        vec![("a", b"hello".to_vec())],
        None,
    );
    let bytes = STANDARD.decode(&uploads[0].file_base64).unwrap();
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    let entry = archive.by_index(0).unwrap();
    let date = entry.last_modified().unwrap();
    assert_eq!((date.year(), date.month(), date.day()), (1980, 1, 1));
    assert_eq!(entry.unix_mode().map(|mode| mode & 0o777), Some(0o644));
    assert_eq!(entry.compression(), zip::CompressionMethod::Deflated);
}

/// Removes the directory when dropped, also when the test fails halfway.
struct TempDir(std::path::PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn the_archive_opens_with_the_system_unzip() {
    let (_, uploads) = run_zip(
        vec![serde_json::json!({ "id": 1, "file": file("a.txt", "a") })],
        vec![("a", b"hello".to_vec())],
        None,
    );
    let directory =
        TempDir(std::env::temp_dir().join(format!("zip-step-test-{}", std::process::id())));
    std::fs::create_dir_all(&directory.0).unwrap();
    let archive = directory.0.join("archive.zip");
    std::fs::write(&archive, STANDARD.decode(&uploads[0].file_base64).unwrap()).unwrap();
    let output = std::process::Command::new("unzip")
        .args(["-p"])
        .arg(&archive)
        .arg("a.txt")
        .output()
        .expect("the `unzip` command must be installed to run this test");
    assert!(output.status.success(), "unzip failed");
    assert_eq!(output.stdout, b"hello");
    assert_eq!(uploads[0].full_filename, "zip.zip");
}

#[test]
fn a_failed_download_is_an_error_and_nothing_is_uploaded() {
    let (result, uploads) = run_zip(
        vec![serde_json::json!({ "id": 1, "file": file("a.txt", "missing") })],
        vec![],
        None,
    );
    let error = result.unwrap_err();
    assert!(error.contains("HTTP status 404"), "error was: {error}");
    assert!(uploads.is_empty());
}

#[test]
fn a_wrong_property_is_an_error() {
    let (result, uploads) = run_zip(
        vec![serde_json::json!({ "id": 1, "document": file("a.txt", "a") })],
        vec![],
        None,
    );
    let error = result.unwrap_err();
    assert!(error.contains("no property \"file\""), "error was: {error}");
    assert!(uploads.is_empty());
}

#[test]
fn records_without_any_file_are_an_error_and_nothing_is_uploaded() {
    let (result, uploads) = run_zip(
        vec![
            serde_json::json!({ "id": 1, "file": null }),
            serde_json::json!({ "id": 2, "file": null }),
        ],
        vec![],
        None,
    );
    let error = result.unwrap_err();
    assert!(
        error.contains("0 of 2 records had a file in \"file\""),
        "error was: {error}"
    );
    assert!(uploads.is_empty());
}

#[test]
fn a_file_value_that_is_text_is_an_error_and_nothing_is_uploaded() {
    let (result, uploads) = run_zip(
        vec![serde_json::json!({ "id": 1, "file": "https://assets.example/a" })],
        vec![("a", b"hello".to_vec())],
        None,
    );
    let error = result.unwrap_err();
    assert!(
        error.contains("\"file\" in record 1 is not a file with a URL"),
        "error was: {error}"
    );
    assert!(uploads.is_empty());
}

#[test]
fn an_empty_collection_uploads_an_empty_zip() {
    let (result, uploads) = run_zip(vec![], vec![], Some("nothing"));
    assert_eq!(result.unwrap(), "reference-1");
    assert_eq!(uploads.len(), 1);
    assert_eq!(uploads[0].full_filename, "nothing.zip");
    assert!(entries(&uploads[0]).is_empty());
}

#[test]
fn the_size_limit_counts_all_downloads_together() {
    // Each file is under the 64 MiB limit; together they are over it.
    let size = 33 * 1024 * 1024;
    let (result, uploads) = run_zip(
        vec![
            serde_json::json!({ "id": 1, "file": file("a.bin", "a") }),
            serde_json::json!({ "id": 2, "file": file("b.bin", "b") }),
        ],
        vec![("a", vec![1; size]), ("b", vec![2; size])],
        None,
    );
    let error = result.unwrap_err();
    assert!(
        error.contains("goes over") && error.contains("https://assets.example/b\""),
        "error was: {error}"
    );
    assert!(uploads.is_empty());
}

#[test]
fn a_content_length_over_the_limit_is_refused_before_reading() {
    // The header claims more than the limit while the body is 5 bytes: only the Content-Length
    // check can fail this run.
    let (result, uploads) = run_zip_with(
        vec![serde_json::json!({ "id": 1, "file": file("a.txt", "a") })],
        vec![("a", b"hello".to_vec())],
        None,
        Some(64 * 1024 * 1024 + 1),
    );
    let error = result.unwrap_err();
    assert!(error.contains("(67108865 bytes)"), "error was: {error}");
    assert!(uploads.is_empty());
}

#[test]
fn the_name_option_cannot_point_into_folders() {
    let (_, uploads) = run_zip(
        vec![serde_json::json!({ "id": 1, "file": file("a.txt", "a") })],
        vec![("a", b"hello".to_vec())],
        Some("../../x"),
    );
    assert_eq!(uploads[0].full_filename, "x.zip");
}
