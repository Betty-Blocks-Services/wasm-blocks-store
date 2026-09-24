mod bindings {
    wasmtime_testing_helper::bindgen!("main");

    wasmtime_testing_helper::setup!(Main);
}

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};

fn make_token(payload: &serde_json::Value) -> String {
    let header = serde_json::json!({"alg": "RS256", "typ": "JWT"});
    let encode =
        |value: &serde_json::Value| URL_SAFE_NO_PAD.encode(serde_json::to_vec(value).unwrap());
    format!("{}.{}.signature", encode(&header), encode(payload))
}

#[test]
fn validates_a_token_through_a_real_component_instance() {
    let harness = bindings::harness();
    let mut component = bindings::instantiate(harness);

    let interface = component
        .component
        .betty_blocks_inspect_id_token_inspect_id_token();

    let token = make_token(&serde_json::json!({
        "iss": "https://issuer.example",
        "aud": "my-client-id",
        "exp": 1_999_999_999_u32,
        "email": "user@example.com",
    }));

    let result = interface
        .call_inspect_id_token(
            &mut component.store,
            &token,
            "https://issuer.example",
            "my-client-id",
        )
        .unwrap()
        .unwrap();

    let claims: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(claims["email"], "user@example.com");
}

#[test]
fn fails_on_issuer_mismatch_through_a_real_component_instance() {
    let harness = bindings::harness();
    let mut component = bindings::instantiate(harness);

    let interface = component
        .component
        .betty_blocks_inspect_id_token_inspect_id_token();

    let token = make_token(&serde_json::json!({
        "iss": "https://wrong-issuer.example",
        "aud": "my-client-id",
        "exp": 1_999_999_999_u32,
    }));

    let error = interface
        .call_inspect_id_token(
            &mut component.store,
            &token,
            "https://issuer.example",
            "my-client-id",
        )
        .unwrap()
        .unwrap_err();

    assert!(error.contains("issuer mismatch"), "error was: {error}");
}
