//! Reads the files to zip from the collection's records.

use std::collections::HashSet;

use serde_json::Value;

use crate::names::{clean_file_name, unique_name};

/// The most files one ZIP may hold. Every file is its own download, so this keeps a single run
/// bounded; not a measured platform limit. The same number as UNZIP's.
pub const MAX_FILES: usize = 1000;

#[derive(Debug, PartialEq)]
pub struct FileToZip {
    /// The name inside the archive, made safe and unique (`a.pdf`, `a_1.pdf`, ...).
    pub entry_name: String,
    pub url: String,
}

/// `text` with `%XX` escapes decoded, or `text` itself when the result is not UTF-8.
fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        let hex = bytes
            .get(index + 1..index + 3)
            .and_then(|hex| std::str::from_utf8(hex).ok())
            .and_then(|hex| u8::from_str_radix(hex, 16).ok());
        match (bytes[index], hex) {
            (b'%', Some(byte)) => {
                decoded.push(byte);
                index += 3;
            }
            (byte, _) => {
                decoded.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8(decoded).unwrap_or_else(|_| text.to_string())
}

/// The last path segment of a URL, without its query string and percent-decoded, for a file that
/// has no name.
fn name_from_url(url: &str) -> Option<String> {
    let path = url.split(['?', '#']).next()?;
    let name = path.rsplit('/').next()?;
    (!name.is_empty()).then(|| percent_decode(name))
}

/// The start of `text`, for an error message that shows what arrived.
fn preview(text: &str) -> String {
    let mut end = text.len().min(200);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    if end < text.len() {
        format!("{}...", &text[..end])
    } else {
        text.to_string()
    }
}

/// The records of the collection, each as a JSON object in text. The option is a Value option
/// limited to collections, so the value arrives as JSON: an object with a `data` list (what the
/// ActionsJS step read), or a bare list. A record may be an object or a JSON string holding one.
pub fn records_in_collection(collection: &str) -> Result<Vec<String>, String> {
    let value: Value = serde_json::from_str(collection).map_err(|error| {
        format!(
            "\"Collection to read files from\" is not valid JSON ({error}); it starts with: {}",
            preview(collection)
        )
    })?;
    let items = match value {
        Value::Array(items) => items,
        Value::Object(mut fields) => match fields.remove("data") {
            Some(Value::Array(items)) => items,
            Some(Value::Null) | None => {
                return Err(format!(
                    "the collection arrived without its records — check that \"Collection to read files from\" is set to a collection. It arrived as: {}",
                    preview(collection)
                ));
            }
            Some(other) => {
                return Err(format!(
                    "the collection's \"data\" is not a list; it arrived as: {}",
                    preview(&other.to_string())
                ));
            }
        },
        Value::Null => {
            return Err(String::from(
                "\"Collection to read files from\" is empty — select a collection",
            ));
        }
        other => {
            return Err(format!(
                "\"Collection to read files from\" is not a collection; it arrived as: {}",
                preview(&other.to_string())
            ));
        }
    };
    Ok(items
        .into_iter()
        .map(|item| match item {
            Value::String(text) => text,
            other => other.to_string(),
        })
        .collect())
}

/// True for a value that means "no file": `null`, `{}`, or an object whose `url` and `name` are
/// both missing, `null` or empty.
fn is_no_file(value: &Value) -> bool {
    let blank = |field: Option<&Value>| match field {
        None | Some(Value::Null) => true,
        Some(Value::String(text)) => text.is_empty(),
        Some(_) => false,
    };
    match value {
        Value::Null => true,
        Value::Object(fields) => blank(fields.get("url")) && blank(fields.get("name")),
        _ => false,
    }
}

/// Collects the file in `property` of every record. Records without a file (see [`is_no_file`])
/// are skipped. Fails when the property is missing from every record, when a value is not a file
/// (an object with a `url`), and when the collection has records but none of them has a file: all
/// three mean the step reads the wrong property. An empty collection gives no files.
pub fn files_in_records(records: &[String], property: &str) -> Result<Vec<FileToZip>, String> {
    let mut files = Vec::new();
    let mut taken = HashSet::new();
    let mut property_seen = records.is_empty();
    let mut first_keys = None;
    for (index, record) in records.iter().enumerate() {
        let number = index + 1;
        let record: Value = serde_json::from_str(record).map_err(|error| {
            format!(
                "record {number} of the collection is not valid JSON ({error}); it starts with: {}",
                preview(record)
            )
        })?;
        let Some(fields) = record.as_object() else {
            return Err(format!(
                "record {number} of the collection is not a JSON object; it starts with: {}",
                preview(&record.to_string())
            ));
        };
        if first_keys.is_none() {
            first_keys = Some(fields.keys().cloned().collect::<Vec<_>>().join(", "));
        }
        let Some(value) = fields.get(property) else {
            continue;
        };
        property_seen = true;
        if is_no_file(value) {
            continue;
        }
        let url = value
            .get("url")
            .and_then(Value::as_str)
            .filter(|url| !url.is_empty())
            .ok_or_else(|| {
                format!(
                    "the value of \"{property}\" in record {number} is not a file with a URL; it is: {} — check that \"Property to read files from\" is a file or image property",
                    preview(&value.to_string())
                )
            })?;
        if files.len() == MAX_FILES {
            return Err(format!(
                "the collection has more than {MAX_FILES} files, the most this step puts in one ZIP — split the collection, for example with a filter or a limit"
            ));
        }
        let name = value
            .get("name")
            .and_then(Value::as_str)
            .and_then(clean_file_name)
            .or_else(|| name_from_url(url).as_deref().and_then(clean_file_name))
            .unwrap_or_else(|| String::from("file"));
        files.push(FileToZip {
            entry_name: unique_name(&name, &mut taken),
            url: url.to_string(),
        });
    }
    if !property_seen {
        return Err(format!(
            "the records in the collection have no property \"{property}\" (found: {}) — check that \"Property to read files from\" belongs to the model of the collection",
            first_keys.unwrap_or_default()
        ));
    }
    if files.is_empty() && !records.is_empty() {
        return Err(format!(
            "0 of {} records had a file in \"{property}\", so there is nothing to zip — check that \"Property to read files from\" is the property that holds the files, and that the records have files",
            records.len()
        ));
    }
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(value: Value) -> String {
        value.to_string()
    }

    #[test]
    fn reads_name_and_url_and_skips_records_without_a_file() {
        let records = [
            record(
                serde_json::json!({"id": 1, "file": {"name": "a.pdf", "url": "https://x/a.pdf?sig=1"}}),
            ),
            record(serde_json::json!({"id": 2, "file": null})),
            record(serde_json::json!({"id": 3, "file": {}})),
            record(serde_json::json!({"id": 4, "file": {"name": null, "url": null}})),
            record(serde_json::json!({"id": 5, "file": {"url": "https://x/files/b.png?sig=2"}})),
            record(serde_json::json!({"id": 6, "file": {"name": "../..", "url": "https://x/"}})),
        ];
        let files = files_in_records(&records, "file").unwrap();
        assert_eq!(
            files,
            [
                FileToZip {
                    entry_name: "a.pdf".into(),
                    url: "https://x/a.pdf?sig=1".into()
                },
                FileToZip {
                    entry_name: "b.png".into(),
                    url: "https://x/files/b.png?sig=2".into()
                },
                FileToZip {
                    entry_name: "file".into(),
                    url: "https://x/".into()
                },
            ]
        );
    }

    #[test]
    fn a_name_from_the_url_is_percent_decoded() {
        assert_eq!(
            name_from_url("https://x/f/my%20report%28v2%29.pdf?sig=1").as_deref(),
            Some("my report(v2).pdf")
        );
        assert_eq!(
            name_from_url("https://x/100%25.txt").as_deref(),
            Some("100%.txt")
        );
        assert_eq!(
            name_from_url("https://x/50%.txt").as_deref(),
            Some("50%.txt")
        );
        assert_eq!(
            name_from_url("https://x/%FF.bin").as_deref(),
            Some("%FF.bin")
        );
        let name = percent_decode("a%2Fb");
        assert_eq!(clean_file_name(&name).as_deref(), Some("b"));
    }

    #[test]
    fn names_are_cleaned_and_made_unique() {
        let records = [
            record(serde_json::json!({"file": {"name": "dir/a.pdf", "url": "https://x/1"}})),
            record(serde_json::json!({"file": {"name": "a\u{0}.pdf", "url": "https://x/2"}})),
        ];
        let names: Vec<String> = files_in_records(&records, "file")
            .unwrap()
            .into_iter()
            .map(|file| file.entry_name)
            .collect();
        assert_eq!(names, ["a.pdf", "a_1.pdf"]);
    }

    #[test]
    fn a_wrong_property_is_a_clear_error() {
        let records = [record(
            serde_json::json!({"id": 1, "document": {"name": "a", "url": "https://x/a"}}),
        )];
        let error = files_in_records(&records, "file").unwrap_err();
        assert!(error.contains("no property \"file\""), "error was: {error}");
        assert!(error.contains("document"), "error was: {error}");
    }

    #[test]
    fn records_without_any_file_are_an_error() {
        let records = [
            record(serde_json::json!({"id": 1, "file": null})),
            record(serde_json::json!({"id": 2, "file": {}})),
        ];
        let error = files_in_records(&records, "file").unwrap_err();
        assert!(
            error.contains("0 of 2 records had a file in \"file\""),
            "error was: {error}"
        );
    }

    #[test]
    fn a_value_that_is_not_a_file_is_an_error() {
        for value in [
            serde_json::json!("https://x/a.pdf"),
            serde_json::json!(5),
            serde_json::json!({"name": "a.pdf"}),
            serde_json::json!({"name": "a.pdf", "url": 5}),
        ] {
            let records = [
                record(serde_json::json!({"file": {"name": "ok", "url": "https://x/ok"}})),
                record(serde_json::json!({ "file": value })),
            ];
            let error = files_in_records(&records, "file").unwrap_err();
            assert!(
                error.contains("in record 2 is not a file with a URL"),
                "for {value}: {error}"
            );
        }
    }

    #[test]
    fn a_bad_record_is_named_and_shortened() {
        let records = [
            record(serde_json::json!({"file": null})),
            serde_json::json!("x".repeat(5_000_000)).to_string(),
        ];
        let error = files_in_records(&records, "file").unwrap_err();
        assert!(
            error.starts_with("record 2 of the collection is not a JSON object"),
            "{error}"
        );
        assert!(error.len() < 400, "error is {} bytes", error.len());
        let error = files_in_records(&[String::from("{oops")], "file").unwrap_err();
        assert!(
            error.starts_with("record 1 of the collection is not valid JSON"),
            "{error}"
        );
    }

    #[test]
    fn reads_records_from_every_collection_shape() {
        let record = r#"{"id":1}"#;
        for collection in [
            r#"{"data":[{"id":1}]}"#,
            r#"{"model":{"name":"Document"},"data":[{"id":1}]}"#,
            r#"{"data":["{\"id\":1}"]}"#,
            r#"[{"id":1}]"#,
        ] {
            assert_eq!(
                records_in_collection(collection).unwrap(),
                [record],
                "for {collection}"
            );
        }
    }

    #[test]
    fn an_unexpected_collection_shape_shows_what_arrived() {
        for (collection, expected) in [
            (r#"{"model":{"name":"Document"}}"#, "without its records"),
            (r#"{"data":null}"#, "without its records"),
            (r#"{"data":5}"#, "not a list"),
            ("null", "is empty"),
            ("42", "not a collection"),
            ("not json", "not valid JSON"),
        ] {
            let error = records_in_collection(collection).unwrap_err();
            assert!(error.contains(expected), "for {collection}: {error}");
        }
        let long = format!(r#"{{"other":"{}"}}"#, "é".repeat(300));
        let error = records_in_collection(&long).unwrap_err();
        assert!(error.ends_with("..."), "error was: {error}");
    }

    #[test]
    fn the_file_limit_counts_only_records_with_a_file() {
        let with_file = record(serde_json::json!({"file": {"url": "https://x/a"}}));
        let without_file = record(serde_json::json!({"file": null}));
        let mut records = vec![without_file; 5];
        records.extend(std::iter::repeat_n(with_file.clone(), MAX_FILES));
        assert_eq!(files_in_records(&records, "file").unwrap().len(), MAX_FILES);
        records.push(with_file);
        let error = files_in_records(&records, "file").unwrap_err();
        assert!(error.contains("more than 1000 files"), "error was: {error}");
    }

    #[test]
    fn an_empty_collection_gives_no_files() {
        assert!(files_in_records(&[], "file").unwrap().is_empty());
    }
}
