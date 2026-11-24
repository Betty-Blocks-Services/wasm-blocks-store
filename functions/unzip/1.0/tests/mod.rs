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

/// A ZIP made by the system's `zip` command, so the step is checked against real-world archives.
fn system_zip(files: &[(&str, &[u8])]) -> Vec<u8> {
    let directory = std::env::temp_dir().join(format!("unzip-step-test-{}", std::process::id()));
    std::fs::create_dir_all(directory.join("folder")).unwrap();
    for (name, content) in files {
        std::fs::write(directory.join(name), content).unwrap();
    }
    let status = std::process::Command::new("zip")
        .current_dir(&directory)
        .args(["-q", "-r", "archive.zip", "."])
        .status()
        .expect("the `zip` command must be installed to run this test");
    assert!(status.success(), "zip failed");
    let bytes = std::fs::read(directory.join("archive.zip")).unwrap();
    std::fs::remove_dir_all(&directory).unwrap();
    bytes
}

/// Runs the step with `zip_file` as the option value; `/archive.zip` serves `archive`.
fn run_unzip(zip_file: &str, archive: Vec<u8>) -> (Result<String, String>, Vec<Input>) {
    let uploads = Arc::new(Mutex::new(Vec::new()));
    let captured = uploads.clone();
    let mut harness = bindings::harness();
    harness.mock_http_handler(Box::new(move |request, _config| {
        let found = request.uri().path() == "/archive.zip";
        let archive = archive.clone();
        Box::pin(async move {
            request.into_body().await?;
            Ok(if found {
                hyper::Response::new(hyper::body::Bytes::from(archive))
            } else {
                hyper::Response::builder()
                    .status(403)
                    .body(hyper::body::Bytes::new())
                    .unwrap()
            })
        })
    }));
    harness.mock(
        UPLOAD,
        "upload",
        move |_context, (_helper, input): (HelperContext, Input)| {
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
    let archive = system_zip(&[("a.txt", b"hello"), ("folder/b.txt", b"world")]);
    let (result, uploads) = run_unzip(FILE_VALUE, archive);
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
fn a_failed_download_is_an_error() {
    let (result, _) = run_unzip(
        r#"{"url":"https://assets.example/expired.zip"}"#,
        Vec::new(),
    );
    let error = result.unwrap_err();
    assert!(error.contains("HTTP status 403"), "error was: {error}");
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
