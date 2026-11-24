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
            "The query could not be checked: the variables could not be placed in it. Simplify \
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
// Kept as written in data-api-query (a copy), not rewritten to `is_none_or`.
#[allow(clippy::nonminimal_bool)]
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
        let note = if template.contains("{{") {
            " (counted after the variables were filled in)"
        } else {
            ""
        };
        let hint = match problem {
            graphql::Problem::UnmatchedClose { .. } => {
                " The filter is wrapped in curly braces for you, so write it without outer \
                 braces, for example: id: { eq: 1 }."
            }
            graphql::Problem::Unterminated { .. } if has_line_break(variables) => {
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
}
