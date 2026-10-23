//! A copy of this file is in `data-api-query/1.0/src/template.rs`: change both.
//!
//! Fills `{{name}}` placeholders in the filter with values from the Variables option. This is a
//! small, flat part of what `templayed.js` (the template library the ActionsJS step used) did:
//! - `{{path.to.value}}`: the value, HTML-escaped (`&`, `"`, `<`, `>`) with backslashes doubled
//!   and line breaks written as `\n` and `\r`
//! - `{{{value}}}` and `{{&value}}`: the value as it is
//! - `{{! comment}}`: dropped
//! - `{{list.length}}` and `{{list.0}}` (an extension): the length or an item of a list
//!
//! Sections (`{{#list}}...{{/list}}`, `{{^list}}...{{/list}}`) are not supported and are an
//! error, not silently ignored: build the filter text in an Expression step instead.
//!
//! Differences from templayed.js for what is supported (all deliberate, listed in the README):
//! - A tag ends at its first `}}`. templayed swallowed every following `}`, which is why the
//!   ActionsJS docs said to always put spaces in filter objects. `{ eq: {{id}}}}` now works.
//! - A missing value (`{{a.b}}` where `a` is missing) renders as empty instead of throwing.
//! - A backslash in a value is doubled, so a value cannot escape the quote of its string.
//! - A line break in a value is written as `\n` or `\r`: a string cannot contain a raw line break,
//!   so a value from a multi-line text field would otherwise leave its string unterminated. (In a
//!   block string `"""..."""`, where `\n` is not an escape, it shows as those two characters.)
//! - A `|` in the template stays a `|` (templayed's newline-escaping regex `[\n|\r\n]` also
//!   matched `|`, so every `|` in an ActionsJS filter silently became a line break).

use std::borrow::Cow;

use serde_json::Value;

/// The most the rendered text may grow to. A very large list value printed into the filter would
/// otherwise make a huge query; this turns that into an error.
const MAX_RENDERED_LEN: usize = 64 * 1024;

pub fn render(template: &str, variables: &Value) -> Result<String, String> {
    let mut output = String::new();
    let mut rest = template;

    while let Some(start) = rest.find("{{") {
        output.push_str(&rest[..start]);
        let after_open = &rest[start + 2..];

        // `{{{value}}}`: raw output, closed by three braces.
        if let Some(inner) = after_open.strip_prefix('{') {
            let Some(end) = inner.find("}}}") else {
                // No closing braces: leave the text alone, like templayed does.
                output.push_str(&rest[start..]);
                rest = "";
                break;
            };
            output.push_str(&stringify(
                lookup(variables, inner[..end].trim()).as_deref(),
            ));
            rest = &inner[end + 3..];
        } else {
            let Some(end) = after_open.find("}}") else {
                output.push_str(&rest[start..]);
                rest = "";
                break;
            };
            let tag = &after_open[..end];
            match tag.chars().next() {
                Some('!') => {}
                Some('&') => {
                    output.push_str(&stringify(lookup(variables, tag[1..].trim()).as_deref()));
                }
                Some('#' | '^' | '/') => return Err(unsupported_section(tag)),
                // `{{.}}` and `{{../x}}` only mean something inside a section.
                _ if tag.trim() == "." || tag.trim().starts_with("../") => {
                    return Err(unsupported_section(tag));
                }
                _ => {
                    let value = stringify(lookup(variables, tag.trim()).as_deref());
                    output.push_str(&escape(&value));
                }
            }
            rest = &after_open[end + 2..];
        }

        if output.len() > MAX_RENDERED_LEN {
            return Err(too_long());
        }
    }

    output.push_str(rest);
    if output.len() > MAX_RENDERED_LEN {
        return Err(too_long());
    }
    Ok(output)
}

fn unsupported_section(tag: &str) -> String {
    format!(
        "The filter uses {{{{{}}}}}, which is not supported: sections ({{{{#list}}}}...{{{{/list}}}}) \
         and the tags that only work inside them ({{{{.}}}}, {{{{../x}}}}) are not available. \
         Build the filter text in an Expression step and use that variable as the filter",
        tag.trim()
    )
}

fn too_long() -> String {
    format!(
        "The filter is longer than {} KB after the variables were filled in",
        MAX_RENDERED_LEN / 1024
    )
}

/// Looks a dotted path up in the variables. Borrows the value; only `.length` builds a new one.
fn lookup<'a>(variables: &'a Value, path: &str) -> Option<Cow<'a, Value>> {
    let mut current = variables;
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
    // of the string it is placed in and let the next value out of that string. It has to be done
    // first, so the backslashes added for the line breaks below are not doubled.
    text.replace('\\', "\\\\")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
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
    fn unterminated_tags_are_left_as_text() {
        assert_eq!(render_ok("a {{oops", json!({})), "a {{oops");
        assert_eq!(render_ok("a {{{oops}}", json!({})), "a {{{oops}}");
    }

    #[test]
    fn a_backslash_in_a_value_is_escaped() {
        let vars = json!({ "v": "a\\" });
        assert_eq!(render_ok("\"{{v}}\"", vars.clone()), "\"a\\\\\"");
        // Raw tags are for the developer who knows what they are doing: no escaping at all.
        assert_eq!(render_ok("\"{{{v}}}\"", vars), "\"a\\\"");
    }

    #[test]
    fn a_line_break_in_a_value_is_written_as_an_escape() {
        let vars = json!({ "v": "line1\nline2\r\nline3" });
        assert_eq!(
            render_ok("\"{{v}}\"", vars.clone()),
            "\"line1\\nline2\\r\\nline3\""
        );
        // A backslash in the value is still doubled, and the escapes added for the line breaks are
        // not.
        assert_eq!(
            render_ok("\"{{v}}\"", json!({ "v": "a\\\nb" })),
            "\"a\\\\\\nb\""
        );
        // Raw tags insert the value untouched.
        assert_eq!(render_ok("{{{v}}}", vars), "line1\nline2\r\nline3");
    }

    #[test]
    fn sections_are_an_error_with_a_pointer_to_the_expression_step() {
        for template in [
            "{{#a}}x{{/a}}",
            "{{^a}}x{{/a}}",
            "stray {{/a}}",
            "{{.}}",
            "{{ . }}",
            "{{../a}}",
        ] {
            let error = render(template, &json!({ "a": [1] })).unwrap_err();
            assert!(error.contains("sections"), "{error}");
            assert!(error.contains("Expression step"), "{error}");
        }
    }

    #[test]
    fn output_that_grows_too_large_is_an_error() {
        let items: Vec<u32> = (0..30_000).collect();
        let error = render("{{a}}", &json!({ "a": items })).unwrap_err();
        assert!(error.contains("longer than 64 KB"), "{error}");
    }

    /// The expected output of the original templayed.js (see tests/generate_templayed_cases.mjs).
    #[test]
    fn matches_the_original_templayed_js() {
        // Update this when tests/generate_templayed_cases.mjs gets more cases, so a regenerated
        // file that quietly lost cases does not pass.
        const EXPECTED_CASES: usize = 15;
        const EXPECTED_SKIPPED: usize = 2;

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
