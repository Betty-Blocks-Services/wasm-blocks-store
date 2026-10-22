//! A small port of the subset of `templayed.js` (a Mustache-style engine) that the ActionsJS
//! Data API Request step used for its filter, so existing filters keep working.
//!
//! Supported, matching templayed:
//! - `{{path.to.value}}`: HTML-escaped (`& " < >`), `{{{value}}}` and `{{&value}}`: raw
//! - `{{! comment}}`: dropped
//! - `{{#key}}...{{/key}}`: renders once for `true`, once per element for an array
//! - `{{^key}}...{{/key}}`: renders for empty arrays and falsy values
//! - `{{.}}` for the current element, `{{../key}}` for the parent context, `{{list.length}}`
//!
//! Differences from templayed (all deliberate, listed in the README):
//! - A tag ends at its first `}}`. templayed swallowed every following `}`, which is why the
//!   ActionsJS docs said to always put spaces in filter objects. `{ eq: {{id}}}}` now works.
//! - A missing intermediate value (`{{a.b}}` where `a` is missing) renders as empty instead of
//!   throwing a TypeError.
//! - A section whose value is a non-empty string, an object or a number renders nothing, where
//!   templayed iterated over the characters of a string.
//! - An unclosed or unmatched section is an error instead of a JavaScript syntax error.
//! - A `|` in the template stays a `|`. templayed's newline-escaping regex `[\n|\r\n]` also matched
//!   `|`, so every `|` in an ActionsJS filter silently became a line break.

use std::borrow::Cow;

use serde_json::Value;

/// The most the rendered text may grow to. A section over a big array that prints the array again
/// for every item multiplies quickly; this turns that into an error instead of running out of memory.
const MAX_RENDERED_LEN: usize = 64 * 1024;

pub fn render(template: &str, variables: &Value) -> Result<String, String> {
    render_block(template, &mut vec![variables.clone()])
}

fn too_long() -> String {
    format!(
        "The filter is longer than {} KB after the variables were filled in",
        MAX_RENDERED_LEN / 1024
    )
}

fn render_block(template: &str, stack: &mut Vec<Value>) -> Result<String, String> {
    let mut output = String::new();
    let mut rest = template;

    while let Some(start) = rest.find("{{") {
        if output.len() > MAX_RENDERED_LEN {
            return Err(too_long());
        }
        output.push_str(&rest[..start]);
        let after_open = &rest[start + 2..];

        // `{{{value}}}`: raw output, closed by three braces.
        if let Some(inner) = after_open.strip_prefix('{') {
            let Some(end) = inner.find("}}}") else {
                output.push_str(&rest[start..]);
                return Ok(output);
            };
            output.push_str(&stringify(lookup(stack, inner[..end].trim()).as_deref()));
            rest = &inner[end + 3..];
            continue;
        }

        let Some(end) = after_open.find("}}") else {
            // No closing braces: leave the text alone, like templayed does.
            output.push_str(&rest[start..]);
            return Ok(output);
        };
        let tag = &after_open[..end];
        let remainder = &after_open[end + 2..];

        match tag.chars().next() {
            Some('!') => rest = remainder,
            Some('&') => {
                output.push_str(&stringify(lookup(stack, tag[1..].trim()).as_deref()));
                rest = remainder;
            }
            Some(marker @ ('#' | '^')) => {
                let key = tag[1..].trim();
                let Some((close_at, close_len)) = find_closing_tag(remainder, key) else {
                    return Err(format!(
                        "The template section {{{{{marker}{key}}}}} is never closed with {{{{/{key}}}}}"
                    ));
                };
                let inner = &remainder[..close_at];
                render_section(&mut output, marker, key, inner, stack)?;
                rest = &remainder[close_at + close_len..];
            }
            Some('/') => {
                return Err(format!(
                    "Found the closing tag {{{{{}}}}} without a matching opening tag",
                    tag.trim()
                ));
            }
            _ => {
                let value = stringify(lookup(stack, tag.trim()).as_deref());
                output.push_str(&escape(&value));
                rest = remainder;
            }
        }
    }

    output.push_str(rest);
    if output.len() > MAX_RENDERED_LEN {
        return Err(too_long());
    }
    Ok(output)
}

