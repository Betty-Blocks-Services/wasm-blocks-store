//! Renders the Filter option with the Variables in filter and checks it before it goes into
//! `where: { ... }`.
//!
//! The value check (`render_tracked`, `is_plain_value`, `code_outside_values`,
//! `check_unquoted_tags`) is a copy of `data-api-query/1.0/src/query.rs`, with "query" changed to
//! "filter" in the messages: change both. It stops a value typed by an end user from adding
//! filters, fields or other models to the export (see the `data-api-query` README, "Variables").

use serde_json::Value;

use crate::graphql;
use crate::template;

/// Whether any text in `value` contains a line break.
fn has_line_break(value: &Value) -> bool {
    match value {
        Value::String(text) => text.contains(['\n', '\r']),
        Value::Array(items) => items.iter().any(has_line_break),
        Value::Object(map) => map.values().any(has_line_break),
        _ => false,
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
            "The filter could not be checked: the variables could not be placed in it. Simplify \
             the {{...}} tags in the filter",
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
                spans.get(next).is_none_or(|span| span.start > *at)
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
                "The variable {{{{{name}}}}} is used outside quotes in the filter, so it may only \
                 hold a number, true or false, a name (letters, digits, _, - and .), or a \
                 comma-separated list of those. Put it inside quotes (\"{{{{{name}}}}}\") for \
                 text, or use {{{{{{{name}}}}}}} for filter code you trust"
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
        "The value of the variable {{{{{name}}}}} changes how the rest of the filter is read: a \
         value that starts with # (a comment) or that ends in a backslash inside a \"\"\" string \
         can hide the filter text after it. Remove those characters from the value, or move the tag"
    ))
}

