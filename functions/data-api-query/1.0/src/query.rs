//! Renders the query the user typed, sends it to the Data API as it is and returns the `data` of
//! the response. Unlike `data-api-request` it knows nothing about models: the query can read as
//! many models as it likes.

use serde_json::Value;

use crate::graphql;
use crate::template;

/// The Variables option as a JSON object. Empty (and `null`, which an unset Map may arrive as)
/// means no variables.
pub fn parse_variables(text: &str) -> Result<Value, String> {
    if text.trim().is_empty() {
        return Ok(serde_json::json!({}));
    }
    match serde_json::from_str::<Value>(text) {
        Ok(Value::Null) => Ok(serde_json::json!({})),
        Ok(object @ Value::Object(_)) => Ok(object),
        Ok(_) => Err(String::from(
            "The Variables must be a map of names to values: check the Variables option",
        )),
        Err(error) => Err(format!(
            "The Variables are not valid JSON ({error}): check the Variables option"
        )),
    }
}

/// Whether any text in `value` contains a line break.
fn has_line_break(value: &Value) -> bool {
    match value {
        Value::String(text) => text.contains(['\n', '\r']),
        Value::Array(items) => items.iter().any(has_line_break),
        Value::Object(map) => map.values().any(has_line_break),
        _ => false,
    }
}

/// Fails with a readable message that points at the line when the query has a string that is not
/// closed or brackets that do not pair up, instead of sending it and getting the Data API's
/// "syntax error before: '}'".
///
/// The offending line is not quoted: the query can contain values filled in from variables that
/// should not end up in an error message.
fn check_balanced(query: &str, was_templated: bool, variables: &Value) -> Result<(), String> {
    let note = if was_templated {
        " (counted after the variables were filled in)"
    } else {
        ""
    };
    match graphql::find_problem(query) {
        None => Ok(()),
        Some(problem) => {
            // A line break in a value that sits inside quotes ends the string early.
            let hint = if matches!(problem, graphql::Problem::Unterminated { .. })
                && was_templated
                && has_line_break(variables)
            {
                " A value in Variables contains a line break, which cannot be inserted inside \
                 quotes with {{name}}: remove the line break first, for example in an \
                 Expression step."
            } else {
                ""
            };
            Err(format!(
                "The query {}{note}.{hint}",
                problem.describe(query, false)
            ))
        }
    }
}

/// Where the value of one escaped `{{tag}}` ended up in the rendered text.
struct EscapedValue {
    /// The tag as written, e.g. `{{ id }}`.
    tag: String,
    value: String,
    span: std::ops::Range<usize>,
}

/// The rendered text, and where the values of its escaped `{{tags}}` are in it.
struct Rendered {
    text: String,
    escaped: Vec<EscapedValue>,
}

/// Renders the template like `template::render`, tag by tag, and records where the value of each
/// escaped tag lands in the output (raw tags, comments and plain text are copied as they are).
///
/// Every tag is rendered by `template::render` itself, so the values are the ones it would
/// insert. The tag boundaries are found the way `template::render` finds them; as a safeguard the
/// result must be exactly what `template::render` gives for the whole template, and the step fails
/// when it is not, so a disagreement between the two can never go unnoticed.
fn render_tracked(template: &str, variables: &Value) -> Result<Rendered, String> {
    let mut text = String::new();
    let mut escaped = Vec::new();
    let mut rest = template;

    while let Some(offset) = rest.find("{{") {
        text.push_str(&rest[..offset]);
        let tag_and_after = &rest[offset..];
        let after = &tag_and_after[2..];

        // A raw tag `{{{x}}}` is closed by `}}}`; without one the rest stays as it is.
        let (tag_len, is_escaped) = if let Some(inner) = after.strip_prefix('{') {
            match inner.find("}}}") {
                Some(end) => (3 + end + 3, false),
                None => {
                    text.push_str(tag_and_after);
                    rest = "";
                    break;
                }
            }
        } else {
            match after.find("}}") {
                Some(end) => (2 + end + 2, !after.starts_with(['&', '!', '#', '^', '/'])),
                None => {
                    text.push_str(tag_and_after);
                    rest = "";
                    break;
                }
            }
        };

        let tag = &tag_and_after[..tag_len];
        let value = template::render(tag, variables)?;
        let from = text.len();
        text.push_str(&value);
        if is_escaped {
            escaped.push(EscapedValue {
                tag: tag.to_string(),
                value,
                span: from..text.len(),
            });
        }
        rest = &tag_and_after[tag_len..];
    }
    text.push_str(rest);

    if text != template::render(template, variables)? {
        return Err(String::from(
            "The query could not be checked: the variables could not be placed in it. Simplify \
             the {{...}} tags in the query",
        ));
    }
    Ok(Rendered { text, escaped })
}