/// Finds the first `{{/key}}` in `text` (spaces around the key are allowed, as in the opening
/// tag) and returns where it starts and how long it is.
fn find_closing_tag(text: &str, key: &str) -> Option<(usize, usize)> {
    let mut search_from = 0;
    while let Some(offset) = text[search_from..].find("{{/") {
        let start = search_from + offset;
        let after = &text[start + 3..];
        let end = after.find("}}")?;
        if after[..end].trim() == key {
            return Some((start, 3 + end + 2));
        }
        search_from = start + 3;
    }
    None
}

fn render_section(
    output: &mut String,
    marker: char,
    key: &str,
    inner: &str,
    stack: &mut Vec<Value>,
) -> Result<(), String> {
    let value = lookup(stack, key);

    if marker == '^' {
        let empty = match value.as_deref() {
            Some(Value::Array(items)) => items.is_empty(),
            other => !is_truthy(other),
        };
        if empty {
            output.push_str(&render_block(inner, stack)?);
        }
        return Ok(());
    }

    // The items are copied out so the stack can be changed while they are rendered.
    let items = match value.as_deref() {
        Some(Value::Bool(true)) => {
            output.push_str(&render_block(inner, stack)?);
            return Ok(());
        }
        Some(Value::Array(items)) => items.clone(),
        _ => return Ok(()),
    };
    for item in items {
        stack.push(item);
        let rendered = render_block(inner, stack);
        stack.pop();
        output.push_str(&rendered?);
        if output.len() > MAX_RENDERED_LEN {
            return Err(too_long());
        }
    }
    Ok(())
}

/// Looks a path up in the current context (`{{../x}}` walks up the context stack). Borrows the
/// value; only `.length` builds a new one.
fn lookup<'a>(stack: &'a [Value], path: &str) -> Option<Cow<'a, Value>> {
    let mut depth = 1;
    let mut path = path;
    while let Some(stripped) = path.strip_prefix("../") {
        depth += 1;
        path = stripped;
    }

    let mut current: &'a Value = stack.get(stack.len().checked_sub(depth)?)?;
    if path == "." {
        return Some(Cow::Borrowed(current));
    }

    let keys: Vec<&str> = path.split('.').collect();
    for (position, key) in keys.iter().enumerate() {
        let is_last = position + 1 == keys.len();
        current = match (current, *key) {
            (Value::Array(items), "length") if is_last => {
                return Some(Cow::Owned(Value::from(items.len())));
            }
            (Value::String(text), "length") if is_last => {
                return Some(Cow::Owned(Value::from(text.encode_utf16().count())));
            }
            (Value::Object(map), key) => map.get(key)?,
            (Value::Array(items), key) => items.get(key.parse::<usize>().ok()?)?,
            _ => return None,
        };
    }
    Some(Cow::Borrowed(current))
}

/// JavaScript truthiness for a JSON value.
fn is_truthy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) | Some(Value::Bool(false)) => false,
        Some(Value::String(text)) => !text.is_empty(),
        Some(Value::Number(number)) => number.as_f64().is_some_and(|n| n != 0.0),
        Some(_) => true,
    }
}

/// What templayed prints for a value: missing values and `null` become empty text, everything
/// else is JavaScript's `String(value)`. (templayed's check is `s || s == 0 ? s : ""`, and
/// `false == 0` is true in JavaScript, so `false` prints as "false".)
fn stringify(value: Option<&Value>) -> String {
    value.map(js_to_string).unwrap_or_default()
}

/// JavaScript's `String(value)` for a JSON value.
fn js_to_string(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::Bool(flag) => flag.to_string(),
        Value::Number(number) => number_to_string(number),
        Value::String(text) => text.clone(),
        // `String([1, 2])` is "1,2"; null elements are empty, nested arrays are flattened.
        Value::Array(items) => items.iter().map(js_to_string).collect::<Vec<_>>().join(","),
        Value::Object(_) => String::from("[object Object]"),
    }
}

fn number_to_string(number: &serde_json::Number) -> String {
    if let Some(integer) = number.as_i64() {
        return integer.to_string();
    }
    if let Some(integer) = number.as_u64() {
        return integer.to_string();
    }
    let float = number.as_f64().unwrap_or(0.0);
    if float == 0.0 {
        return String::from("0");
    }
    // Rust prints the shortest digits that round-trip, like JavaScript, but never switches to
    // exponent notation for very large or small numbers (JavaScript does beyond 1e21 / 1e-6).
    float.to_string()
}

