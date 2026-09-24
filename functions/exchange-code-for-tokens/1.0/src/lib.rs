use waki::Client;

use crate::exports::betty_blocks::exchange_code_for_tokens::exchange_code_for_tokens::{
    self, TokenResponse,
};

// `waki` links against the official `wasi` crate's own generated bindings for `wasi:http`, so
// this crate's own `wit_bindgen::generate!` must reuse those same types (via `with:`) instead of
// generating a second, incompatible copy — mirrors bettyblocks/native-wasm-components' HTTP step.
wit_bindgen::generate!({ with: {
    "wasi:io/streams@0.2.6": ::wasi::io::streams,
    "wasi:io/error@0.2.6": ::wasi::io::error,
    "wasi:clocks/monotonic-clock@0.2.6": ::wasi::clocks::monotonic_clock,
    "wasi:io/poll@0.2.6": ::wasi::io::poll,
    "wasi:http/types@0.2.6": ::wasi::http::types,
    "wasi:http/outgoing-handler@0.2.6": ::wasi::http::outgoing_handler,
    }
});

struct ExchangeCodeForTokens;

fn build_form_body(client_id: &str, client_secret: &str, code: &str, redirect_uri: &str) -> String {
    url::form_urlencoded::Serializer::new(String::new())
        .append_pair("grant_type", "authorization_code")
        .append_pair("code", code)
        .append_pair("client_id", client_id)
        .append_pair("client_secret", client_secret)
        .append_pair("redirect_uri", redirect_uri)
        .finish()
}

fn required_str_field(body: &serde_json::Value, field: &str) -> Result<String, String> {
    body.get(field)
        .and_then(|value| value.as_str())
        .map(String::from)
        .ok_or_else(|| format!("token endpoint response did not include an \"{field}\" field"))
}

fn oauth_error_message(body: &serde_json::Value, raw_body: &str) -> String {
    let error = body.get("error").and_then(|value| value.as_str());
    let description = body
        .get("error_description")
        .and_then(|value| value.as_str());

    match (error, description) {
        (Some(error), Some(description)) => format!("{error}: {description}"),
        (Some(error), None) => error.to_string(),
        (None, Some(description)) => description.to_string(),
        (None, None) if !raw_body.trim().is_empty() => raw_body.to_string(),
        (None, None) => String::from("token endpoint returned an empty error response"),
    }
}

impl exchange_code_for_tokens::Guest for ExchangeCodeForTokens {
    fn exchange_code_for_tokens(
        token_endpoint: String,
        client_id: String,
        client_secret: String,
        code: String,
        redirect_uri: String,
    ) -> Result<TokenResponse, String> {
        if !token_endpoint.starts_with("https://") {
            return Err(String::from(
                "\"Token Endpoint\" must start with https:// — sending the client secret over plain HTTP is not safe",
            ));
        }

        let body = build_form_body(&client_id, &client_secret, &code, &redirect_uri);

        let response = Client::new()
            .post(&token_endpoint)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .body(body)
            .send()
            .map_err(|error| {
                format!(
                    "request to token endpoint failed: {error} — check that \"Token Endpoint\" is correct and reachable"
                )
            })?;

        let status = response.status_code();
        let raw_body = response
            .body()
            .map_err(|error| format!("could not read token endpoint response body: {error}"))?;
        let raw_body = String::from_utf8(raw_body)
            .unwrap_or_else(|error| String::from_utf8_lossy(error.as_bytes()).into_owned());

        let is_success = (200..300).contains(&status);

        let parsed: serde_json::Value = match serde_json::from_str(&raw_body) {
            Ok(value) => value,
            Err(error) if is_success => {
                return Err(format!(
                    "token endpoint returned HTTP {status} but the response body was not valid JSON: {error}"
                ));
            }
            Err(_) => serde_json::Value::Null,
        };

        if !is_success {
            return Err(format!(
                "token endpoint returned HTTP {status}: {} — check that \"Client ID\", \"Client Secret\", \"Authorization Code\", and \"Redirect URI\" are all correct",
                oauth_error_message(&parsed, &raw_body)
            ));
        }

        if parsed.get("error").is_some() {
            return Err(oauth_error_message(&parsed, &raw_body));
        }

        Ok(TokenResponse {
            access_token: required_str_field(&parsed, "access_token")?,
            id_token: required_str_field(&parsed, "id_token")?,
        })
    }
}

export!(ExchangeCodeForTokens);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_the_expected_form_body() {
        let body = build_form_body(
            "client id/with space",
            "s3cr3t&",
            "the=code",
            "https://app.example/cb",
        );

        assert_eq!(
            body,
            "grant_type=authorization_code&code=the%3Dcode&client_id=client+id%2Fwith+space&client_secret=s3cr3t%26&redirect_uri=https%3A%2F%2Fapp.example%2Fcb"
        );
    }

    #[test]
    fn oauth_error_message_prefers_error_and_description() {
        let body =
            serde_json::json!({"error": "invalid_grant", "error_description": "code expired"});
        assert_eq!(
            oauth_error_message(&body, "{}"),
            "invalid_grant: code expired"
        );
    }

    #[test]
    fn oauth_error_message_falls_back_to_raw_body() {
        let body = serde_json::Value::Null;
        assert_eq!(oauth_error_message(&body, "not json"), "not json");
    }

    #[test]
    fn rejects_a_non_https_token_endpoint() {
        use exchange_code_for_tokens::Guest;

        let error = ExchangeCodeForTokens::exchange_code_for_tokens(
            String::from("http://token.example/token"),
            String::from("client-id"),
            String::from("client-secret"),
            String::from("code"),
            String::from("https://app.example/cb"),
        )
        .unwrap_err();

        assert!(error.contains("https://"), "error was: {error}");
    }
}