/// Whether an escaped `{{tag}}` value is plain: a number, `true`/`false`, a name, or a
/// comma-separated list of those (what a list value prints as). Nothing in it can close a bracket,
/// start a string or a comment, or add a field, wherever it is put. (It can still be any name, so a
/// tag in the place of a field, argument or operation keyword lets its value choose one.)
fn is_plain_value(value: &str) -> bool {
    value.split(',').all(|part| {
        !part.is_empty()
            && part
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
    }) || value.is_empty()
}

/// The finished query with each non-plain escaped value either as it is (`hostile`) or replaced
/// by the harmless text `a`, and what its code looks like outside those values: the code
/// characters of the query (strings and comments are not code) that are not part of such a value.
/// `None` when the query has a string that is never closed.
fn code_outside_values(rendered: &Rendered, hostile: impl Fn(usize) -> bool) -> Option<String> {
    let mut text = String::new();
    let mut spans = Vec::new();
    let mut copied = 0;

    for (index, value) in rendered.escaped.iter().enumerate() {
        if is_plain_value(&value.value) {
            continue;
        }
        text.push_str(&rendered.text[copied..value.span.start]);
        let from = text.len();
        text.push_str(if hostile(index) { &value.value } else { "a" });
        spans.push(from..text.len());
        copied = value.span.end;
    }
    text.push_str(&rendered.text[copied..]);

    let code = graphql::code_chars(&text).ok()?;
    Some(
        code.into_iter()
            .filter(|(at, _)| {
                // The spans are in order and do not overlap: look the one at `at` up by binary search.
                let next = spans.partition_point(|span| span.end <= *at);
                !spans.get(next).is_some_and(|span| span.start <= *at)
            })
            .map(|(_, character)| character)
            .collect(),
    )
}

