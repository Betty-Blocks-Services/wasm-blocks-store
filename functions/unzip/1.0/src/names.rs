//! File names from untrusted input (names inside a ZIP, file values in records, the name option),
//! made safe to store. The same code is in the `zip` and `unzip` steps.

use std::collections::HashSet;

/// The longest file name kept, in bytes. Longer names are shortened before the extension.
pub const MAX_NAME_BYTES: usize = 200;

/// The longest extension kept when a long name is shortened, in bytes, dot included.
const MAX_EXTENSION_BYTES: usize = 20;

/// The start of `text`, at most `max` bytes, cut at a character boundary.
fn cut(text: &str, max: usize) -> &str {
    let mut end = text.len().min(max);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

/// `name` split into its stem and extension (dot included). Only a dot after the first character
/// starts an extension (`.env` has none), and an extension longer than [`MAX_EXTENSION_BYTES`] is
/// taken as part of the stem.
fn split_extension(name: &str) -> (&str, &str) {
    match name.rfind('.') {
        Some(dot) if dot > 0 && name.len() - dot <= MAX_EXTENSION_BYTES => name.split_at(dot),
        _ => (name, ""),
    }
}

/// `stem` + `suffix` + `extension`, with the stem shortened so the whole is at most `max` bytes.
fn join_within(stem: &str, suffix: &str, extension: &str, max: usize) -> String {
    let room = max.saturating_sub(suffix.len() + extension.len());
    format!("{}{suffix}{extension}", cut(stem, room).trim_end())
}

/// `name` shortened to at most `max` bytes, keeping its extension.
pub fn shorten(name: &str, max: usize) -> String {
    if name.len() <= max {
        return name.to_string();
    }
    let (stem, extension) = split_extension(name);
    join_within(stem, "", extension, max)
}

/// `name` as a single file name: the folders before its last `/` or `\` removed, NUL and other
/// control characters removed, spaces around it trimmed, and at most [`MAX_NAME_BYTES`] long
/// (the extension is kept). `None` when nothing usable is left: empty, `.` or `..`.
pub fn clean_file_name(name: &str) -> Option<String> {
    let name = name.rsplit(['/', '\\']).next().unwrap_or_default();
    let name: String = name.chars().filter(|c| !c.is_control()).collect();
    let name = name.trim();
    if name.is_empty() || name == "." || name == ".." {
        return None;
    }
    Some(shorten(name, MAX_NAME_BYTES))
}

/// `name`, or `name_1`, `name_2`, ... before the extension, whichever is not taken yet, shortened
/// to stay within [`MAX_NAME_BYTES`]. Names are compared without regard to case, because Windows
/// and macOS would write `Report.pdf` and `report.pdf` to the same file. A name that starts with
/// its only dot (`.env`) has no extension, so it becomes `.env_1`.
pub fn unique_name(name: &str, taken: &mut HashSet<String>) -> String {
    let (stem, extension) = split_extension(name);
    let mut candidate = name.to_string();
    let mut count = 1;
    while taken.contains(&candidate.to_lowercase()) {
        candidate = join_within(stem, &format!("_{count}"), extension, MAX_NAME_BYTES);
        count += 1;
    }
    taken.insert(candidate.to_lowercase());
    candidate
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folders_and_control_characters_are_removed() {
        assert_eq!(
            clean_file_name("../../etc/passwd").as_deref(),
            Some("passwd")
        );
        assert_eq!(
            clean_file_name("dir\\file.txt").as_deref(),
            Some("file.txt")
        );
        assert_eq!(clean_file_name("x\0y.txt").as_deref(), Some("xy.txt"));
        assert_eq!(clean_file_name("a\r\nb\t.pdf").as_deref(), Some("ab.pdf"));
        assert_eq!(
            clean_file_name("  report.pdf ").as_deref(),
            Some("report.pdf")
        );
    }

    #[test]
    fn nothing_usable_is_none() {
        for name in ["", " ", ".", "..", "d/..", "d/", "\0", "a\\.."] {
            assert_eq!(clean_file_name(name), None, "for {name:?}");
        }
    }

    #[test]
    fn a_long_name_is_shortened_before_the_extension() {
        let name = clean_file_name(&format!("{}.pdf", "é".repeat(30_000))).unwrap();
        assert!(name.len() <= MAX_NAME_BYTES, "{} bytes", name.len());
        assert!(name.ends_with("é.pdf"), "{name}");
        let name = clean_file_name(&"a".repeat(65_000)).unwrap();
        assert_eq!(name.len(), MAX_NAME_BYTES);
        let name = clean_file_name(&format!("x.{}", "b".repeat(300))).unwrap();
        assert_eq!(name.len(), MAX_NAME_BYTES);
    }

    #[test]
    fn duplicate_names_get_a_number_before_the_extension() {
        let mut taken = HashSet::new();
        assert_eq!(unique_name("a.pdf", &mut taken), "a.pdf");
        assert_eq!(unique_name("a.pdf", &mut taken), "a_1.pdf");
        assert_eq!(unique_name("a.pdf", &mut taken), "a_2.pdf");
        assert_eq!(unique_name("README", &mut taken), "README");
        assert_eq!(unique_name("README", &mut taken), "README_1");
        assert_eq!(unique_name(".env", &mut taken), ".env");
        assert_eq!(unique_name(".env", &mut taken), ".env_1");
    }

    #[test]
    fn names_that_differ_only_in_case_are_duplicates() {
        let mut taken = HashSet::new();
        assert_eq!(unique_name("Report.pdf", &mut taken), "Report.pdf");
        assert_eq!(unique_name("report.pdf", &mut taken), "report_1.pdf");
        assert_eq!(unique_name("REPORT_1.PDF", &mut taken), "REPORT_1_1.PDF");
    }

    #[test]
    fn a_numbered_name_stays_within_the_limit() {
        let long = clean_file_name(&format!("{}.pdf", "a".repeat(300))).unwrap();
        assert_eq!(long.len(), MAX_NAME_BYTES);
        let mut taken = HashSet::new();
        unique_name(&long, &mut taken);
        let second = unique_name(&long, &mut taken);
        assert_eq!(second.len(), MAX_NAME_BYTES);
        assert!(second.ends_with("a_1.pdf"), "{second}");
        assert_eq!(shorten("abcdef.zip", 8), "abcd.zip");
    }
}
