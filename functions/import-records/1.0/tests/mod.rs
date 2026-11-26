//! Runs the built component (`target/wasm32-wasip2/release/import_records.wasm`, from `just build`)
//! with a stubbed file download and a mocked Data API, to check what the unit tests cannot: the WIT
//! parameters, the `wasi:http` download and the requests that reach the Data API.

mod bindings {
    wasmtime_testing_helper::bindgen!("import-records-world");

    wasmtime_testing_helper::setup!(ImportRecordsWorld);
}

use std::sync::{Arc, Mutex};

use bindings::betty_blocks_types::data_api::data_api::HelperContext;
use bindings::betty_blocks_types::types::types::BettyModel;
use bindings::exports::betty_blocks::import_records::import_records::{
    Delimiter, FileType, UniqueType,
};
use serde_json::{Value, json};
use wasmtime_testing_helper::http::hyper;

const DATA_API: &str = "betty-blocks-types:data-api/data-api@3.0.0";

fn helper_context() -> HelperContext {
    HelperContext {
        application_id: String::from("app"),
        action_id: String::from("action"),
        log_id: String::from("log"),
        encrypted_configurations: None,
        jwt: None,
    }
}

struct Run {
    result: Result<String, String>,
    requests: Vec<(String, Value)>,
}

/// Imports `file` (served with HTTP `status`, from a URL with spaces, as a file property URL can
/// have) into Person, deduplicating on Email. The Data API has
/// one Person, with the email `ann@x.nl` and id 41.
fn run(status: u16, file: &'static str, deduplicate: bool) -> Run {
    let requests = Arc::new(Mutex::new(Vec::new()));

    let mut harness = bindings::harness();
    harness.stub_http_handler(Ok(hyper::Response::builder()
        .status(status)
        .body(file)
        .unwrap()));
    let seen = Arc::clone(&requests);
    harness.mock(
        DATA_API,
        "request",
        move |_context, (_helper, query, variables): (HelperContext, String, String)| {
            let variables: Value = serde_json::from_str(&variables).unwrap();
            seen.lock()
                .unwrap()
                .push((query.clone(), variables.clone()));
            let body = if query.starts_with("query") {
                let asked = &variables["where"]["email"]["in"];
                let results: Vec<Value> = asked
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|email| *email == "ann@x.nl")
                    .map(|_| json!({"id": 41, "email": "ann@x.nl"}))
                    .collect();
                json!({"data": {"allPerson": {"totalCount": results.len(), "results": results}}})
            } else {
                let name = if query.contains("createMany") {
                    "createManyPerson"
                } else {
                    "upsertManyPerson"
                };
                let count = variables["input"].as_array().unwrap().len();
                json!({"data": {name: vec![json!({"id": 1}); count]}})
            };
            Ok((Ok::<String, String>(body.to_string()),))
        },
    );

    let mut component = bindings::instantiate(harness);
    let result = component
        .component
        .betty_blocks_import_records_import_records()
        .call_import_records(
            &mut component.store,
            &helper_context(),
            &String::from("https://files.example/files/people import"),
            FileType::Csv,
            Delimiter::Auto,
            &BettyModel {
                name: String::from("Person"),
            },
            &String::from(r#"{"Email*": "email", "Age": "age"}"#),
            &String::from(r#"{"Age": "number"}"#),
            &String::from(r#"{"source": "csv"}"#),
            Some(deduplicate),
            Some("Email"),
            UniqueType::Text,
            &String::from("null"),
            Some(true),
        )
        .expect("the component call traps");

    let requests = requests.lock().unwrap().clone();
    Run { result, requests }
}

#[test]
fn downloads_the_file_and_creates_and_updates_records() {
    let run = run(200, "Email;Age\nann@x.nl;31\nbob@x.nl;40\n", true);

    assert_eq!(
        run.result,
        Ok(String::from("Records created: 1, records updated: 1"))
    );
    assert_eq!(run.requests.len(), 3, "{:?}", run.requests);
    assert_eq!(
        run.requests[0].1,
        json!({"where": {"email": {"in": ["ann@x.nl", "bob@x.nl"]}}})
    );
    assert_eq!(
        run.requests[1],
        (
            String::from("mutation { createManyPerson(input: $input) { id } }"),
            json!({"input": [{"email": "bob@x.nl", "age": 40, "source": "csv"}]})
        )
    );
    assert_eq!(
        run.requests[2],
        (
            String::from("mutation { upsertManyPerson(input: $input) { id } }"),
            json!({"input": [{"id": 41, "email": "ann@x.nl", "age": 31, "source": "csv"}]})
        )
    );
}

#[test]
fn a_failed_download_is_reported_and_nothing_is_written() {
    let run = run(403, "Forbidden", false);

    let error = run.result.unwrap_err();
    assert!(error.contains("HTTP 403"), "{error}");
    assert!(run.requests.is_empty());
}

#[test]
fn a_missing_required_column_is_reported_and_nothing_is_written() {
    let run = run(200, "Name;Age\nAnn;31\n", false);

    assert_eq!(
        run.result,
        Err(String::from(
            "Required columns are missing from the file: Email. Nothing was imported"
        ))
    );
    assert!(run.requests.is_empty());
}
