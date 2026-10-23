//! Renders the query the user typed, sends it to the Data API as it is and returns the `data` of
//! the response. Unlike `data-api-request` it knows nothing about models: the query can read as
//! many models as it likes.

use serde_json::Value;

use crate::graphql;
use crate::template;

/// Fails with a readable message that points at the line when the query has a string that is not
/// closed or brackets that do not pair up, instead of sending it and getting the Data API's
/// "syntax error before: '}'".
///
/// The offending line is not quoted: the query can contain values filled in from variables that
/// should not end up in an error message.
fn check_balanced(query: &str, was_templated: bool) -> Result<(), String> {
    let note = if was_templated {
        " (counted after the variables were filled in)"
    } else {
        ""
    };
    match graphql::find_problem(query) {
        None => Ok(()),
        Some(problem) => Err(format!(
            "The query {}{note}.",
            problem.describe(query, false)
        )),
    }
}

/// Whether the query already is a GraphQL document: it starts with `{` (the shorthand for a
/// query) or with an operation or fragment keyword. Anything else, like
/// `allWebuser { results { id } }`, is a list of fields that still needs `query { ... }` around it.
fn is_document(query: &str) -> bool {
    // An unterminated string is reported by check_balanced; here it only means "not a document".
    let Ok(chars) = graphql::code_chars(query) else {
        return false;
    };
    let code: String = chars.into_iter().map(|(_, character)| character).collect();
    let code = code.trim_start();
    if code.starts_with('{') {
        return true;
    }
    let first_word: String = code
        .chars()
        .take_while(|character| character.is_alphanumeric() || *character == '_')
        .collect();
    matches!(
        first_word.as_str(),
        "query" | "mutation" | "subscription" | "fragment"
    )
}

/// Renders the `{{variables}}` in the query, checks it and adds `query { ... }` when it is only a
/// list of fields.
pub fn build_query(query: &str, variables: &Value) -> Result<String, String> {
    if query.trim().is_empty() {
        return Err(String::from(
            "The query is empty: write a Data API query, e.g. \"query { allWebuser { results { id } } }\"",
        ));
    }

    let rendered = template::render(query, variables)?;
    let rendered = rendered.trim();
    if rendered.is_empty() {
        return Err(String::from(
            "The query is empty after inserting the variables",
        ));
    }
    check_balanced(rendered, query.contains("{{"))?;

    Ok(if is_document(rendered) {
        rendered.to_string()
    } else {
        format!("query {{\n{rendered}\n}}")
    })
}

/// The messages of a GraphQL error envelope (`{"errors": [{"message": ...}]}`), if it is one.
fn graphql_error_messages(response: &Value) -> Option<String> {
    let errors = response.get("errors")?.as_array()?;
    if errors.is_empty() {
        return None;
    }
    let messages: Vec<String> = errors
        .iter()
        .map(|error| match error.get("message").and_then(Value::as_str) {
            Some(message) => message.to_string(),
            None => error.to_string(),
        })
        .collect();
    Some(messages.join("; "))
}

/// The host reports a backend error as `Err(<the GraphQL error envelope as a JSON string>)`.
/// Turn that into the plain message when it is one, otherwise keep the text as it is.
fn host_error_message(raw: &str) -> String {
    serde_json::from_str::<Value>(raw)
        .ok()
        .and_then(|parsed| graphql_error_messages(&parsed))
        .unwrap_or_else(|| raw.to_string())
}

/// Picks the `data` out of the Data API response, as JSON text.
fn extract_data(response: &str) -> Result<String, String> {
    let parsed: Value = serde_json::from_str(response)
        .map_err(|error| format!("The Data API response is not valid JSON: {error}"))?;

    if let Some(messages) = graphql_error_messages(&parsed) {
        return Err(messages);
    }

    match parsed.get("data") {
        Some(data) if !data.is_null() => {
            serde_json::to_string(data).map_err(|error| error.to_string())
        }
        _ => Err(String::from("The Data API response has no data")),
    }
}

