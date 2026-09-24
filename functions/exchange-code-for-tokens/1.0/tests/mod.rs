mod bindings {
    wasmtime_testing_helper::bindgen!("main");

    wasmtime_testing_helper::setup!(Main);
}

// Only verifies the component links against the host and exports the expected WIT interface —
// this catches WIT/component-wiring mismatches (e.g. a renamed field the Rust side no longer
// matches). Actually invoking exchange-code-for-tokens against a mocked wasi:http response was
// attempted here but hit a `StreamError::Closed` between `waki`'s HTTP client and this harness's
// WASI HTTP mock that wasn't resolved — left as a known gap rather than a flaky/failing test.
#[test]
fn component_links_and_exports_the_expected_interface() {
    let harness = bindings::harness();
    let component = bindings::instantiate(harness);

    let _interface = component
        .component
        .betty_blocks_exchange_code_for_tokens_exchange_code_for_tokens();
}
