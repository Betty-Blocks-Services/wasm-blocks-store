mod bindings {
    wasmtime_testing_helper::bindgen!("main");

    wasmtime_testing_helper::setup!(Main);
}

fn base64_text(mode: &str, text: Option<&str>, alphabet: &str) -> Result<String, String> {
    let harness = bindings::harness();
    let mut component = bindings::instantiate(harness);
    component
        .component
        .betty_blocks_base64_text_base64_text()
        .call_base64_text(&mut component.store, mode, text, alphabet)
        .unwrap()
}

#[test]
fn encodes_through_a_real_component_instance() {
    // A Basic auth header value for user "Aladdin", password "open sesame" (RFC 7617).
    assert_eq!(
        base64_text("encode", Some("Aladdin:open sesame"), "standard").unwrap(),
        "QWxhZGRpbjpvcGVuIHNlc2FtZQ=="
    );
}

#[test]
fn decodes_a_jwt_payload_through_a_real_component_instance() {
    assert_eq!(
        base64_text("decode", Some("eyJzdWIiOiIxMjM0In0"), "url-safe").unwrap(),
        "{\"sub\":\"1234\"}"
    );
}

#[test]
fn an_empty_text_variable_gives_an_empty_result() {
    assert_eq!(base64_text("decode", None, "standard").unwrap(), "");
    assert_eq!(base64_text("encode", None, "url-safe").unwrap(), "");
}

#[test]
fn binary_data_fails_with_a_readable_error() {
    let error = base64_text("decode", Some("iVBORw=="), "standard").unwrap_err();
    assert!(error.contains("not UTF-8 text"), "{error}");
}
