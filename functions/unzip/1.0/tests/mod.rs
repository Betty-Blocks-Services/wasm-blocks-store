mod bindings {
    wasmtime_testing_helper::bindgen!("unzip-world");

    wasmtime_testing_helper::setup!(UnzipWorld);
}

use std::io::{Cursor, Write};
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

fn make_zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    for (name, content) in entries {
        writer.start_file(*name, options).unwrap();
        writer.write_all(content).unwrap();
    }
    writer.finish().unwrap().into_inner()
}

/// A ZIP made once by the system's `zip` command (Info-ZIP 3.0: `a.txt`, `folder/`,
/// `folder/b.txt`), so the step is checked against a real-world archive without needing `zip`.
const SYSTEM_ZIP: &[u8] = include_bytes!("fixtures/system-zip.zip");

/// How the mocks behave in one run.
#[derive(Default)]
struct Mocks {
    /// The upload with this number (1 for the first) fails with the storage's size error.
    fail_upload: Option<usize>,
    /// A `Content-Length` header to send instead of the real one.
    content_length: Option<u64>,
}

/// Runs the step with `zip_file` as the option value; `/archive.zip` serves `archive`.
fn run_unzip(zip_file: &str, archive: Vec<u8>) -> (Result<String, String>, Vec<Input>) {
    run_unzip_with(zip_file, archive, Mocks::default())
}

fn run_unzip_with(
    zip_file: &str,
    archive: Vec<u8>,
    mocks: Mocks,
) -> (Result<String, String>, Vec<Input>) {
    let uploads = Arc::new(Mutex::new(Vec::new()));
    let captured = uploads.clone();
    let attempts = Arc::new(Mutex::new(0));
    let mut harness = bindings::harness();
    let content_length = mocks.content_length;
    harness.mock_http_handler(Box::new(move |request, _config| {
        let found = request.uri().path() == "/archive.zip";
        let archive = archive.clone();
        Box::pin(async move {
            request.into_body().await?;
            Ok(if found {
                let mut response = hyper::Response::builder();
                if let Some(length) = content_length {
                    response = response.header("content-length", length);
                }
                response.body(hyper::body::Bytes::from(archive)).unwrap()
            } else {
                hyper::Response::builder()
                    .status(403)
                    .body(hyper::body::Bytes::new())
                    .unwrap()
            })
        })
    }));
    let fail_upload = mocks.fail_upload;
    harness.mock(
        UPLOAD,
        "upload",
        move |_context, (_helper, input): (HelperContext, Input)| {
            let mut attempts = attempts.lock().unwrap();
            *attempts += 1;
            if Some(*attempts) == fail_upload {
                return Ok((Err::<UploadResult, _>(String::from(
                    "upload failed with status 400: <Error><Code>EntityTooLarge</Code><ProposedSize>2097152</ProposedSize><MaxSizeAllowed>1048576</MaxSizeAllowed></Error>",
                )),));
            }
            let mut uploads = captured.lock().unwrap();
            uploads.push(input);
            Ok((Ok::<_, String>(UploadResult {
                reference: format!("reference-{}", uploads.len()),
                file_size: 0,
                message: None,
            }),))
        },
    );
    let mut component = bindings::instantiate(harness);
    let result = component
        .component
        .betty_blocks_unzip_unzip()
        .call_unzip(
            &mut component.store,
            &helper_context(),
            &zip_file.to_string(),
            &BettyModel {
                name: String::from("Attachment"),
            },
            &[BettyProperty {
                name: String::from("file"),
            }],
        )
        .unwrap();
    let uploads = std::mem::take(&mut *uploads.lock().unwrap());
    (result, uploads)
}

const FILE_VALUE: &str =
    r#"{"name":"archive.zip","url":"https://assets.example/archive.zip?signature=x"}"#;

fn uploaded(uploads: &[Input]) -> Vec<(String, Vec<u8>)> {
    uploads
        .iter()
        .map(|input| {
            (
                input.full_filename.clone(),
                STANDARD.decode(&input.file_base64).unwrap(),
            )
        })
        .collect()
}

#[test]
fn extracts_every_file_and_returns_their_references() {
    let xml = "<Document>Grüße</Document>".repeat(1000);
    let archive = make_zip(&[
        ("invoices/a.xml", xml.as_bytes()),
        ("b.bin", &[0, 159, 146, 150]),
        ("__MACOSX/invoices/._a.xml", b"metadata"),
    ]);
    let (result, uploads) = run_unzip(FILE_VALUE, archive);
    let references: serde_json::Value = serde_json::from_str(&result.unwrap()).unwrap();
    assert_eq!(
        references,
        serde_json::json!(["reference-1", "reference-2"])
    );
    assert_eq!(
        uploaded(&uploads),
        [
            (String::from("a.xml"), xml.into_bytes()),
            (String::from("b.bin"), vec![0, 159, 146, 150]),
        ]
    );
    assert!(
        uploads
            .iter()
            .all(|input| input.model.name == "Attachment" && input.property.name == "file")
    );
}