/// Renders and checks the filter, and returns what goes between the braces of `where: { ... }`:
/// empty for no filter. One pair of outer braces is removed if the whole filter is wrapped in
/// them, because the step adds those itself.
///
/// The rendered filter must have balanced brackets, so it cannot close `where` and add arguments
/// or fields of its own.
pub fn build(template: &str, variables: &Value) -> Result<String, String> {
    if template.trim().is_empty() {
        return Ok(String::new());
    }
    let tracked = render_tracked(template, variables)?;
    check_unquoted_tags(&tracked)?;
    let rendered = tracked.text.trim();

    if let Some(problem) = graphql::find_problem(rendered) {
        let was_templated = template.contains("{{");
        let note = if was_templated {
            " (counted after the variables were filled in)"
        } else {
            ""
        };
        let hint = match problem {
            graphql::Problem::UnmatchedClose { .. } => {
                " The filter is wrapped in curly braces for you, so write it without outer \
                 braces, for example: id: { eq: 1 }."
            }
            // As in data-api-query: only when the filter has a tag, or an unused variable with a
            // line break would get the blame for a quote the user left open.
            graphql::Problem::Unterminated { .. } if was_templated && has_line_break(variables) => {
                " A value in Variables in filter contains a line break, which cannot be inserted \
                 inside quotes with {{name}}: remove the line break first, for example in an \
                 Expression step."
            }
            _ => "",
        };
        return Err(format!(
            "The filter {}{note}.{hint}",
            problem.describe(rendered, false)
        ));
    }

    Ok(graphql::inside_outer_braces(rendered)
        .unwrap_or(rendered)
        .trim()
        .to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn an_unclosed_string_without_tags_does_not_blame_a_variable() {
        let error = build(r#"name: { eq: "x }"#, &json!({"note": "a\nb"})).unwrap_err();
        assert!(!error.contains("line break"), "{error}");
    }

    #[test]
    fn an_empty_filter_is_empty() {
        assert_eq!(build("  ", &json!({})).unwrap(), "");
    }

    #[test]
    fn variables_are_filled_in() {
        let variables = json!({"type": "Account", "id": 7});
        assert_eq!(
            build("type: { eq: \"{{type}}\" }, id: { eq: {{id}} }", &variables).unwrap(),
            "type: { eq: \"Account\" }, id: { eq: 7 }"
        );
    }

    #[test]
    fn outer_braces_are_removed() {
        assert_eq!(
            build("{ id: { eq: 1 } }", &json!({})).unwrap(),
            "id: { eq: 1 }"
        );
    }

    #[test]
    fn a_quote_in_a_value_cannot_close_the_string() {
        let variables = json!({"name": "x\" } } other { id"});
        assert_eq!(
            build("name: { eq: \"{{name}}\" }", &variables).unwrap(),
            "name: { eq: \"x&quot; } } other { id\" }"
        );
    }

    #[test]
    fn a_value_outside_quotes_may_not_be_code() {
        let variables = json!({"id": "1 } }, _or: [{ id: { gt: 0 } }"});
        let error = build("id: { eq: {{id}} }", &variables).unwrap_err();
        assert!(error.contains("outside quotes in the filter"), "{error}");
    }

    #[test]
    fn a_filter_that_closes_where_is_refused() {
        let error = build(
            "id: { eq: 1 } }) { id } other: allUser(where: {",
            &json!({}),
        )
        .unwrap_err();
        assert!(error.contains("without outer braces"), "{error}");
    }

    #[test]
    fn an_unclosed_string_is_reported() {
        let error = build("name: { eq: \"{{n}}\" }", &json!({"n": "a\nb"})).unwrap_err();
        assert!(error.contains("line break"), "{error}");
    }

    // Ported from data-api-query/1.0/src/query.rs, which has the same value check: keep the two
    // sets of tests in step.

    #[test]
    fn a_tag_directly_before_closing_braces_works() {
        assert_eq!(
            build("id: {eq: {{id}}}", &json!({ "id": 3 })).unwrap(),
            "id: {eq: 3}"
        );
    }

    #[test]
    fn list_items_and_length_can_be_inserted() {
        assert_eq!(
            build(
                "id: { in: [{{ids.0}}, {{ids.1}}] }, count: { eq: {{ids.length}} }",
                &json!({ "ids": [4, 5] })
            )
            .unwrap(),
            "id: { in: [4, 5] }, count: { eq: 2 }"
        );
    }

    #[test]
    fn sections_are_not_supported() {
        let error = build(
            "id: { in: [{{#ids}}{{.}},{{/ids}}] }",
            &json!({ "ids": [1] }),
        )
        .unwrap_err();
        assert!(error.contains("not supported"), "{error}");
        assert!(error.contains("The filter"), "{error}");
    }

    #[test]
    fn a_backslash_in_a_value_cannot_escape_the_closing_quote() {
        assert_eq!(
            build("name: { eq: \"{{name}}\" }", &json!({ "name": "a\\" })).unwrap(),
            "name: { eq: \"a\\\\\" }"
        );
    }

    #[test]
    fn a_filter_that_grows_too_large_is_an_error() {
        let items: Vec<u32> = (0..30_000).collect();
        let error = build("id: { in: [{{v}}] }", &json!({ "v": items })).unwrap_err();
        assert!(error.contains("longer than 64 KB"), "{error}");
    }

    #[test]
    fn raw_tags_do_not_escape_text() {
        assert_eq!(
            build("name: {{{f}}}", &json!({ "f": "{ eq: \"y\" }" })).unwrap(),
            "name: { eq: \"y\" }"
        );
    }

    #[test]
    fn a_filter_that_is_empty_after_filling_in_is_no_filter() {
        // query.rs refuses an empty query; an empty filter means "no filter".
        assert_eq!(build("{{x}}", &json!({ "x": "" })).unwrap(), "");
        assert_eq!(build(" {{x}} ", &json!({})).unwrap(), "");
    }

    #[test]
    fn brackets_that_do_not_pair_up_are_reported_with_the_line() {
        let error = build("id: {\n  eq: 1\n", &json!({})).unwrap_err();
        assert!(
            error.contains("never closed") && error.contains("line 1"),
            "{error}"
        );

        let error = build("id: { eq: 1 } }", &json!({})).unwrap_err();
        assert!(error.contains("without an opening one"), "{error}");

        let error = build("id: { in: [1 }", &json!({})).unwrap_err();
        assert!(error.contains("does not match"), "{error}");
    }

    #[test]
    fn brackets_in_strings_and_comments_are_ignored() {
        let filter = "# a ) in a comment\nname: { eq: \"a } b\" }";
        assert_eq!(build(filter, &json!({})).unwrap(), filter);
    }

    #[test]
    fn a_template_error_is_returned() {
        let error = build("id: { eq: 1 } {{/x}}", &json!({})).unwrap_err();
        assert!(error.contains("not supported"), "{error}");
    }

    #[test]
    fn an_unterminated_string_is_reported() {
        let error = build("name: { eq: \"abc }", &json!({})).unwrap_err();
        assert!(
            error.contains("never closed") && error.contains("line 1"),
            "{error}"
        );
    }

    #[test]
    fn a_block_string_hides_brackets_like_the_data_api_reads_it() {
        assert!(build("name: { eq: \"\"\"a } \" b\"\"\" }", &json!({})).is_ok());
    }

    #[test]
    fn a_comment_ends_at_a_carriage_return() {
        // The bracket after the \r is code, so the unclosed brace before it is found.
        let error = build("id: { eq: { a: 1 # comment\r}", &json!({})).unwrap_err();
        assert!(error.contains("never closed"), "{error}");
    }

    #[test]
    fn a_raw_value_cannot_hide_a_bracket_inside_a_comment() {
        // The value starts a comment; the closing brace on the next line is still code.
        let built = build("id: { eq: {{{v}}}\n}", &json!({ "v": "1 #" }));
        assert!(built.is_ok(), "{built:?}");
        let error = build("id: { eq: {{{v}}}\n} }", &json!({ "v": "1 #" })).unwrap_err();
        assert!(error.contains("without an opening one"), "{error}");
        let error = build("id: { in: [{{{v}}}\n}", &json!({ "v": "1 #" })).unwrap_err();
        assert!(error.contains("does not match"), "{error}");
    }

    #[test]
    fn bracket_errors_name_the_line_but_do_not_quote_it() {
        let error = build("id: { eq: 1 },\nname: { eq: \"secret\"\n", &json!({})).unwrap_err();
        assert!(error.contains("line 2"), "{error}");
        assert!(!error.contains("secret"), "{error}");
    }

    #[test]
    fn a_bracket_error_in_a_templated_filter_says_the_lines_are_after_filling_in() {
        let error = build("id: { eq: {{v}}", &json!({ "v": 1 })).unwrap_err();
        assert!(
            error.contains("after the variables were filled in"),
            "{error}"
        );
        let plain = build("id: { eq: 1", &json!({})).unwrap_err();
        assert!(!plain.contains("after the variables"), "{plain}");
        // Text the template leaves alone, spaces and line breaks around it included, is not
        // "after the variables were filled in" either.
        let padded = build("\n  {\r\n id: {eq: 1}\r\n  ", &json!({})).unwrap_err();
        assert!(!padded.contains("after the variables"), "{padded}");
    }

    // T1: escaped tags outside a string may only hold plain values.

    #[test]
    fn an_unquoted_tag_cannot_add_a_filter_clause() {
        let id = "7 }, _or: [{ id: { gt: 0 } }], x: { eq: 1 ";
        let error = build("owner: { eq: {{id}} }", &json!({ "id": id })).unwrap_err();
        assert!(error.contains("outside quotes"), "{error}");
        assert!(error.contains("{{id}}"), "{error}");
        assert!(!error.contains("_or"), "the value is not echoed: {error}");
    }

    #[test]
    fn an_unquoted_tag_cannot_add_a_field_on_another_model() {
        let id = "1 } }) { results { id } } other: allUser { results { id email } } x: allWebuser(where: { id: { eq: 1 ";
        let error = build("id: { eq: {{id}} }", &json!({ "id": id })).unwrap_err();
        assert!(error.contains("outside quotes"), "{error}");
    }

    #[test]
    fn an_unquoted_tag_cannot_add_a_second_operation() {
        let id = "1 } }) { totalCount } }\nmutation M { deleteWebuser(id: 1) { id } }\nquery Q { allWebuser(where: { id: { eq: 1 ";
        let error = build("id: { eq: {{id}} }", &json!({ "id": id })).unwrap_err();
        assert!(error.contains("outside quotes"), "{error}");
    }

    #[test]
    fn plain_values_are_fine_outside_quotes() {
        let template = "id: { eq: {{id}} }, flag: { eq: {{flag}} }, {{field}}: { eq: 1 }, ids: { in: [{{ids}}] }, n: { eq: {{n.0}} }";
        let variables = json!({
            "id": 7, "flag": true, "field": "first_name",
            "ids": [1, 2, 3], "n": [-1.5],
        });
        assert_eq!(
            build(template, &variables).unwrap(),
            "id: { eq: 7 }, flag: { eq: true }, first_name: { eq: 1 }, ids: { in: [1,2,3] }, n: { eq: -1.5 }"
        );
    }

    #[test]
    fn a_missing_or_empty_value_outside_quotes_is_not_an_injection() {
        assert!(build("id: { eq: {{nope}} }", &json!({})).is_ok());
        assert!(build("id: { eq: {{x}} }", &json!({ "x": null })).is_ok());
    }

    #[test]
    fn text_with_spaces_or_symbols_outside_quotes_is_refused() {
        for value in [
            "a b", "x:1", "a)", "1;2", "a\"b", "{", "#", "1,", ",1", "a,,b", "é",
        ] {
            assert!(
                build("id: { eq: {{v}} }", &json!({ "v": value })).is_err(),
                "{value:?}"
            );
        }
        // An object prints as "[object Object]".
        assert!(build("id: { eq: {{v}} }", &json!({ "v": { "a": 1 } })).is_err());
    }

    #[test]
    fn plain_values_are_numbers_booleans_names_and_lists_of_those() {
        for value in [
            "",
            "7",
            "-1.5",
            "true",
            "first_name",
            "a-b.c",
            "1,2,3",
            "a,b",
        ] {
            assert!(is_plain_value(value), "{value:?}");
        }
        for value in [
            "a b", "x:1", "a)", "1;2", "a\"b", "{", "#", "1,", ",1", "a,,b", "é", ",", "a\\",
        ] {
            assert!(!is_plain_value(value), "{value:?}");
        }
    }

    #[test]
    fn a_tag_inside_a_string_may_hold_any_text() {
        assert_eq!(
            build("name: { eq: \"{{v}}\" }", &json!({ "v": "a b } { x" })).unwrap(),
            "name: { eq: \"a b } { x\" }"
        );
    }

    #[test]
    fn a_tag_inside_a_block_string_may_hold_any_text() {
        let template = "name: { eq: \"\"\"x {{v}} y\"\"\" }";
        assert!(build(template, &json!({ "v": "a b }" })).is_ok());
    }

    #[test]
    fn a_tag_after_a_string_is_outside_it() {
        // The string closes before the tag; the tag is code.
        let template = "name: { eq: \"x\" }, id: { eq: {{v}} }";
        assert!(build(template, &json!({ "v": "1 } }" })).is_err());
        // The same text with the string on the other side of the tag.
        let template = "id: { eq: {{v}} }, name: { eq: \"x\" }";
        assert!(build(template, &json!({ "v": "1 } }" })).is_err());
    }

    #[test]
    fn a_tag_in_a_comment_is_checked_because_a_line_break_ends_the_comment() {
        let template = "id: { eq: 1 } # {{v}}";
        let error = build(template, &json!({ "v": "x\n} } { y" })).unwrap_err();
        assert!(error.contains("outside quotes"), "{error}");
    }

    #[test]
    fn a_tag_in_a_string_that_is_never_closed_is_checked() {
        let error = build("name: { eq: \"{{v}} }", &json!({ "v": "a b" })).unwrap_err();
        assert!(error.contains("never closed"), "{error}");
    }

    #[test]
    fn raw_tags_are_not_checked_outside_quotes() {
        let part = json!({ "part": "id: { eq: 1 }" });
        assert_eq!(build("{{{part}}}", &part).unwrap(), "id: { eq: 1 }");
        assert_eq!(build("{{&part}}", &part).unwrap(), "id: { eq: 1 }");
    }

    #[test]
    fn comments_and_end_tags_are_not_checked_as_values() {
        assert!(build("id: { eq: 1 } {{! a note }}", &json!({})).is_ok());
    }

    #[test]
    fn the_variable_error_names_the_tag_and_how_to_fix_it() {
        let error = build("id: { eq: {{ name }} }", &json!({ "name": "a b" })).unwrap_err();
        assert!(
            error.contains("{{name}}") && error.contains("inside quotes"),
            "{error}"
        );
        assert!(error.contains("{{{name}}}"), "{error}");
    }

    // T3: a line break in a quoted value.

    #[test]
    fn a_line_break_in_a_quoted_value_points_at_the_variables() {
        let error = build("name: { eq: \"{{v}}\" }", &json!({ "v": "line1\nline2" })).unwrap_err();
        assert!(error.contains("never closed"), "{error}");
        assert!(
            error.contains("line break") && error.contains("Variables in filter"),
            "{error}"
        );
        // A carriage return counts too.
        let error = build("name: { eq: \"{{v}}\" }", &json!({ "v": ["a\rb"] })).unwrap_err();
        assert!(error.contains("line break"), "{error}");
        // Without a line break in the Variables the hint is not given.
        let error = build("name: { eq: \"{{v}} }", &json!({ "v": "a" })).unwrap_err();
        assert!(!error.contains("line break"), "{error}");
        // A tab is fine.
        assert!(build("name: { eq: \"{{v}}\" }", &json!({ "v": "a\tb" })).is_ok());
    }

    // T5: cases that mutation testing showed were not covered.

    #[test]
    fn the_rendered_filter_is_checked_not_the_template() {
        // The template has no bracket problem; the raw value brings one.
        let ok = build("id: { in: [{{{v}}} }", &json!({ "v": "1]" }));
        assert_eq!(ok.unwrap(), "id: { in: [1] }");
        let error = build("id: { eq: {{{v}}} }", &json!({ "v": "1 }" })).unwrap_err();
        assert!(error.contains("closing"), "{error}");
    }

    // The check judges the rendered text, so a template that reads differently from the
    // rendered text cannot hide an unquoted tag.

    const INJECTION: &str = "1 } }) { results { id } } other: allUser { results { id email } } z: allWebuser(where: { id: { eq: 1 ";

    #[test]
    fn a_quote_in_a_tag_comment_cannot_hide_an_unquoted_tag() {
        let template = "{{! note: \"id }} id: { eq: {{id}} }, name: { eq: \"a\" }";
        let error = build(template, &json!({ "id": INJECTION })).unwrap_err();
        assert!(error.contains("outside quotes"), "{error}");
    }

    #[test]
    fn a_quote_in_a_tag_name_cannot_hide_an_unquoted_tag() {
        let template = "{{x\"y}} id: { eq: {{id}} }, name: { eq: \"a\" }";
        let error = build(template, &json!({ "id": INJECTION })).unwrap_err();
        assert!(error.contains("outside quotes"), "{error}");
    }

    #[test]
    fn a_backslash_before_a_tag_cannot_move_the_string_boundaries() {
        let template =
            "path: { eq: \"C:\\{{x}}\" }, name: { eq: \"{{y}}\" }, note: { eq: \"t\\\"u\" }";
        for x in [json!("\\"), json!(""), json!(null)] {
            let result = build(template, &json!({ "x": x, "y": INJECTION }));
            assert!(result.is_err(), "x = {x}: {result:?}");
        }
        // Without x the backslash escapes the quote the same way.
        let result = build(template, &json!({ "y": INJECTION }));
        assert!(result.is_err(), "{result:?}");
    }

    #[test]
    fn a_backslash_before_a_tag_in_a_block_string_cannot_move_the_boundaries() {
        // In the template the block string closes after the tag; in the rendered text a backslash
        // directly before the closing quotes escapes them, and the next block string opens where
        // this one ends.
        let template = "path: { eq: \"\"\"C:\\{{x}}\"\"\", name: { eq: \"\"\"{{y}}\"\"\" } }";
        for x in [json!(""), json!("\\"), json!(null)] {
            let result = build(template, &json!({ "x": x, "y": INJECTION }));
            assert!(result.is_err(), "x = {x}: {result:?}");
        }
        let result = build(template, &json!({ "y": INJECTION }));
        assert!(result.is_err(), "{result:?}");
    }

    #[test]
    fn a_value_in_a_string_is_still_fine_when_the_template_has_odd_text_around_it() {
        // An escaped backslash before the tag: the string closes normally.
        let built = build(
            "path: { eq: \"C:\\\\{{x}}\" }, name: { eq: \"{{y}}\" }",
            &json!({ "x": "d", "y": "a b } {" }),
        )
        .unwrap();
        assert_eq!(
            built,
            "path: { eq: \"C:\\\\d\" }, name: { eq: \"a b } {\" }"
        );
        // A quote inside a tag comment does not matter when the tags are in strings.
        let built = build(
            "name: { eq: \"{{y}}\" } {{! it's a \"note }}",
            &json!({ "y": "a b" }),
        )
        .unwrap();
        assert_eq!(built, "name: { eq: \"a b\" }");
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

    // Check 2: a value that changes how the rest of the filter is read.

    #[test]
    fn a_value_ending_in_a_backslash_cannot_run_a_block_string_on() {
        let template = "name: {\n  matches: \"\"\"{{q}}\"\"\"\n}\nownerId: { eq: 5 }\nnote: {\n  matches: \"\"\"{{r}}\"\"\"\n}";
        let error = build(template, &json!({ "q": "\\", "r": "#" })).unwrap_err();
        assert!(
            error.contains("{{q}}") && error.contains("changes how the rest of the filter"),
            "{error}"
        );
        // Harmless values keep the owner filter in the code.
        let built = build(template, &json!({ "q": "abc", "r": "def" })).unwrap();
        assert!(built.contains("ownerId: { eq: 5 }"));
        // A backslash in the middle of a block string is fine.
        assert!(build(template, &json!({ "q": "a\\b c", "r": "d" })).is_ok());
    }

    #[test]
    fn a_value_that_starts_a_comment_cannot_hide_the_text_after_it() {
        let template = "{{extra}} ownerId: { eq: 5 }";
        let error = build(template, &json!({ "extra": "# hi" })).unwrap_err();
        assert!(
            error.contains("{{extra}}") && error.contains("changes how the rest of the filter"),
            "{error}"
        );

        let template = "ownerId: { eq: 5 } {{x}} name: { eq: 1 }\n";
        let error = build(template, &json!({ "x": "#" })).unwrap_err();
        assert!(error.contains("{{x}}"), "{error}");
        // The same value at the very end of a line with nothing after it hides nothing (the step
        // puts the closing brace of `where` on a line of its own).
        assert_eq!(
            build("ownerId: { eq: 5 } {{x}}", &json!({ "x": "# note" })).unwrap(),
            "ownerId: { eq: 5 } # note"
        );
    }

    #[test]
    fn a_value_in_a_comment_or_string_that_changes_nothing_is_fine() {
        let ok = [
            ("name: { eq: \"{{v}}\" }", "x # y } {"),
            ("id: { eq: 1 } # {{v}}", "x # y } {"),
            ("name: { eq: \"\"\"{{v}}\"\"\" }", "x # y \" } {"),
        ];
        for (template, value) in ok {
            assert!(
                build(template, &json!({ "v": value })).is_ok(),
                "{template}"
            );
        }
    }

    // A property test: for every filter that is built, the code outside the escaped values is what
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
    fn no_filter_that_is_built_lets_a_value_change_the_code_around_it() {
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
            let Ok(built) = build(&template, &variables) else {
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
        // 2,500 quoted tags with a non-plain value, then 3,000 tags with values that fail.
        let template = "a: { eq: \"{{v}}\" } ".repeat(2_500);
        let started = std::time::Instant::now();
        assert!(build(&template, &json!({ "v": "a b" })).is_ok());
        let failing = "{{v}} \"\"\"{{v}}\"\"\" ".repeat(1_500);
        assert!(build(&failing, &json!({ "v": "\\" })).is_err());
        assert!(
            started.elapsed() < std::time::Duration::from_secs(5),
            "took {:?}",
            started.elapsed()
        );
    }
}
