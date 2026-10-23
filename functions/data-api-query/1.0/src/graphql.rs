//! A copy of `data-api-request/1.0/src/graphql.rs`: change both.
//!
//! Just enough GraphQL lexing to check the text a developer typed or a variable filled in: which
//! characters are code (not inside a string or a comment), whether the brackets pair up, and which
//! fields a selection picks at its top level.
//!
//! The lexer follows the GraphQL specification, because the Data API reads the query the same way
//! and the error messages should agree with what it would report. The bracket check only makes the
//! errors readable; it is not a security boundary (what a value may be is decided where it is
//! inserted):
//! - a `#` comment ends at `\n`, `\r` or `\r\n`;
//! - a string ends at its closing `"`, may not contain a line break, and `\"` does not close it;
//! - a block string runs from `"""` to the next `"""` (`\"""` is an escaped triple quote);
//! - an unterminated string is an error.

/// A string that is never closed (a line break inside a normal string counts as not closed).
#[derive(Debug, PartialEq, Eq)]
pub struct UnterminatedString {
    pub at: usize,
}

/// The characters of `text` that are code, with their byte offsets. Strings and comments are
/// left out; a whole string is reported as a single space so words around it stay separate.
pub fn code_chars(text: &str) -> Result<Vec<(usize, char)>, UnterminatedString> {
    let mut chars = Vec::new();
    let mut index = 0;

    while index < text.len() {
        let rest = &text[index..];
        let Some(character) = rest.chars().next() else {
            break;
        };

        match character {
            '#' => index += rest.find(['\n', '\r']).unwrap_or(rest.len()),
            '"' if rest.starts_with("\"\"\"") => {
                let start = index;
                index += 3;
                loop {
                    let rest = &text[index..];
                    if rest.starts_with("\\\"\"\"") {
                        index += 4;
                    } else if rest.starts_with("\"\"\"") {
                        index += 3;
                        break;
                    } else if let Some(next) = rest.chars().next() {
                        index += next.len_utf8();
                    } else {
                        return Err(UnterminatedString { at: start });
                    }
                }
                chars.push((start, ' '));
            }
            '"' => {
                let start = index;
                index += 1;
                loop {
                    match text[index..].chars().next() {
                        None | Some('\n') | Some('\r') => {
                            return Err(UnterminatedString { at: start });
                        }
                        Some('"') => {
                            index += 1;
                            break;
                        }
                        Some('\\') => {
                            index += 1;
                            match text[index..].chars().next() {
                                None | Some('\n') | Some('\r') => {
                                    return Err(UnterminatedString { at: start });
                                }
                                Some(escaped) => index += escaped.len_utf8(),
                            }
                        }
                        Some(other) => index += other.len_utf8(),
                    }
                }
                chars.push((start, ' '));
            }
            _ => {
                chars.push((index, character));
                index += character.len_utf8();
            }
        }
    }
    Ok(chars)
}

#[derive(Debug, PartialEq, Eq)]
pub enum Problem {
    /// A string that is never closed.
    Unterminated { at: usize },
    /// A closing bracket with nothing open.
    UnmatchedClose { close: char, at: usize },
    /// A closing bracket of another kind than the one that is open.
    Mismatch {
        close: char,
        close_at: usize,
        open: char,
        open_at: usize,
    },
    /// An opening bracket that is never closed.
    Unclosed { open: char, at: usize },
}

pub fn find_problem(text: &str) -> Option<Problem> {
    let chars = match code_chars(text) {
        Ok(chars) => chars,
        Err(UnterminatedString { at }) => return Some(Problem::Unterminated { at }),
    };

    let mut open: Vec<(char, usize)> = Vec::new();
    for (index, character) in chars {
        match character {
            '{' | '[' | '(' => open.push((character, index)),
            '}' | ']' | ')' => match open.pop() {
                None => {
                    return Some(Problem::UnmatchedClose {
                        close: character,
                        at: index,
                    });
                }
                Some((opener, open_at)) => {
                    let expected = match opener {
                        '{' => '}',
                        '[' => ']',
                        _ => ')',
                    };
                    if character != expected {
                        return Some(Problem::Mismatch {
                            close: character,
                            close_at: index,
                            open: opener,
                            open_at,
                        });
                    }
                }
            },
            _ => {}
        }
    }
    open.pop()
        .map(|(opener, at)| Problem::Unclosed { open: opener, at })
}

/// The 1-based number of the line that byte offset `index` is on.
pub fn line_number(text: &str, index: usize) -> usize {
    text[..index].matches('\n').count() + 1
}

/// "line 12", or "line 12 (`  organization { name }`)" when `show_text` is set.
fn describe_line(text: &str, index: usize, show_text: bool) -> String {
    let number = line_number(text, index);
    if !show_text {
        return format!("line {number}");
    }
    let line = text.lines().nth(number - 1).unwrap_or_default().trim();
    let shown: String = line.chars().take(60).collect();
    let ellipsis = if line.chars().count() > 60 { "..." } else { "" };
    format!("line {number} (`{shown}{ellipsis}`)")
}

impl Problem {
    /// What is wrong and where, e.g. "has a closing `}` without an opening one on line 4".
    ///
    /// `show_text` quotes the offending line. Leave it off for text that can contain values
    /// filled in from variables, so those values do not end up in an error message.
    pub fn describe(&self, text: &str, show_text: bool) -> String {
        match self {
            Self::Unterminated { at } => format!(
                "has a string that is never closed (it starts on {})",
                describe_line(text, *at, show_text)
            ),
            Self::UnmatchedClose { close, at } => format!(
                "has a closing `{close}` without an opening one on {}",
                describe_line(text, *at, show_text)
            ),
            Self::Mismatch {
                close,
                close_at,
                open,
                open_at,
            } => format!(
                "has a closing `{close}` on {} that does not match the `{open}` opened on {}",
                describe_line(text, *close_at, show_text),
                describe_line(text, *open_at, show_text)
            ),
            Self::Unclosed { open, at } => format!(
                "has an opening `{open}` on {} that is never closed",
                describe_line(text, *at, show_text)
            ),
        }
    }
}