#[test]
fn extracts_an_archive_made_by_the_system_zip() {
    let (result, uploads) = run_unzip(FILE_VALUE, SYSTEM_ZIP.to_vec());
    assert!(result.is_ok(), "result was: {result:?}");
    let mut files = uploaded(&uploads);
    files.sort();
    assert_eq!(
        files,
        [
            (String::from("a.txt"), b"hello".to_vec()),
            (String::from("b.txt"), b"world".to_vec()),
        ]
    );
}

#[test]
fn accepts_a_plain_url_as_json_text() {
    let archive = make_zip(&[("a.txt", b"hello")]);
    let (result, uploads) = run_unzip(r#""https://assets.example/archive.zip""#, archive);
    assert_eq!(result.unwrap(), r#"["reference-1"]"#);
    assert_eq!(uploads.len(), 1);
}

#[test]
fn an_empty_zip_gives_an_empty_list() {
    let (result, uploads) = run_unzip(FILE_VALUE, make_zip(&[]));
    assert_eq!(result.unwrap(), "[]");
    assert!(uploads.is_empty());
}

#[test]
fn a_file_that_is_not_a_zip_is_an_error() {
    let (result, uploads) = run_unzip(FILE_VALUE, b"<Document/>".to_vec());
    let error = result.unwrap_err();
    assert!(error.contains("not a valid ZIP"), "error was: {error}");
    assert!(uploads.is_empty());
}

#[test]
fn a_failed_download_is_an_error_without_the_signature() {
    let (result, _) = run_unzip(
        r#"{"url":"https://assets.example/expired.zip?X-Amz-Signature=secret"}"#,
        Vec::new(),
    );
    let error = result.unwrap_err();
    assert!(error.contains("HTTP status 403"), "error was: {error}");
    assert!(
        error.contains("https://assets.example/expired.zip\""),
        "error was: {error}"
    );
    assert!(!error.contains("secret"), "error was: {error}");
}

#[test]
fn a_download_over_the_limit_is_refused() {
    let (result, uploads) = run_unzip(FILE_VALUE, vec![0; 64 * 1024 * 1024 + 1]);
    let error = result.unwrap_err();
    assert!(error.contains("goes over"), "error was: {error}");
    assert!(uploads.is_empty());
}

#[test]
fn a_zip_bomb_after_a_good_file_uploads_nothing() {
    let bomb = vec![0; 64 * 1024 * 1024];
    let archive = make_zip(&[("a.txt", b"hello"), ("bomb.bin", &bomb)]);
    let (result, uploads) = run_unzip(FILE_VALUE, archive);
    let error = result.unwrap_err();
    assert!(
        error.contains("extracted files are larger"),
        "error was: {error}"
    );
    assert!(uploads.is_empty(), "uploaded: {}", uploads.len());
}

#[test]
fn no_file_is_an_error() {
    let (result, _) = run_unzip("null", Vec::new());
    let error = result.unwrap_err();
    assert!(
        error.contains("No ZIP file was found"),
        "error was: {error}"
    );
}

#[test]
fn a_content_length_over_the_limit_is_refused_before_reading() {
    // The header claims more than the limit while the body is a small, valid ZIP: only the
    // Content-Length check can fail this run. Without it, the step would extract `a.txt`.
    let archive = make_zip(&[("a.txt", b"hello")]);
    let (result, uploads) = run_unzip_with(
        FILE_VALUE,
        archive,
        Mocks {
            content_length: Some(64 * 1024 * 1024 + 1),
            ..Mocks::default()
        },
    );
    let error = result.unwrap_err();
    assert!(error.contains("(67108865 bytes)"), "error was: {error}");
    assert!(uploads.is_empty());
}

#[test]
fn a_failed_upload_names_the_files_already_uploaded() {
    let archive = make_zip(&[("a.txt", b"1"), ("b.txt", b"2"), ("c.txt", b"3")]);
    let (result, uploads) = run_unzip_with(
        FILE_VALUE,
        archive,
        Mocks {
            fail_upload: Some(2),
            ..Mocks::default()
        },
    );
    let error = result.unwrap_err();
    assert!(
        error.starts_with("storing \"b.txt\" failed: it is larger than the maximum file size of \"Property to save the extracted files into\" (2.0 MB while it accepts 1.0 MB)"),
        "error was: {error}"
    );
    assert!(
        error.contains("1 of the 3 files were already uploaded (\"a.txt\")"),
        "error was: {error}"
    );
    assert_eq!(uploads.len(), 1);
}

#[test]
fn files_with_the_same_name_in_different_folders_get_unique_names() {
    let archive = make_zip(&[("2023/report.pdf", b"old"), ("2024/report.pdf", b"new")]);
    let (result, uploads) = run_unzip(FILE_VALUE, archive);
    assert!(result.is_ok(), "result was: {result:?}");
    assert_eq!(
        uploaded(&uploads),
        [
            (String::from("report.pdf"), b"old".to_vec()),
            (String::from("report_1.pdf"), b"new".to_vec()),
        ]
    );
}