/// Fails when an escaped `{{tag}}` value could change how the query is read.
///
/// `{{name}}` escapes `&`, `"`, `<`, `>` and backslashes, which keeps a value inside a string it
/// sits in, but a value that is not plain (see `is_plain_value`) can still do harm in two ways, and
/// both are checked on the finished query, read the way the GraphQL specification reads it (the
/// Data API reads strings more simply, see the learnings log of 2026-10-24; what that means for
/// a backslash before a quote written ahead of a tag is not tested). (A template can read
/// differently: a quote in a `{{! comment}}`, or a backslash in front of a tag, moves the string
/// boundaries):
/// 1. it is query code itself: outside a string, `1 }, _or: [...]` adds a filter and
///    `1 } } other: allUser { ... }` a field on another model;
/// 2. it changes how the rest of the query is read: a value that starts a `#` comment hides the
///    builder's text after it on the same line, and a value ending in a backslash inside a block
///    string (where doubling the backslash does nothing) runs the string on into the next one.
///    This is checked by reading the query again with every such value replaced by `a`: the
///    code outside the values must be the same.
///
/// Raw tags (`{{{name}}}`, `{{&name}}`) are not checked: they are for query code the builder
/// trusts.
fn check_unquoted_tags(rendered: &Rendered) -> Result<(), String> {
    // An unterminated string is reported by check_balanced, which runs next and fails the step.
    let Ok(code) = graphql::code_chars(&rendered.text) else {
        return Ok(());
    };
    let non_plain: Vec<usize> = (0..rendered.escaped.len())
        .filter(|&index| !is_plain_value(&rendered.escaped[index].value))
        .collect();
    let name = |index: usize| {
        rendered.escaped[index]
            .tag
            .trim_start_matches('{')
            .trim_end_matches('}')
            .trim()
            .to_string()
    };

    // 1. A value that is code.
    for &index in &non_plain {
        let span = &rendered.escaped[index].span;
        let first = code.partition_point(|(at, _)| *at < span.start);
        if code.get(first).is_some_and(|(at, _)| *at < span.end) {
            let name = name(index);
            return Err(format!(
                "The variable {{{{{name}}}}} is used outside quotes in the query, so it may only \
                 hold a number, true or false, a name (letters, digits, _, - and .), or a \
                 comma-separated list of those. Put it inside quotes (\"{{{{{name}}}}}\") for \
                 text, or use {{{{{{{name}}}}}}} for query code you trust"
            ));
        }
    }

    // 2. A value that changes how the rest of the query is read.
    if non_plain.is_empty() {
        return Ok(());
    }
    let actual = code_outside_values(rendered, |_| true);
    let harmless = code_outside_values(rendered, |_| false);
    if actual == harmless {
        return Ok(());
    }
    // Name the value that does it, or the first one when it takes two together (or when there are
    // so many that trying each would be slow).
    let culprit = if non_plain.len() <= 32 {
        non_plain
            .iter()
            .copied()
            .find(|&only| code_outside_values(rendered, |index| index == only) != harmless)
            .unwrap_or(non_plain[0])
    } else {
        non_plain[0]
    };
    let name = name(culprit);
    Err(format!(
        "The value of the variable {{{{{name}}}}} changes how the rest of the query is read: a \
         value that starts with # (a comment) or that ends in a backslash inside a \"\"\" string \
         can hide the query text after it. Remove those characters from the value, or move the tag"
    ))
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
    // `query: allWebuser { ... }` is a field with an alias that happens to be called `query`.
    let is_alias = code[first_word.len()..].trim_start().starts_with(':');
    !is_alias
        && matches!(
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

    let tracked = render_tracked(query, variables)?;
    check_unquoted_tags(&tracked)?;
    let rendered = tracked.text.trim();
    if rendered.is_empty() {
        return Err(String::from(
            "The query is empty after inserting the variables: check the values in Variables",
        ));
    }
    check_balanced(rendered, query.contains("{{"), variables)?;

    Ok(if is_document(rendered) {
        rendered.to_string()
    } else {
        format!("query {{\n{rendered}\n}}")
    })
}

const NO_MESSAGE: &str = "The Data API returned an error without a message";

/// The text of one GraphQL error: its message, or a fixed text (plus the `path` when there is
/// one). The rest of the error object is not shown: it can hold server-side detail.
fn error_text(error: &Value) -> String {
    match error.get("message").and_then(Value::as_str).map(str::trim) {
        Some(message) if !message.is_empty() => message.to_string(),
        _ => {
            let path: Vec<String> = error
                .get("path")
                .and_then(Value::as_array)
                .map(|steps| {
                    steps
                        .iter()
                        .map(|step| match step {
                            Value::String(text) => text.clone(),
                            other => other.to_string(),
                        })
                        .collect()
                })
                .unwrap_or_default();
            if path.is_empty() {
                NO_MESSAGE.to_string()
            } else {
                format!("{NO_MESSAGE} (at {})", path.join("."))
            }
        }
    }
}

/// The messages of a GraphQL error envelope (`{"errors": ...}`), if it holds any error. An
/// `errors` that is not a list still counts as an error unless it is empty.
fn graphql_error_messages(response: &Value) -> Option<String> {
    match response.get("errors")? {
        Value::Null => None,
        Value::Array(errors) if errors.is_empty() => None,
        Value::Array(errors) => Some(errors.iter().map(error_text).collect::<Vec<_>>().join("; ")),
        Value::String(text) if text.trim().is_empty() => None,
        Value::String(text) => Some(text.trim().to_string()),
        Value::Object(map) if map.is_empty() => None,
        other @ Value::Object(_) => Some(error_text(other)),
        _ => Some(NO_MESSAGE.to_string()),
    }
}

/// The host reports a backend error as `Err(<the GraphQL error envelope as a JSON string>)`.
/// Turn that into the plain message when it is one, otherwise keep the text as it is (and say
/// something when there is none).
fn host_error_message(raw: &str) -> String {
    let message = serde_json::from_str::<Value>(raw)
        .ok()
        .and_then(|parsed| graphql_error_messages(&parsed))
        .unwrap_or_else(|| raw.to_string());
    if message.trim().is_empty() {
        NO_MESSAGE.to_string()
    } else {
        message
    }
}

/// Picks the `data` out of the Data API response, as JSON text.
fn extract_data(response: &str) -> Result<String, String> {
    let parsed: Value = serde_json::from_str(response).map_err(|error| {
        format!(
            "The Data API response could not be read ({error}): run the step again, and check \
             the query if it keeps happening"
        )
    })?;

    if let Some(messages) = graphql_error_messages(&parsed) {
        return Err(messages);
    }

    match parsed.get("data") {
        Some(data) if !data.is_null() => {
            serde_json::to_string(data).map_err(|error| error.to_string())
        }
        _ => Err(String::from(
            "The Data API returned no data for this query: check that the models and fields in \
             the query exist",
        )),
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
        assert!(error.contains("could not be read"), "{error}");
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
    fn a_raw_value_cannot_hide_a_bracket_inside_a_comment() {
        // The value starts a comment; the closing brace on the next line is still code.
        let built = build_query("{ a(f: {{{v}}}\n) { id } }", &json!({ "v": "1 #" }));
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

    // T1: escaped tags outside a string may only hold plain values.

    #[test]
    fn an_unquoted_tag_cannot_add_a_filter_clause() {
        let template = "query { allWebuser(where: { owner: { eq: {{id}} } }) { results { id } } }";
        let id = "7 }, _or: [{ id: { gt: 0 } }], x: { eq: 1 ";
        let error = build_query(template, &json!({ "id": id })).unwrap_err();
        assert!(error.contains("outside quotes"), "{error}");
        assert!(error.contains("{{id}}"), "{error}");
        assert!(!error.contains("_or"), "the value is not echoed: {error}");
    }

    #[test]
    fn an_unquoted_tag_cannot_add_a_field_on_another_model() {
        let template = "query { allWebuser(where: { id: { eq: {{id}} } }) { results { id } } }";
        let id = "1 } }) { results { id } } other: allUser { results { id email } } x: allWebuser(where: { id: { eq: 1 ";
        assert!(build_query(template, &json!({ "id": id })).is_err());
    }

    #[test]
    fn an_unquoted_tag_cannot_add_a_second_operation() {
        let template = "query { allWebuser(where: { id: { eq: {{id}} } }) { totalCount } }";
        let id = "1 } }) { totalCount } }\nmutation M { deleteWebuser(id: 1) { id } }\nquery Q { allWebuser(where: { id: { eq: 1 ";
        assert!(build_query(template, &json!({ "id": id })).is_err());
    }

    #[test]
    fn plain_values_are_fine_outside_quotes() {
        let template =
            "{ a(id: {{id}}, flag: {{flag}}, sort: {{field}}, ids: [{{ids}}], n: {{n.0}}) { id } }";
        let variables = json!({
            "id": 7, "flag": true, "field": "first_name",
            "ids": [1, 2, 3], "n": [-1.5],
        });
        assert_eq!(
            build_query(template, &variables).unwrap(),
            "{ a(id: 7, flag: true, sort: first_name, ids: [1,2,3], n: -1.5) { id } }"
        );
    }

    #[test]
    fn a_missing_or_empty_value_outside_quotes_is_not_an_injection() {
        assert!(build_query("{ a(id: {{nope}}) { id } }", &json!({})).is_ok());
        assert!(build_query("{ a(id: {{x}}) { id } }", &json!({ "x": null })).is_ok());
    }

    #[test]
    fn text_with_spaces_or_symbols_outside_quotes_is_refused() {
        for value in [
            "a b", "x:1", "a)", "1;2", "a\"b", "{", "#", "1,", ",1", "a,,b", "é",
        ] {
            assert!(
                build_query("{ a(f: {{v}}) { id } }", &json!({ "v": value })).is_err(),
                "{value:?}"
            );
        }
        // An object prints as "[object Object]".
        assert!(build_query("{ a(f: {{v}}) { id } }", &json!({ "v": { "a": 1 } })).is_err());
    }

    #[test]
    fn a_tag_inside_a_string_may_hold_any_text() {
        let built =
            build_query("{ a(f: \"{{v}}\") { id } }", &json!({ "v": "a b } { x" })).unwrap();
        assert_eq!(built, "{ a(f: \"a b } { x\") { id } }");
    }

    #[test]
    fn a_tag_inside_a_block_string_may_hold_any_text() {
        let template = "{ a(f: \"\"\"x {{v}} y\"\"\") { id } }";
        assert!(build_query(template, &json!({ "v": "a b }" })).is_ok());
    }

    #[test]
    fn a_tag_after_a_string_is_outside_it() {
        // The string closes before the tag; the tag is code.
        let template = "{ a(f: \"x\", g: {{v}}) { id } }";
        assert!(build_query(template, &json!({ "v": "1 } }" })).is_err());
        // The same text with the string on the other side of the tag.
        let template = "{ a(g: {{v}}, f: \"x\") { id } }";
        assert!(build_query(template, &json!({ "v": "1 } }" })).is_err());
    }

    #[test]
    fn a_tag_in_a_comment_is_checked_because_a_line_break_ends_the_comment() {
        let template = "{ a { id } } # {{v}}";
        let error = build_query(template, &json!({ "v": "x\n} } { y" })).unwrap_err();
        assert!(error.contains("outside quotes"), "{error}");
    }

    #[test]
    fn a_tag_in_a_string_that_is_never_closed_is_checked() {
        let template = "{ a(f: \"{{v}} ) { id } }";
        assert!(build_query(template, &json!({ "v": "a b" })).is_err());
    }

    #[test]
    fn raw_tags_are_not_checked_outside_quotes() {
        let built = build_query(
            "{ a(where: { {{{part}}} }) { id } }",
            &json!({ "part": "id: { eq: 1 }" }),
        )
        .unwrap();
        assert_eq!(built, "{ a(where: { id: { eq: 1 } }) { id } }");
        assert!(
            build_query(
                "{ a(where: { {{&part}} }) { id } }",
                &json!({ "part": "id: { eq: 1 }" })
            )
            .is_ok()
        );
    }

    #[test]
    fn comments_and_end_tags_are_not_checked_as_values() {
        assert!(build_query("{ a { id } } {{! a note }}", &json!({})).is_ok());
    }

    #[test]
    fn the_variable_error_names_the_tag_and_how_to_fix_it() {
        let error =
            build_query("{ a(f: {{ name }}) { id } }", &json!({ "name": "a b" })).unwrap_err();
        assert!(
            error.contains("{{name}}") && error.contains("inside quotes"),
            "{error}"
        );
        assert!(error.contains("{{{name}}}"), "{error}");
    }

    // T3: a line break in a quoted value.

    #[test]
    fn a_line_break_in_a_quoted_value_points_at_the_variables() {
        let error = build_query(
            "{ a(f: \"{{v}}\") { id } }",
            &json!({ "v": "line1\nline2" }),
        )
        .unwrap_err();
        assert!(error.contains("never closed"), "{error}");
        assert!(
            error.contains("line break") && error.contains("Variables"),
            "{error}"
        );
        // Without a line break in the Variables the hint is not given.
        let error = build_query("{ a(f: \"{{v}} ) { id } }", &json!({ "v": "a" })).unwrap_err();
        assert!(!error.contains("line break"), "{error}");
        // A tab is fine.
        assert!(build_query("{ a(f: \"{{v}}\") { id } }", &json!({ "v": "a\tb" })).is_ok());
    }

    // T4: errors and variables.

    fn run_with_response(response: &str) -> Result<String, String> {
        run("{ a { id } }", &json!({}), |_| Ok(response.to_string()))
    }

    #[test]
    fn an_error_without_a_message_gets_a_fixed_text() {
        for response in [
            r#"{"errors":[{"message":""}]}"#,
            r#"{"errors":[{"message":"  "}]}"#,
            r#"{"errors":[{"locations":[{"line":1}],"extensions":{"secret":"detail"}}]}"#,
            r#"{"errors":[{"code":1}]}"#,
        ] {
            let error = run_with_response(response).unwrap_err();
            assert_eq!(
                error, "The Data API returned an error without a message",
                "{response}"
            );
        }
        let error = run("{ a { id } }", &json!({}), |_| Err(String::new())).unwrap_err();
        assert_eq!(error, "The Data API returned an error without a message");
        let error = run("{ a { id } }", &json!({}), |_| Err("  ".to_string())).unwrap_err();
        assert_eq!(error, "The Data API returned an error without a message");
    }

    #[test]
    fn an_error_without_a_message_names_its_path_when_it_has_one() {
        let response = r#"{"errors":[{"path":["allWebuser",0,"id"]}]}"#;
        assert_eq!(
            run_with_response(response).unwrap_err(),
            "The Data API returned an error without a message (at allWebuser.0.id)"
        );
    }

    #[test]
    fn errors_that_are_not_a_list_still_fail_the_step() {
        let data = r#""data":{"a":{"id":1}}"#;
        assert_eq!(
            run_with_response(&format!(r#"{{"errors":{{"message":"bad"}},{data}}}"#)).unwrap_err(),
            "bad"
        );
        assert_eq!(
            run_with_response(&format!(r#"{{"errors":"bad",{data}}}"#)).unwrap_err(),
            "bad"
        );
        assert!(run_with_response(&format!(r#"{{"errors":5,{data}}}"#)).is_err());
        assert!(run_with_response(&format!(r#"{{"errors":true,{data}}}"#)).is_err());
    }

    #[test]
    fn empty_errors_do_not_fail_the_step() {
        for errors in ["[]", "null", "{}", "\"\""] {
            let response = format!(r#"{{"errors":{errors},"data":{{"a":{{"id":1}}}}}}"#);
            assert!(run_with_response(&response).is_ok(), "{errors}");
        }
    }

    #[test]
    fn errors_with_partial_data_fail_the_step_and_return_no_data() {
        let response = r#"{"errors":[{"message":"first"}],"data":{"a":{"id":1}}}"#;
        assert_eq!(run_with_response(response).unwrap_err(), "first");
    }

    #[test]
    fn the_generic_errors_say_what_to_check() {
        let error = run_with_response("{}").unwrap_err();
        assert!(
            error.contains("check that the models and fields"),
            "{error}"
        );
        let error = run_with_response("not json").unwrap_err();
        assert!(
            error.contains("could not be read") && error.contains("check the query"),
            "{error}"
        );
    }

    #[test]
    fn variables_are_parsed_into_an_object() {
        assert_eq!(parse_variables("").unwrap(), json!({}));
        assert_eq!(parse_variables("  \n").unwrap(), json!({}));
        assert_eq!(parse_variables("null").unwrap(), json!({}));
        assert_eq!(parse_variables("{}").unwrap(), json!({}));
        assert_eq!(parse_variables(r#"{"a":1}"#).unwrap(), json!({ "a": 1 }));
    }

    #[test]
    fn variables_that_are_not_a_map_are_refused() {
        for text in ["[1]", "\"abc\"", "5", "true"] {
            let error = parse_variables(text).unwrap_err();
            assert!(
                error.contains("must be a map") && error.contains("Variables option"),
                "{text}: {error}"
            );
        }
        let error = parse_variables("{nope").unwrap_err();
        assert!(
            error.contains("not valid JSON") && error.contains("Variables option"),
            "{error}"
        );
    }

    // T5: cases that mutation testing showed were not covered.

    #[test]
    fn the_rendered_query_is_checked_not_the_template() {
        // The template has no bracket problem; the raw value brings one.
        let ok = build_query("{ a(f: [{{{v}}}) { id } }", &json!({ "v": "1]" }));
        assert_eq!(ok.unwrap(), "{ a(f: [1]) { id } }");
        let error = build_query("{ a(f: {{{v}}}) { id } }", &json!({ "v": "1 }" })).unwrap_err();
        assert!(error.contains("closing"), "{error}");
    }

    #[test]
    fn only_keywords_make_a_document_and_names_that_start_with_one_do_not() {
        assert_eq!(
            build_query("subscription { a { id } }", &json!({})).unwrap(),
            "subscription { a { id } }"
        );
        let built = build_query("query_log { id }", &json!({})).unwrap();
        assert!(built.starts_with("query {"), "{built}");
        let built = build_query("fragment_x { id }", &json!({})).unwrap();
        assert!(built.starts_with("query {"), "{built}");
    }

    #[test]
    fn an_alias_called_like_a_keyword_is_not_a_document() {
        for query in [
            "query: allWebuser { results { id } }",
            "mutation : allWebuser { results { id } }",
            "fragment:allWebuser { results { id } }",
        ] {
            let built = build_query(query, &json!({})).unwrap();
            assert!(built.starts_with("query {\n"), "{built}");
        }
    }

    #[test]
    fn the_wrapper_closes_on_its_own_line_after_a_comment() {
        let built = build_query("allWebuser { id } # the users", &json!({})).unwrap();
        assert!(built.ends_with("\n}"), "{built:?}");
        assert!(graphql::find_problem(&built).is_none());
    }

    // The check judges the rendered text, so a template that reads differently from the
    // rendered text cannot hide an unquoted tag.

    const INJECTION: &str = "1 } }) { results { id } } other: allUser { results { id email } } z: allWebuser(where: { id: { eq: 1 ";

    #[test]
    fn a_quote_in_a_tag_comment_cannot_hide_an_unquoted_tag() {
        let template = "query { {{! note: \"id }} allWebuser(where: { id: { eq: {{id}} }, name: { eq: \"a\" } }) { results { id } } }";
        let error = build_query(template, &json!({ "id": INJECTION })).unwrap_err();
        assert!(error.contains("outside quotes"), "{error}");
    }

    #[test]
    fn a_quote_in_a_tag_name_cannot_hide_an_unquoted_tag() {
        let template = "query { {{x\"y}} allWebuser(where: { id: { eq: {{id}} }, name: { eq: \"a\" } }) { results { id } } }";
        let error = build_query(template, &json!({ "id": INJECTION })).unwrap_err();
        assert!(error.contains("outside quotes"), "{error}");
    }

    #[test]
    fn a_backslash_before_a_tag_cannot_move_the_string_boundaries() {
        let template = "query { allWebuser(where: { path: { eq: \"C:\\{{x}}\" }, name: { eq: \"{{y}}\" }, note: { eq: \"t\\\"u\" } }) { results { id } } }";
        for x in [json!("\\"), json!(""), json!(null)] {
            let result = build_query(template, &json!({ "x": x, "y": INJECTION }));
            assert!(result.is_err(), "x = {x}: {result:?}");
        }
        // Without x the backslash escapes the quote the same way.
        let result = build_query(template, &json!({ "y": INJECTION }));
        assert!(result.is_err(), "{result:?}");
    }

    #[test]
    fn a_backslash_before_a_tag_in_a_block_string_cannot_move_the_boundaries() {
        // In the template the block string closes after the tag; in the rendered text a backslash
        // directly before the closing quotes escapes them, and the next block string opens where
        // this one ends.
        let template = "query { allWebuser(where: { path: { eq: \"\"\"C:\\{{x}}\"\"\", name: { eq: \"\"\"{{y}}\"\"\" } }) { results { id } } }";
        for x in [json!(""), json!("\\"), json!(null)] {
            let result = build_query(template, &json!({ "x": x, "y": INJECTION }));
            assert!(result.is_err(), "x = {x}: {result:?}");
        }
        let result = build_query(template, &json!({ "y": INJECTION }));
        assert!(result.is_err(), "{result:?}");
    }

    #[test]
    fn a_value_in_a_string_is_still_fine_when_the_template_has_odd_text_around_it() {
        // An escaped backslash before the tag: the string closes normally.
        let built = build_query(
            "{ a(p: \"C:\\\\{{x}}\", n: \"{{y}}\") { id } }",
            &json!({ "x": "d", "y": "a b } {" }),
        )
        .unwrap();
        assert_eq!(built, "{ a(p: \"C:\\\\d\", n: \"a b } {\") { id } }");
        // A quote inside a tag comment does not matter when the tags are in strings.
        let built = build_query(
            "{ a(n: \"{{y}}\") { id } } {{! it's a \"note }}",
            &json!({ "y": "a b" }),
        )
        .unwrap();
        assert!(built.starts_with("{ a(n: \"a b\") { id } }"), "{built}");
    }

    #[test]
    fn the_tracked_rendering_is_exactly_the_normal_rendering() {
        let variables = json!({ "a": "x & y", "b": [1, 2], "c": null, "d": { "e": "z" } });
        for template in [
            "plain",
            "{{a}} {{{a}}} {{&a}} {{! c }} {{b}} {{c}} {{d.e}} {{missing}}",
            "unterminated {{a",
            "raw unterminated {{{a}}",
            "{{{a}}}}} {{{{a}}}}",
            "{{}} {{ }} {{ a }}",
            "\"{{a}}\" # {{a}}\n{{{b}}}",
        ] {
            let tracked = render_tracked(template, &variables).unwrap();
            assert_eq!(
                tracked.text,
                template::render(template, &variables).unwrap(),
                "{template}"
            );
        }
        // The same errors as the normal rendering.
        assert!(render_tracked("{{#a}}", &variables).is_err());
        assert!(render_tracked("{{.}}", &variables).is_err());
    }

    // The two gaps from the third review: a value that changes how the rest of the query is read.

    #[test]
    fn a_value_ending_in_a_backslash_cannot_run_a_block_string_on() {
        let template = "{\n  allTask(where: {\n    name: {\n      matches: \"\"\"{{q}}\"\"\"\n    }\n    ownerId: { eq: 5 }\n    note: {\n      matches: \"\"\"{{r}}\"\"\"\n    }\n  }) { results { id } }\n}";
        let error = build_query(template, &json!({ "q": "\\", "r": "#" })).unwrap_err();
        assert!(
            error.contains("{{q}}") && error.contains("changes how the rest"),
            "{error}"
        );
        // Harmless values keep the owner filter in the code.
        let built = build_query(template, &json!({ "q": "abc", "r": "def" })).unwrap();
        assert!(built.contains("ownerId: { eq: 5 }"));
        // A backslash in the middle of a block string is fine.
        assert!(build_query(template, &json!({ "q": "a\\b c", "r": "d" })).is_ok());
    }

    #[test]
    fn a_value_that_starts_a_comment_cannot_hide_the_text_after_it() {
        let template =
            "{ allTask(where: {\n  {{extra}} ownerId: { eq: 5 }\n}) { results { id } } }";
        let error = build_query(template, &json!({ "extra": "# hi" })).unwrap_err();
        assert!(
            error.contains("{{extra}}") && error.contains("changes how the rest"),
            "{error}"
        );

        let template =
            "{ allTask(where: { ownerId: { eq: 5 } } {{x}} first: 10\n  ) { results { id } } }";
        let error = build_query(template, &json!({ "x": "#" })).unwrap_err();
        assert!(error.contains("{{x}}"), "{error}");
        // The same value at the very end of a line with nothing after it hides nothing.
        let built = build_query(
            "{ allTask { results { id } } } {{x}}",
            &json!({ "x": "# note" }),
        )
        .unwrap();
        assert_eq!(built, "{ allTask { results { id } } } # note");
    }

    #[test]
    fn a_value_in_a_comment_or_string_that_changes_nothing_is_fine() {
        let ok = [
            ("{ a(f: \"{{v}}\") { id } }", "x # y } {"),
            ("{ a { id } } # {{v}}", "x # y } {"),
            ("{ a(f: \"\"\"{{v}}\"\"\") { id } }", "x # y \" } {"),
        ];
        for (template, value) in ok {
            assert!(
                build_query(template, &json!({ "v": value })).is_ok(),
                "{template}"
            );
        }
    }

    // A property test: for every query that is built, the code outside the escaped values is what
    // it would be with harmless values, and no non-plain value is code.

    struct Lcg(u64);

    impl Lcg {
        fn next(&mut self, below: usize) -> usize {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((self.0 >> 33) as usize) % below
        }
    }

    #[test]
    fn no_query_that_is_built_lets_a_value_change_the_code_around_it() {
        const PIECES: [&str; 22] = [
            "{{x}}", "{{y}}", "{{{z}}}", "{{! c }}", "\"", "\"\"\"", "\\", "#", "{", "}", "(", ")",
            "\n", " ", "a", ":", "\\\"\"\"", "é", "{{ x }}", "{{x.0}}", "id", "[",
        ];
        const VALUES: [&str; 14] = [
            "",
            "a",
            "1",
            "a b",
            "\\",
            "\\\\",
            "#",
            "# c",
            "x\\",
            "1 } }",
            "é#",
            "\"",
            "\n}",
            "a\\\"\"\"",
        ];
        let mut random = Lcg(7);
        let mut built_count = 0;

        for _ in 0..30_000 {
            let template: String = (0..1 + random.next(14))
                .map(|_| PIECES[random.next(PIECES.len())])
                .collect();
            let variables = json!({
                "x": VALUES[random.next(VALUES.len())],
                "y": VALUES[random.next(VALUES.len())],
                "z": "a",
            });
            // `{` + `{{x}}` + `}` makes `{{{x}}}`, a raw tag: raw values are not checked on purpose.
            if ["{{{x", "{{{y", "{{{ x", "{{{ y"]
                .iter()
                .any(|raw| template.contains(raw))
            {
                continue;
            }
            let Ok(built) = build_query(&template, &variables) else {
                continue;
            };
            built_count += 1;

            let tracked = render_tracked(&template, &variables).unwrap();
            let non_plain: Vec<usize> = (0..tracked.escaped.len())
                .filter(|&i| !is_plain_value(&tracked.escaped[i].value))
                .collect();

            // The same template with every non-plain value replaced by "a" (plain ones stay).
            let harmless_of = |name: &str| {
                let value = variables[name].as_str().unwrap();
                if is_plain_value(value) {
                    value.to_string()
                } else {
                    "a".to_string()
                }
            };
            let harmless_variables =
                json!({ "x": harmless_of("x"), "y": harmless_of("y"), "z": "a" });
            let harmless = render_tracked(&template, &harmless_variables).unwrap();

            // The code outside the non-plain values, in both renderings.
            let view = |rendered: &Rendered| -> Option<String> {
                let code = graphql::code_chars(&rendered.text).ok()?;
                Some(
                    code.into_iter()
                        .filter(|(at, _)| {
                            !non_plain
                                .iter()
                                .any(|&i| rendered.escaped[i].span.contains(at))
                        })
                        .map(|(_, c)| c)
                        .collect(),
                )
            };
            assert!(view(&tracked).is_some(), "{template:?} {variables}");
            assert_eq!(
                view(&tracked),
                view(&harmless),
                "template {template:?}, variables {variables}: {built:?}"
            );

            // And no non-plain value is code.
            let code = graphql::code_chars(&tracked.text).unwrap();
            for &i in &non_plain {
                assert!(
                    !code
                        .iter()
                        .any(|(at, _)| tracked.escaped[i].span.contains(at)),
                    "template {template:?}, variables {variables}: {built:?}"
                );
            }
        }
        // Make sure the loop checks something: a good share of the templates is built.
        assert!(
            built_count > 1_000,
            "only {built_count} templates were built"
        );
    }

    #[test]
    fn thousands_of_tags_are_checked_quickly() {
        // About 5,000 quoted tags with a non-plain value, then the same with one value that fails.
        let template = "{ a(f: \"{{v}}\") { id } } ".repeat(2_500);
        let started = std::time::Instant::now();
        assert!(build_query(&template, &json!({ "v": "a b" })).is_ok());
        let failing = "{{v}} \"\"\"{{v}}\"\"\" ".repeat(1_500);
        assert!(build_query(&failing, &json!({ "v": "\\" })).is_err());
        assert!(
            started.elapsed() < std::time::Duration::from_secs(5),
            "took {:?}",
            started.elapsed()
        );
    }
}