/// Whether a selection like `id name company { id }` selects `id` at its top level (an `id`
/// inside a relation or an argument does not count).
pub fn selects_id(selection: &str) -> bool {
    let Ok(chars) = code_chars(selection) else {
        return false;
    };

    let mut depth = 0i32;
    let mut word = String::new();
    let mut top_level_words = Vec::new();

    for (_, character) in chars {
        if depth == 0 && (character.is_alphanumeric() || character == '_') {
            word.push(character);
            continue;
        }
        if !word.is_empty() {
            top_level_words.push(std::mem::take(&mut word));
        }
        match character {
            '{' | '[' | '(' => depth += 1,
            '}' | ']' | ')' => depth -= 1,
            _ => {}
        }
    }
    if !word.is_empty() {
        top_level_words.push(word);
    }

    top_level_words.iter().any(|word| word == "id")
}

/// The text between the outer curly braces if the whole of `text` (ignoring comments and
/// whitespace) is one `{ ... }` pair, else `None`.
pub fn inside_outer_braces(text: &str) -> Option<&str> {
    let chars = code_chars(text).ok()?;
    let &(first_at, first) = chars
        .iter()
        .find(|(_, character)| !character.is_whitespace())?;
    if first != '{' {
        return None;
    }

    let mut depth = 0i32;
    let mut close_at = None;
    for &(index, character) in chars.iter().filter(|(index, _)| *index >= first_at) {
        match character {
            '{' | '[' | '(' => depth += 1,
            '}' | ']' | ')' => {
                depth -= 1;
                if depth == 0 {
                    close_at = Some(index);
                    break;
                }
            }
            _ => {}
        }
    }
    let close_at = close_at?;

    // Nothing but whitespace and comments may follow the closing brace.
    let nothing_after = chars
        .iter()
        .filter(|(index, _)| *index > close_at)
        .all(|(_, character)| character.is_whitespace());
    nothing_after.then(|| text[first_at + 1..close_at].trim())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn code(text: &str) -> String {
        code_chars(text)
            .unwrap()
            .into_iter()
            .map(|(_, character)| character)
            .collect()
    }

    #[test]
    fn comments_end_at_any_line_break() {
        assert_eq!(code("a # x } y\nb"), "a \nb");
        assert_eq!(code("a # x } y\rb"), "a \rb");
        assert_eq!(code("a # x } y\r\nb"), "a \r\nb");
    }

    #[test]
    fn strings_are_not_code_and_escaped_quotes_do_not_close_them() {
        assert_eq!(code(r#"a "x } \" { y" b"#), "a   b");
        assert_eq!(code(r#"a "" b"#), "a   b");
    }

    #[test]
    fn block_strings_run_to_the_next_triple_quote() {
        assert_eq!(code("a \"\"\"x \" } \"\"\" b"), "a   b");
        assert_eq!(code("a \"\"\"x \\\"\"\" } \"\"\" b"), "a   b");
        // The case a naive lexer reads as an empty string plus an open string.
        assert_eq!(code("\"\"\"a(a\" \"\"\"{{}"), " {{}");
    }

    #[test]
    fn unterminated_strings_are_errors() {
        assert_eq!(code_chars("a \"open"), Err(UnterminatedString { at: 2 }));
        assert_eq!(
            code_chars("a \"open\nnext\""),
            Err(UnterminatedString { at: 2 })
        );
        assert_eq!(code_chars("a \"ends \\"), Err(UnterminatedString { at: 2 }));
        assert_eq!(
            code_chars("a \"\"\"open"),
            Err(UnterminatedString { at: 2 })
        );
    }

    #[test]
    fn a_comment_ended_by_a_bare_carriage_return_does_not_hide_brackets() {
        // A lexer that only ends comments at `\n` sees this as balanced.
        assert_eq!(
            find_problem("a # c\r}"),
            Some(Problem::UnmatchedClose { close: '}', at: 6 })
        );
    }

    #[test]
    fn bracket_problems_are_described_with_their_line() {
        let text = "name\n  company { name\n]";
        let problem = find_problem(text).unwrap();
        assert_eq!(
            problem.describe(text, false),
            "has a closing `]` on line 3 that does not match the `{` opened on line 2"
        );
        let text = "a\n\n}";
        assert_eq!(
            find_problem(text).unwrap().describe(text, true),
            "has a closing `}` without an opening one on line 3 (`}`)"
        );
        assert_eq!(
            find_problem("a { b").unwrap().describe("a { b", false),
            "has an opening `{` on line 1 that is never closed"
        );
    }

    #[test]
    fn selects_id_looks_at_the_top_level_only() {
        assert!(selects_id("id name"));
        assert!(selects_id("name\n id\n company { name }"));
        assert!(!selects_id("name company { id }"));
        assert!(!selects_id("identifier user_id"));
        assert!(!selects_id("# the id of it\nname"));
    }

    #[test]
    fn outer_braces_are_found_only_when_they_wrap_everything() {
        assert_eq!(
            inside_outer_braces("{ id: { eq: 1 } }"),
            Some("id: { eq: 1 }")
        );
        assert_eq!(inside_outer_braces("{ a: 1 } # note"), Some("a: 1"));
        assert_eq!(inside_outer_braces("{ a: 1 } { b: 2 }"), None);
        assert_eq!(inside_outer_braces("id: { eq: 1 }"), None);
    }
}
