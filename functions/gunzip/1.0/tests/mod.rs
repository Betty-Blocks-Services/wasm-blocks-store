mod bindings {
    wasmtime_testing_helper::bindgen!("main");

    wasmtime_testing_helper::setup!(Main);
}

use std::io::Write;
use std::process::{Command, Stdio};

use base64::{Engine, engine::general_purpose::STANDARD};

fn gunzip(data: &str, input_encoding: &str, output_encoding: &str) -> Result<String, String> {
    let harness = bindings::harness();
    let mut component = bindings::instantiate(harness);
    component
        .component
        .betty_blocks_gunzip_gunzip()
        .call_gunzip(
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

fn gzip_natively(data: &[u8]) -> Vec<u8> {
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(data).unwrap();
    encoder.finish().unwrap()
}

/// Compresses with the system's `gzip`, so the step is checked against real-world gzip output.
fn gzip_with_system_gzip(data: &[u8]) -> Vec<u8> {
    let mut child = Command::new("gzip")
        .arg("-c")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("the `gzip` command must be installed to run this test");
    let mut stdin = child.stdin.take().unwrap();
    let input = data.to_vec();
    let writer = std::thread::spawn(move || stdin.write_all(&input).unwrap());
    let output = child.wait_with_output().unwrap();
    writer.join().unwrap();
    assert!(output.status.success(), "gzip failed");
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
        let compressed = STANDARD.encode(gzip_natively(input.as_bytes()));
        assert_eq!(gunzip(&compressed, "base64", "utf8").unwrap(), input);
    }
}

#[test]
fn decompresses_system_gzip_output() {
    let xml = large_xml();
    let compressed = STANDARD.encode(gzip_with_system_gzip(xml.as_bytes()));
    assert_eq!(gunzip(&compressed, "base64", "utf8").unwrap(), xml);
}

#[test]
fn binary_content_comes_back_as_base64() {
    let bytes: Vec<u8> = (0..=255).collect();
    let compressed = STANDARD.encode(gzip_natively(&bytes));
    assert_eq!(
        gunzip(&compressed, "base64", "base64").unwrap(),
        STANDARD.encode(&bytes)
    );
    let error = gunzip(&compressed, "base64", "utf8").unwrap_err();
    assert!(error.contains("not valid UTF-8"), "error was: {error}");
}

#[test]
fn non_gzip_input_is_an_error_not_a_trap() {
    let error = gunzip(&STANDARD.encode("<Document/>"), "base64", "utf8").unwrap_err();
    assert!(error.contains("not gzip data"), "error was: {error}");
}

#[test]
fn invalid_base64_is_an_error_not_a_trap() {
    let error = gunzip("H4sI %% broken", "base64", "utf8").unwrap_err();
    assert!(error.contains("not valid base64"), "error was: {error}");
}