fn escape(text: &str) -> String {
    // A backslash is escaped too: a value ending in `\` would otherwise escape the closing quote
    // of the string it is placed in and let the next value out of that string.
    text.replace('\\', "\\\\")
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('>', "&gt;")
        .replace('<', "&lt;")
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn render_ok(template: &str, variables: Value) -> String {
        render(template, &variables).unwrap()
    }

    #[test]
    fn plain_text_is_untouched() {
        assert_eq!(render_ok("id: { eq: 1 }", json!({})), "id: { eq: 1 }");
    }

    #[test]
    fn substitutes_text_and_numbers() {
        let vars = json!({ "name": "Jan", "id": 7 });
        assert_eq!(
            render_ok("name: { eq: \"{{name}}\" }, id: { eq: {{ id }} }", vars),
            "name: { eq: \"Jan\" }, id: { eq: 7 }"
        );
    }

    #[test]
    fn text_variables_are_substituted_raw_without_quotes() {
        // The Map option delivers a text variable as the JSON string "1".
        assert_eq!(
            render_ok("id: { eq: {{id}} }", json!({ "id": "1" })),
            "id: { eq: 1 }"
        );
    }

    #[test]
    fn closing_braces_directly_after_a_tag_work() {
        assert_eq!(
            render_ok("{ id: { eq: {{id}}}}", json!({ "id": 5 })),
            "{ id: { eq: 5}}"
        );
    }

    #[test]
    fn missing_variables_render_empty() {
        assert_eq!(render_ok("a{{nope}}b", json!({})), "ab");
        assert_eq!(render_ok("a{{nope.deeper.still}}b", json!({})), "ab");
    }

    #[test]
    fn null_and_empty_render_empty_but_false_and_zero_do_not() {
        let vars = json!({ "f": false, "t": true, "n": null, "s": "", "z": 0 });
        assert_eq!(
            render_ok("[{{f}}][{{t}}][{{n}}][{{s}}][{{z}}]", vars),
            "[false][true][][][0]"
        );
    }

    #[test]
    fn double_braces_html_escape_and_triple_braces_do_not() {
        let vars = json!({ "v": "a&b\"c<d>e" });
        assert_eq!(render_ok("{{v}}", vars.clone()), "a&amp;b&quot;c&lt;d&gt;e");
        assert_eq!(render_ok("{{{v}}}", vars.clone()), "a&b\"c<d>e");
        assert_eq!(render_ok("{{&v}}", vars), "a&b\"c<d>e");
    }

    #[test]
    fn arrays_are_joined_with_commas_like_javascript() {
        let vars = json!({ "ids": [1, 2, 3], "mixed": [1, null, "x", [2, 3]] });
        assert_eq!(render_ok("[{{ids}}]", vars.clone()), "[1,2,3]");
        assert_eq!(render_ok("{{mixed}}", vars), "1,,x,2,3");
    }

    #[test]
    fn objects_render_like_javascript() {
        assert_eq!(
            render_ok("{{o}}", json!({ "o": { "a": 1 } })),
            "[object Object]"
        );
    }

    #[test]
    fn dotted_paths_and_array_indexes() {
        let vars = json!({ "webuser": { "id": 1, "tags": ["a", "b"] } });
        assert_eq!(render_ok("{{webuser.id}}", vars.clone()), "1");
        assert_eq!(render_ok("{{webuser.tags.1}}", vars.clone()), "b");
        assert_eq!(render_ok("{{webuser.tags.length}}", vars), "2");
    }

    #[test]
    fn numbers_format_like_javascript() {
        let vars = json!({ "a": 1.0, "b": 1.5, "c": -3, "d": 100000000000.0 });
        assert_eq!(
            render_ok("{{a}} {{b}} {{c}} {{d}}", vars),
            "1 1.5 -3 100000000000"
        );
    }

    #[test]
    fn comments_are_dropped() {
        assert_eq!(render_ok("a{{! a note }}b", json!({})), "ab");
    }

    #[test]
    fn sections_iterate_arrays_with_the_current_item_as_context() {
        let vars = json!({ "users": [{ "name": "a" }, { "name": "b" }] });
        assert_eq!(render_ok("{{#users}}<{{name}}>{{/users}}", vars), "<a><b>");
    }

    #[test]
    fn dot_is_the_current_item_and_dotdot_the_parent() {
        let vars = json!({ "list": [1, 2], "sep": "-" });
        assert_eq!(render_ok("{{#list}}{{.}}{{../sep}}{{/list}}", vars), "1-2-");
    }

    #[test]
    fn true_sections_render_once_and_false_sections_not_at_all() {
        let vars = json!({ "yes": true, "no": false });
        assert_eq!(render_ok("{{#yes}}A{{/yes}}{{#no}}B{{/no}}", vars), "A");
    }

    #[test]
    fn inverted_sections_render_for_empty_and_falsy_values() {
        let vars = json!({ "empty": [], "full": [1], "no": false, "yes": true });
        assert_eq!(
            render_ok(
                "{{^empty}}1{{/empty}}{{^full}}2{{/full}}{{^no}}3{{/no}}{{^yes}}4{{/yes}}{{^missing}}5{{/missing}}",
                vars
            ),
            "135"
        );
    }

    #[test]
    fn sections_can_nest_with_different_keys() {
        let vars = json!({ "rows": [{ "cells": [1, 2] }, { "cells": [3] }] });
        assert_eq!(
            render_ok("{{#rows}}[{{#cells}}{{.}}{{/cells}}]{{/rows}}", vars),
            "[12][3]"
        );
    }

    #[test]
    fn unterminated_tags_are_left_as_text() {
        assert_eq!(render_ok("a {{oops", json!({})), "a {{oops");
        assert_eq!(render_ok("a {{{oops}}", json!({})), "a {{{oops}}");
    }

    #[test]
    fn unclosed_and_unmatched_sections_are_errors() {
        assert!(
            render("{{#a}}never closed", &json!({ "a": true }))
                .unwrap_err()
                .contains("never closed")
        );
        assert!(
            render("stray {{/a}}", &json!({}))
                .unwrap_err()
                .contains("without a matching")
        );
    }

    #[test]
    fn a_backslash_in_a_value_is_escaped() {
        let vars = json!({ "v": "a\\" });
        assert_eq!(render_ok("\"{{v}}\"", vars.clone()), "\"a\\\\\"");
        // Raw tags are for the developer who knows what they are doing: no escaping at all.
        assert_eq!(render_ok("\"{{{v}}}\"", vars), "\"a\\\"");
    }

    #[test]
    fn section_keys_may_have_spaces_in_the_closing_tag() {
        let vars = json!({ "ids": [1, 2] });
        assert_eq!(render_ok("{{# ids }}{{.}}{{/ ids }}", vars.clone()), "12");
        assert_eq!(render_ok("{{#ids}}{{.}}{{/ids }}", vars.clone()), "12");
        assert_eq!(render_ok("{{^ none }}x{{/ none }}", vars), "x");
    }

    #[test]
    fn output_that_grows_too_large_is_an_error() {
        let items: Vec<u32> = (0..3000).collect();
        let error = render("{{#a}}{{../a}}{{/a}}", &json!({ "a": items })).unwrap_err();
        assert!(error.contains("longer than 64 KB"), "{error}");
    }

    /// The expected output of the original templayed.js (see tests/generate_templayed_cases.mjs).
    #[test]
    fn matches_the_original_templayed_js() {
        // Update this when tests/generate_templayed_cases.mjs gets more cases, so a regenerated
        // file that quietly lost cases does not pass.
        const EXPECTED_CASES: usize = 25;
        const EXPECTED_SKIPPED: usize = 4;

        let fixture: Value =
            serde_json::from_str(include_str!("../tests/templayed_cases.json")).unwrap();
        let cases = fixture["cases"].as_array().unwrap();
        assert_eq!(cases.len(), EXPECTED_CASES);
        assert_eq!(
            fixture["skipped"].as_array().unwrap().len(),
            EXPECTED_SKIPPED
        );

        for case in cases {
            let template = case["template"].as_str().unwrap();
            let expected = case["expected"].as_str().unwrap();
            assert_eq!(
                render(template, &case["variables"]).unwrap(),
                expected,
                "template {template:?} with {}",
                case["variables"]
            );
        }
    }
}
