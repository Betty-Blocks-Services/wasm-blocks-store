//! Reads the files to zip from the collection's records.

use std::collections::HashSet;

use serde_json::Value;

/// The most files one ZIP may hold. Every file is its own download, so this keeps a single run
/// bounded; not a measured platform limit. The same number as UNZIP's.
pub const MAX_FILES: usize = 1000;

#[derive(Debug, PartialEq)]
pub struct FileToZip {
    /// The name inside the archive, made unique (`a.pdf`, `a_1.pdf`, ...).
    pub entry_name: String,
    pub url: String,
}

/// The last path segment of a URL, without its query string, for a file that has no name.
fn name_from_url(url: &str) -> Option<String> {
    let path = url.split(['?', '#']).next()?;
    let name = path.rsplit('/').next()?;
    (!name.is_empty()).then(|| name.to_string())
}

/// Makes `name` usable as one archive entry: no folders, no empty name.
fn clean_name(name: &str) -> String {
    let name = name.rsplit(['/', '\\']).next().unwrap_or_default().trim();
    if name.is_empty() || name == "." || name == ".." {
        String::from("file")
    } else {
        name.to_string()
    }
}

/// `name`, or `name_1`, `name_2`, ... before the extension, whichever is not taken yet. The same
/// scheme as the ActionsJS ZIP step.
fn unique_name(name: &str, taken: &mut HashSet<String>) -> String {
    let mut candidate = name.to_string();
    let mut count = 1;
    while taken.contains(&candidate) {
        candidate = match name.rfind('.') {
            Some(dot) if dot > 0 => format!("{}_{count}{}", &name[..dot], &name[dot..]),
            _ => format!("{name}_{count}"),
        };
        count += 1;
    }
    taken.insert(candidate.clone());
    candidate
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

/// The records of a Collection option, each as a JSON object in text. The value arrives as JSON:
/// an object with a `data` list (what the ActionsJS step read), or a bare list. A record may be an
/// object or a JSON string holding one.
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

/// Collects the file in `property` of every record. Records without a file (no value, or no URL)
/// are skipped, as the ActionsJS step did.
pub fn files_in_records(records: &[String], property: &str) -> Result<Vec<FileToZip>, String> {
    let mut files = Vec::new();
    let mut taken = HashSet::new();
    let mut property_seen = records.is_empty();
    let mut first_keys = None;
    for record in records {
        let record: Value = serde_json::from_str(record)
            .map_err(|error| format!("a record in the collection is not valid JSON: {error}"))?;
        let Some(fields) = record.as_object() else {
            return Err(format!(
                "a record in the collection is not a JSON object: {record}"
            ));
        };
        if first_keys.is_none() {
            first_keys = Some(fields.keys().cloned().collect::<Vec<_>>().join(", "));
        }
        let Some(value) = fields.get(property) else {
            continue;
        };
        property_seen = true;
        let url = value.get("url").and_then(Value::as_str).unwrap_or_default();
        if url.is_empty() {
            continue;
        }
        let name = value
            .get("name")
            .and_then(Value::as_str)
            .filter(|name| !name.is_empty())
            .map(str::to_string)
            .or_else(|| name_from_url(url))
            .unwrap_or_default();
        if files.len() == MAX_FILES {
            return Err(format!(
                "the collection has more than {MAX_FILES} files, the most this step puts in one ZIP — split the collection, for example with a filter or a limit"
            ));
        }
        files.push(FileToZip {
            entry_name: unique_name(&clean_name(&name), &mut taken),
            url: url.to_string(),
        });
    }
    if !property_seen {
        return Err(format!(
            "the records in the collection have no property \"{property}\" (found: {}) — check that \"Property to read files from\" belongs to the model of the collection",
            first_keys.unwrap_or_default()
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
            record(serde_json::json!({"id": 4, "file": {"url": "https://x/files/b.png?sig=2"}})),
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
            ]
        );
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
    fn folder_parts_are_removed_from_names() {
        assert_eq!(clean_name("../../etc/passwd"), "passwd");
        assert_eq!(clean_name("dir\\file.txt"), "file.txt");
        assert_eq!(clean_name(".."), "file");
        assert_eq!(clean_name(""), "file");
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
