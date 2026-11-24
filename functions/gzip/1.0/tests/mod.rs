mod bindings {
    wasmtime_testing_helper::bindgen!("main");

    wasmtime_testing_helper::setup!(Main);
}

use std::io::{Read, Write};
use std::process::{Command, Stdio};

use base64::{Engine, engine::general_purpose::STANDARD};

fn gzip(data: &str, input_encoding: &str, output_encoding: &str) -> Result<String, String> {
    let harness = bindings::harness();
    let mut component = bindings::instantiate(harness);
    component
        .component
        .betty_blocks_gzip_gzip()
        .call_gzip(
            &mut component.store,
            Some(data),
            input_encoding,
            output_encoding,
        )
        .unwrap()
}

/// Roughly 1 MB of pain.001-like XML with multi-byte characters.
fn large_xml() -> String {
    let mut xml = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<Document>\n");
    let mut index = 0;
    while xml.len() < 1_000_000 {
        xml.push_str(&format!(
            "  <CdtTrfTxInf><EndToEndId>E2E-{index:08}</EndToEndId><Amt Ccy=\"EUR\">{}.{:02}</Amt><Nm>Café Müller – Ørsted №{index}</Nm></CdtTrfTxInf>\n",
            index % 9973,
            index % 100
        ));
        index += 1;
    }
    xml.push_str("</Document>\n");
    xml
}

fn gunzip_natively(compressed: &[u8]) -> Vec<u8> {
    let mut output = Vec::new();
    flate2::read::MultiGzDecoder::new(compressed)
        .read_to_end(&mut output)
        .unwrap();
    output
}

/// Decompresses with the system's `gzip -d`, the check the receiving side will effectively do.
fn gunzip_with_system_gzip(compressed: &[u8]) -> Vec<u8> {
    let mut child = Command::new("gzip")
        .arg("-dc")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("the `gzip` command must be installed to run this test");
    let mut stdin = child.stdin.take().unwrap();
    let input = compressed.to_vec();
    let writer = std::thread::spawn(move || stdin.write_all(&input).unwrap());
    let output = child.wait_with_output().unwrap();
    writer.join().unwrap();
    assert!(output.status.success(), "gzip -d failed");
    output.stdout
}

#[test]
fn round_trips_through_a_real_component_instance() {
    for input in [
        String::new(),
        String::from("Hello, world!"),
        String::from("Grüße – 日本語 – 🚀 – €12,50"),
        large_xml(),
    ] {
        let result = gzip(&input, "utf8", "base64").unwrap();
        assert!(result.starts_with("H4sI"), "result was: {result}");
        let compressed = STANDARD.decode(&result).unwrap();
        assert_eq!(gunzip_natively(&compressed), input.as_bytes());
    }
}

#[test]
fn system_gzip_decompresses_the_output() {
    let xml = large_xml();
    let compressed = STANDARD
        .decode(gzip(&xml, "utf8", "base64").unwrap())
        .unwrap();
    assert!(
        compressed.len() < xml.len() / 4,
        "the XML barely compressed"
    );
    assert_eq!(gunzip_with_system_gzip(&compressed), xml.as_bytes());
}

#[test]
fn output_is_deterministic() {
    let xml = large_xml();
    assert_eq!(
        gzip(&xml, "utf8", "base64").unwrap(),
        gzip(&xml, "utf8", "base64").unwrap()
    );
}

#[test]
fn base64_input_gives_the_same_result_as_raw_text() {
    let xml = large_xml();
    assert_eq!(
        gzip(&STANDARD.encode(&xml), "base64", "base64").unwrap(),
        gzip(&xml, "utf8", "base64").unwrap()
    );
}

#[test]
fn a_missing_value_is_treated_as_empty() {
    let harness = bindings::harness();
    let mut component = bindings::instantiate(harness);
    let result = component
        .component
        .betty_blocks_gzip_gzip()
        .call_gzip(&mut component.store, None, "utf8", "base64")
        .unwrap()
        .unwrap();
    assert!(gunzip_natively(&STANDARD.decode(result).unwrap()).is_empty());
}

#[test]
fn invalid_base64_is_an_error_not_a_trap() {
    let error = gzip("this is %% not base64", "base64", "base64").unwrap_err();
    assert!(error.contains("not valid base64"), "error was: {error}");
}
