mod bindings {
    wasmtime_testing_helper::bindgen!("main");

    wasmtime_testing_helper::setup!(Main);
}

fn hash_text(
    algorithm: &str,
    text: Option<&str>,
    secret_key: Option<&str>,
    output_encoding: &str,
) -> Result<String, String> {
    let harness = bindings::harness();
    let mut component = bindings::instantiate(harness);
    component
        .component
        .betty_blocks_hash_text_hash_text()
        .call_hash_text(
            &mut component.store,
            algorithm,
            text,
            secret_key,
            output_encoding,
        )
        .unwrap()
}

#[test]
fn hashes_with_sha256_through_a_real_component_instance() {
    assert_eq!(
        hash_text("sha256", Some("abc"), None, "hex").unwrap(),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}

#[test]
fn signs_with_hmac_sha256_through_a_real_component_instance() {
    // RFC 4231, test case 2.
    assert_eq!(
        hash_text(
            "hmac-sha256",
            Some("what do ya want for nothing?"),
            Some("Jefe"),
            "base64"
        )
        .unwrap(),
        "W9zBRr9gdU5qBCQmCJV1x1oAPwidJzmDnexYuWTsOEM="
    );
}

#[test]
fn an_empty_text_variable_hashes_the_empty_string() {
    assert_eq!(
        hash_text("sha1", None, Some(""), "hex").unwrap(),
        "da39a3ee5e6b4b0d3255bfef95601890afd80709"
    );
}

#[test]
fn hmac_without_a_key_fails_with_a_readable_error() {
    let error = hash_text("hmac-sha256", Some("message"), None, "hex").unwrap_err();
    assert!(error.contains("\"Secret key\" is empty"), "{error}");
}
