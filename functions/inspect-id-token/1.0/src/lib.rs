mod bindings {
    wit_bindgen::generate!({ generate_all });

    use crate::InspectIdToken;
    export!(InspectIdToken);
}

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};

use crate::bindings::exports::betty_blocks::inspect_id_token::inspect_id_token::Guest;

struct InspectIdToken;

fn decode_payload(id_token: &str) -> Result<(serde_json::Value, String), String> {
    let mut segments = id_token.split('.');
    let (Some(_header), Some(payload), Some(_signature)) =
        (segments.next(), segments.next(), segments.next())
    else {
        return Err(String::from(
            "id-token is not a valid JWT: expected three dot-separated segments",
        ));
    };
    if segments.next().is_some() {
        return Err(String::from(
            "id-token is not a valid JWT: expected exactly three dot-separated segments",
        ));
    }

    let decoded = URL_SAFE_NO_PAD
        .decode(payload)
        .map_err(|error| format!("id-token payload is not valid base64url: {error}"))?;

    let decoded_text = String::from_utf8(decoded)
        .map_err(|error| format!("id-token payload is not valid UTF-8: {error}"))?;

    let claims: serde_json::Value = serde_json::from_str(&decoded_text)
        .map_err(|error| format!("id-token payload is not valid JSON: {error}"))?;

    // A JWT payload must always be a JSON object per spec — unlike an HTTP response body,
    // there's no legitimate case where this should be anything else. Erroring loudly here
    // (rather than silently accepting a non-object) proves, one way or the other, whether
    // decoding/parsing is where a "claims ends up as a plain string" problem originates.
    if !claims.is_object() {
        return Err(format!(
            "id-token payload decoded to valid JSON but not a JSON object (got: {claims})"
        ));
    }

    Ok((claims, decoded_text))
}

// Claim comparisons are case-insensitive: values like a tenant GUID or issuer URL are usually
// copy-pasted or hand-typed into a canvas configuration field, where a casing slip is a realistic
// mistake that shouldn't reject an otherwise-legitimate token.
fn claim_equals(actual: &str, expected: &str) -> bool {
    actual.eq_ignore_ascii_case(expected)
}

fn audience_matches(claims: &serde_json::Value, expected_audience: &str) -> bool {
    match claims.get("aud") {
        Some(serde_json::Value::String(aud)) => claim_equals(aud, expected_audience),
        Some(serde_json::Value::Array(values)) => values.iter().any(|value| {
            value
                .as_str()
                .is_some_and(|value| claim_equals(value, expected_audience))
        }),
        _ => false,
    }
}

impl Guest for InspectIdToken {
    fn inspect_id_token(
        id_token: String,
        expected_issuer: String,
        expected_audience: String,
    ) -> Result<String, String> {
        let (claims, decoded_text) = decode_payload(&id_token)?;

        let issuer = claims.get("iss").and_then(|value| value.as_str());
        if !issuer.is_some_and(|value| claim_equals(value, &expected_issuer)) {
            return Err(format!(
                "id-token issuer mismatch: expected \"{expected_issuer}\", found {} — check that \"Expected Issuer\" matches the value your identity provider actually issues (its OpenID configuration document lists the correct value)",
                issuer.map_or(String::from("no \"iss\" claim"), |value| format!(
                    "\"{value}\""
                ))
            ));
        }

        if !audience_matches(&claims, &expected_audience) {
            return Err(format!(
                "id-token audience mismatch: expected \"{expected_audience}\" to be present in the \"aud\" claim, found {} — check that \"Expected Audience\" matches this application's client ID",
                claims
                    .get("aud")
                    .map_or(String::from("no \"aud\" claim"), |value| value.to_string())
            ));
        }

        Ok(decoded_text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_token(header: &serde_json::Value, payload: &serde_json::Value) -> String {
        let encode =
            |value: &serde_json::Value| URL_SAFE_NO_PAD.encode(serde_json::to_vec(value).unwrap());
        format!("{}.{}.signature", encode(header), encode(payload))
    }

    fn header() -> serde_json::Value {
        serde_json::json!({"alg": "RS256", "typ": "JWT"})
    }

    #[test]
    fn accepts_a_valid_token_with_string_audience() {
        let token = make_token(
            &header(),
            &serde_json::json!({
                "iss": "https://issuer.example",
                "aud": "my-client-id",
                "exp": 1_999_999_999_i64,
                "email": "user@example.com",
            }),
        );

        let result = InspectIdToken::inspect_id_token(
            token,
            String::from("https://issuer.example"),
            String::from("my-client-id"),
        )
        .unwrap();

        let claims: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(claims["email"], "user@example.com");
    }

    #[test]
    fn accepts_a_valid_token_with_array_audience() {
        let token = make_token(
            &header(),
            &serde_json::json!({
                "iss": "https://issuer.example",
                "aud": ["some-other-audience", "my-client-id"],
                "exp": 1_999_999_999_i64,
            }),
        );

        InspectIdToken::inspect_id_token(
            token,
            String::from("https://issuer.example"),
            String::from("my-client-id"),
        )
        .unwrap();
    }

    #[test]
    fn rejects_issuer_mismatch() {
        let token = make_token(
            &header(),
            &serde_json::json!({
                "iss": "https://wrong-issuer.example",
                "aud": "my-client-id",
                "exp": 1_999_999_999_i64,
            }),
        );

        let error = InspectIdToken::inspect_id_token(
            token,
            String::from("https://issuer.example"),
            String::from("my-client-id"),
        )
        .unwrap_err();

        assert!(error.contains("issuer mismatch"), "error was: {error}");
        assert!(error.contains("wrong-issuer"), "error was: {error}");
    }

    #[test]
    fn rejects_audience_mismatch() {
        let token = make_token(
            &header(),
            &serde_json::json!({
                "iss": "https://issuer.example",
                "aud": "some-other-client",
                "exp": 1_999_999_999_i64,
            }),
        );

        let error = InspectIdToken::inspect_id_token(
            token,
            String::from("https://issuer.example"),
            String::from("my-client-id"),
        )
        .unwrap_err();

        assert!(error.contains("audience mismatch"), "error was: {error}");
    }

    #[test]
    fn rejects_a_malformed_token() {
        let error = InspectIdToken::inspect_id_token(
            String::from("not-a-jwt"),
            String::from("https://issuer.example"),
            String::from("my-client-id"),
        )
        .unwrap_err();

        assert!(
            error.contains("three dot-separated segments"),
            "error was: {error}"
        );
    }
}