/// Builds the query, sends it with `send`, and returns the `data` of the response as JSON text.
pub fn run(
    query: &str,
    variables: &Value,
    send: impl FnOnce(&str) -> Result<String, String>,
) -> Result<String, String> {
    let query = build_query(query, variables)?;
    let response = send(&query).map_err(|error| host_error_message(&error))?;
    extract_data(&response)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn squash(text: &str) -> String {
        text.split_whitespace().collect::<Vec<_>>().join(" ")
    }

    #[test]
    fn a_full_query_is_sent_as_it_is() {
        let query = "query { allWebuser { results { id name } } allCompany { totalCount } }";
        assert_eq!(build_query(query, &json!({})).unwrap(), query);
    }

    #[test]
    fn shorthand_and_other_operations_are_sent_as_they_are() {
        for query in [
            "{ allWebuser { results { id } } }",
            "  \n# comment first\nquery Users { allWebuser { results { id } } }",
            "mutation { createWebuser(input: { name: \"a\" }) { id } }",
            "fragment F on Webuser { id }\nquery { allWebuser { results { ...F } } }",
        ] {
            assert_eq!(
                build_query(query, &json!({})).unwrap(),
                query.trim(),
                "{query}"
            );
        }
    }

    #[test]
    fn a_list_of_fields_gets_query_around_it() {
        let built = build_query(
            "allWebuser { results { id } }\nallCompany { totalCount }",
            &json!({}),
        )
        .unwrap();
        assert_eq!(
            squash(&built),
            "query { allWebuser { results { id } } allCompany { totalCount } }"
        );
    }

    #[test]
    fn a_comment_does_not_hide_the_missing_query_keyword() {
        let built = build_query("# the users\nallWebuser { results { id } }", &json!({})).unwrap();
        assert!(built.starts_with("query {"), "{built}");
    }

    #[test]
    fn variables_are_inserted_in_the_query() {
        let built = build_query(
            "query { allWebuser(where: { id: { eq: {{id}} }, name: { eq: \"{{name}}\" } }) { results { id } } }",
            &json!({ "id": 7, "name": "Jan" }),
        )
        .unwrap();
        assert_eq!(
            built,
            "query { allWebuser(where: { id: { eq: 7 }, name: { eq: \"Jan\" } }) { results { id } } }"
        );
    }

    #[test]
    fn a_tag_directly_before_closing_braces_works() {
        let built = build_query(
            "{ oneWebuser(where: {id: {eq: {{id}}}}) { id } }",
            &json!({ "id": 3 }),
        )
        .unwrap();
        assert_eq!(built, "{ oneWebuser(where: {id: {eq: 3}}) { id } }");
    }

    #[test]
    fn list_items_and_length_can_be_inserted() {
        let built = build_query(
            "{ allWebuser(where: { id: { in: [{{ids.0}}, {{ids.1}}] } }, take: {{ids.length}}) { totalCount } }",
            &json!({ "ids": [4, 5] }),
        )
        .unwrap();
        assert_eq!(
            built,
            "{ allWebuser(where: { id: { in: [4, 5] } }, take: 2) { totalCount } }"
        );
    }

    #[test]
    fn sections_are_not_supported() {
        let error = build_query(
            "{ a(f: [{{#ids}}{{.}},{{/ids}}]) { id } }",
            &json!({ "ids": [1] }),
        )
        .unwrap_err();
        assert!(error.contains("not supported"), "{error}");
        assert!(error.contains("The query"), "{error}");
    }

    #[test]
    fn a_backslash_in_a_value_cannot_escape_the_closing_quote() {
        let built = build_query(
            "{ allWebuser(where: { name: { eq: \"{{name}}\" } }) { totalCount } }",
            &json!({ "name": "a\\" }),
        )
        .unwrap();
        assert_eq!(
            built,
            "{ allWebuser(where: { name: { eq: \"a\\\\\" } }) { totalCount } }"
        );
    }

    #[test]
    fn a_quote_in_a_value_is_escaped() {
        let built = build_query("{ a(f: \"{{v}}\") { id } }", &json!({ "v": "x\" } y" })).unwrap();
        assert_eq!(built, "{ a(f: \"x&quot; } y\") { id } }");
    }

    #[test]
    fn a_query_that_grows_too_large_is_an_error() {
        let items: Vec<u32> = (0..30_000).collect();
        let error = build_query("{ a(f: [{{v}}]) { id } }", &json!({ "v": items })).unwrap_err();
        assert!(error.contains("longer than 64 KB"), "{error}");
    }

    #[test]
    fn raw_tags_do_not_escape_text() {
        let built =
            build_query("{ a(f: {{{f}}}) { id } }", &json!({ "f": "{ x: \"y\" }" })).unwrap();
        assert_eq!(built, "{ a(f: { x: \"y\" }) { id } }");
    }

    #[test]
    fn an_empty_query_is_an_error() {
        assert!(
            build_query("  \n ", &json!({}))
                .unwrap_err()
                .contains("empty")
        );
        assert!(
            build_query("{{x}}", &json!({ "x": "" }))
                .unwrap_err()
                .contains("empty after")
        );
    }

    #[test]
    fn brackets_that_do_not_pair_up_are_reported_with_the_line() {
        let unclosed = build_query("query {\n  allWebuser {\n    results { id }\n}", &json!({}));
        let error = unclosed.unwrap_err();
        assert!(
            error.contains("never closed") && error.contains("line 1"),
            "{error}"
        );

        let unmatched = build_query("query { allWebuser { id } } }", &json!({}));
        assert!(unmatched.unwrap_err().contains("without an opening one"));

        let mismatch = build_query("query { allWebuser(take: 1] { id } }", &json!({}));
        assert!(mismatch.unwrap_err().contains("does not match"));
    }

    #[test]
    fn brackets_in_strings_and_comments_are_ignored() {
        let query = "query {\n  # a ) in a comment\n  allWebuser(where: { name: { eq: \"a } b\" } }) { results { id } }\n}";
        assert!(build_query(query, &json!({})).is_ok());
    }

    #[test]
    fn a_template_error_is_returned() {
        let error = build_query("{ a { id } } {{/x}}", &json!({})).unwrap_err();
        assert!(error.contains("not supported"), "{error}");
    }

    #[test]
    fn the_data_of_the_response_is_returned() {
        let response = json!({
            "data": {
                "allWebuser": { "results": [{ "id": 1, "name": "a" }] },
                "allCompany": { "totalCount": 4 }
            }
        })
        .to_string();
        let result = run(
            "{ allWebuser { results { id name } } allCompany { totalCount } }",
            &json!({}),
            |_| Ok(response.clone()),
        )
        .unwrap();
        let parsed: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["allWebuser"]["results"][0]["name"], "a");
        assert_eq!(parsed["allCompany"]["totalCount"], 4);
    }

    #[test]
    fn the_rendered_query_is_what_gets_sent() {
        let mut sent = String::new();
        run(
            "allWebuser(take: {{n}}) { totalCount }",
            &json!({ "n": 2 }),
            |query| {
                sent = query.to_string();
                Ok(r#"{"data":{"allWebuser":{"totalCount":2}}}"#.to_string())
            },
        )
        .unwrap();
        assert_eq!(
            squash(&sent),
            "query { allWebuser(take: 2) { totalCount } }"
        );
    }

    #[test]
    fn a_graphql_error_in_the_response_fails_the_step() {
        let response = r#"{"errors":[{"message":"first"},{"message":"second"}],"data":null}"#;
        let error = run("{ a { id } }", &json!({}), |_| Ok(response.to_string())).unwrap_err();
        assert_eq!(error, "first; second");
    }

    #[test]
    fn a_host_error_envelope_becomes_its_message() {
        let envelope =
            r#"{"errors":[{"message":"in object 'Webuser' the fields 'ida' are not found."}]}"#;
        let error = run("{ a { id } }", &json!({}), |_| Err(envelope.to_string())).unwrap_err();
        assert_eq!(error, "in object 'Webuser' the fields 'ida' are not found.");
    }

    #[test]
    fn another_host_error_is_kept_as_it_is() {
        let error = run("{ a { id } }", &json!({}), |_| {
            Err("rate limited".to_string())
        })
        .unwrap_err();
        assert_eq!(error, "rate limited");
    }

    #[test]
    fn a_response_without_data_is_an_error() {
        for response in ["{}", r#"{"data":null}"#] {
            let error = run("{ a { id } }", &json!({}), |_| Ok(response.to_string())).unwrap_err();
            assert!(error.contains("no data"), "{error}");
        }
        let error = run("{ a { id } }", &json!({}), |_| Ok("nope".to_string())).unwrap_err();
        assert!(error.contains("not valid JSON"), "{error}");
    }

    #[test]
    fn nothing_is_sent_when_the_query_is_invalid() {
        let mut called = false;
        let result = run("query {", &json!({}), |_| {
            called = true;
            Ok(String::new())
        });
        assert!(result.is_err());
        assert!(!called);
    }

    #[test]
    fn an_unterminated_string_is_reported() {
        let error = build_query(
            "{ allWebuser(where: { name: { eq: \"abc } }) { id } }",
            &json!({}),
        )
        .unwrap_err();
        assert!(
            error.contains("never closed") && error.contains("line 1"),
            "{error}"
        );
    }

    #[test]
    fn a_block_string_hides_brackets_like_the_data_api_reads_it() {
        let query = "{ allWebuser(where: { name: { eq: \"\"\"a } \" b\"\"\" } }) { id } }";
        assert!(build_query(query, &json!({})).is_ok());
    }

    #[test]
    fn a_comment_ends_at_a_carriage_return() {
        // The bracket after the \r is code, so the unclosed brace before it is found.
        let error = build_query("{ allWebuser { id # comment\r}", &json!({})).unwrap_err();
        assert!(error.contains("never closed"), "{error}");
    }

    #[test]
    fn a_value_cannot_hide_a_bracket_inside_a_comment_or_string() {
        // The value adds a comment start; the closing brace on the next line is still code.
        let built = build_query("{ a(f: {{v}}\n) { id } }", &json!({ "v": "1 #" }));
        assert!(built.is_ok(), "{built:?}");
        let error = build_query("{ a(f: {{{v}}}\n) { id }", &json!({ "v": "1 #" })).unwrap_err();
        assert!(error.contains("never closed"), "{error}");
    }

    #[test]
    fn bracket_errors_name_the_line_but_do_not_quote_it() {
        let error =
            build_query("{\n  allWebuser(name: \"secret\"\n  { id }\n}", &json!({})).unwrap_err();
        assert!(error.contains("line 2"), "{error}");
        assert!(!error.contains("secret"), "{error}");
    }

    #[test]
    fn a_bracket_error_in_a_templated_query_says_the_lines_are_after_filling_in() {
        let error = build_query("{ a(f: {{v}} { id }", &json!({ "v": 1 })).unwrap_err();
        assert!(
            error.contains("after the variables were filled in"),
            "{error}"
        );
        let plain = build_query("{ a { id }", &json!({})).unwrap_err();
        assert!(!plain.contains("after the variables"), "{plain}");
        // Text the template leaves alone, spaces and line breaks around it included, is not
        // "after the variables were filled in" either.
        let padded =
            build_query("\n  {\r\n a(f: {id: {eq: 1}}) { id }\r\n  ", &json!({})).unwrap_err();
        assert!(!padded.contains("after the variables"), "{padded}");
    }
}
