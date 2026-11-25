//! Runs the built component (`target/wasm32-wasip2/release/export_records.wasm`, from `just build`)
//! with a mocked Data API and a mocked upload-file, to check the parts the unit tests cannot: the
//! WIT parameters, the host calls and what reaches upload-file.

mod bindings {
    wasmtime_testing_helper::bindgen!("export-records-world");

    wasmtime_testing_helper::setup!(ExportRecordsWorld);
}

use std::sync::{Arc, Mutex};

use base64::{Engine, engine::general_purpose::STANDARD as BASE64};
use bindings::betty_blocks_types::data_api::data_api::HelperContext;
use bindings::betty_blocks_types::types::types::{BettyModel, BettyProperty};
use bindings::betty_blocks_types::upload_file::upload_file::{Input, UploadResult};
use bindings::exports::betty_blocks::export_records::export_records::{
    Delimiter, FileType, SortOrder,
};

const DATA_API: &str = "betty-blocks-types:data-api/data-api@3.0.0";
const UPLOAD_FILE: &str = "betty-blocks-types:upload-file/upload-file@4.0.0";

fn helper_context() -> HelperContext {
    HelperContext {
        application_id: String::from("app"),
        action_id: String::from("action"),
        log_id: String::from("log"),
        encrypted_configurations: None,
        jwt: None,
    }
}

fn skip_of(query: &str) -> usize {
    query
        .split("skip: ")
        .nth(1)
        .and_then(|rest| rest.split(',').next())
        .and_then(|skip| skip.parse().ok())
        .expect("the query has a skip")
}

struct Run {
    result: Result<String, String>,
    queries: Vec<String>,
    upload: Option<Input>,
}

/// Exports `total` Person records to CSV through a Data API that returns at most `page_cap`
/// records per page.
fn run(total: usize, page_cap: usize, names: fn(usize) -> String) -> Run {
    let queries = Arc::new(Mutex::new(Vec::new()));
    let upload = Arc::new(Mutex::new(None));

    let mut harness = bindings::harness();
    let seen = Arc::clone(&queries);
    harness.mock(
        DATA_API,
        "request",
        move |_context, (_helper, query, _variables): (HelperContext, String, String)| {
            let skip = skip_of(&query);
            seen.lock().unwrap().push(query);
            let count = total.saturating_sub(skip).min(page_cap);
            let results: Vec<serde_json::Value> = (skip..skip + count)
                .map(|id| serde_json::json!({"id": id, "name": names(id)}))
                .collect();
            let body = serde_json::json!({
                "data": {"allPerson": {"results": results, "totalCount": total}}
            });
            Ok((Ok::<String, String>(body.to_string()),))
        },
    );
    let stored = Arc::clone(&upload);
    harness.mock(
        UPLOAD_FILE,
        "upload",
        move |_context, (_helper, input): (HelperContext, Input)| {
            let file_size = input.file_base64.len() as u64;
            *stored.lock().unwrap() = Some(input);
            Ok((Ok::<UploadResult, String>(UploadResult {
                reference: String::from("file-reference"),
                file_size,
                message: None,
            }),))
        },
    );

    let mut component = bindings::instantiate(harness);
    let result = component
        .component
        .betty_blocks_export_records_export_records()
        .call_export_records(
            &mut component.store,
            &helper_context(),
            FileType::Csv,
            &BettyModel {
                name: String::from("Person"),
            },
            None,
            &String::from("{}"),
            None,
            SortOrder::Asc,
            Delimiter::Semicolon,
            &BettyModel {
                name: String::from("Export"),
            },
            &[BettyProperty {
                name: String::from("file"),
            }],
            Some("people"),
            &String::from(r#"{"Id": "id", "Name": "name"}"#),
            &String::from("{}"),
            Some(false),
            None,
        )
        .expect("the component call traps");

    let queries = queries.lock().unwrap().clone();
    let upload = upload.lock().unwrap().take();
    Run {
        result,
        queries,
        upload,
    }
}

fn csv_of(input: &Input) -> String {
    String::from_utf8(BASE64.decode(&input.file_base64).unwrap()).unwrap()
}

#[test]
fn exports_every_page_and_uploads_the_csv() {
    let run = run(250, 100, |id| format!("Person {id}"));

    assert_eq!(run.result, Ok(String::from("file-reference")));
    let skips: Vec<usize> = run.queries.iter().map(|q| skip_of(q)).collect();
    assert_eq!(skips, [0, 100, 200]);
    assert!(
        run.queries[0].contains("sort: { field: id, order: ASC }"),
        "{}",
        run.queries[0]
    );

    let input = run.upload.expect("upload-file was called");
    assert_eq!(input.full_filename, "people.csv");
    assert_eq!(input.model.name, "Export");
    assert_eq!(input.property.name, "file");
    let text = csv_of(&input);
    let lines: Vec<&str> = text.split("\r\n").collect();
    assert_eq!(
        lines.len(),
        252,
        "header + 250 rows + the empty text after the last \\r\\n"
    );
    assert_eq!(lines[0], "Id;Name");
    assert_eq!(lines[1], "0;\"Person 0\"");
    assert_eq!(lines[250], "249;\"Person 249\"");
}

#[test]
fn escapes_formulas_by_default() {
    let run = run(1, 200, |_| String::from("=HYPERLINK(\"http://x\")"));

    let text = csv_of(&run.upload.expect("upload-file was called"));
    assert_eq!(text, "Id;Name\r\n0;\"'=HYPERLINK(\"\"http://x\"\")\"\r\n");
}
