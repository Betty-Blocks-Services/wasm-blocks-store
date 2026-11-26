//! The Map options: Import mapping and Update mapping (column -> property), Default mapping
//! (property -> value) and the column side of the Format mapping.
//!
//! `parse_map`, `json_kind`, `snake_to_camel` and `is_name` also exist in the export-records step
//! (PR #21): once both steps are merged, keep the copies the same.

use std::fmt;

use serde::de::{Deserializer, MapAccess, SeqAccess, Visitor};
use serde_json::Value;

/// The entries of a Map option in the order they were entered.
///
/// A Map arrives as a JSON object string. `serde_json::Value` sorts object keys alphabetically
/// (no `preserve_order` feature in this workspace), which would reorder the columns, so the
/// object is read entry by entry here. A list of `{ "key": ..., "value": ... }` (the shape the
/// ActionsJS step received) is accepted too. Empty text and `null` mean no entries.
pub fn parse_map(text: &str, label: &str) -> Result<Vec<(String, Value)>, String> {
    if text.trim().is_empty() {
        return Ok(Vec::new());
    }
    let mut deserializer = serde_json::Deserializer::from_str(text);
    let entries = deserializer
        .deserialize_any(EntriesVisitor)
        .and_then(|entries| deserializer.end().map(|()| entries))
        .map_err(|error| format!("{label} is not valid JSON ({error})"))?;
    entries.ok_or_else(|| {
        // The kind of JSON, not the text itself: the option may hold values that should not end
        // up in the log.
        let kind = serde_json::from_str::<Value>(text)
            .map(|value| json_kind(&value))
            .unwrap_or("something else");
        format!(
            "{label} must be a map of keys to values (KEY = VALUE rows in the option), not {kind}"
        )
    })
}

/// What kind of JSON a value is, for an error message that should not repeat the value.
pub fn json_kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "empty",
        Value::Bool(_) => "a yes/no value",
        Value::Number(_) => "a number",
        Value::String(_) => "text",
        Value::Array(_) => "a list",
        Value::Object(_) => "a map",
    }
}

struct EntriesVisitor;

impl<'de> Visitor<'de> for EntriesVisitor {
    /// `None` when the JSON is neither an object, a list of key/value pairs nor `null`.
    type Value = Option<Vec<(String, Value)>>;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("a map")
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E> {
        Ok(Some(Vec::new()))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        let mut entries = Vec::new();
        while let Some((key, value)) = map.next_entry::<String, Value>()? {
            entries.push((key, value));
        }
        Ok(Some(entries))
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
        let mut entries = Vec::new();
        let mut valid = true;
        while let Some(item) = seq.next_element::<Value>()? {
            match item.get("key").and_then(Value::as_str) {
                Some(key) => entries.push((
                    key.to_string(),
                    item.get("value").cloned().unwrap_or(Value::Null),
                )),
                None => valid = false,
            }
        }
        Ok(valid.then_some(entries))
    }

    fn visit_bool<E>(self, _: bool) -> Result<Self::Value, E> {
        Ok(None)
    }
    fn visit_i64<E>(self, _: i64) -> Result<Self::Value, E> {
        Ok(None)
    }
    fn visit_u64<E>(self, _: u64) -> Result<Self::Value, E> {
        Ok(None)
    }
    fn visit_f64<E>(self, _: f64) -> Result<Self::Value, E> {
        Ok(None)
    }
    fn visit_str<E>(self, _: &str) -> Result<Self::Value, E> {
        Ok(None)
    }
}

/// The ActionsJS step's `snakeToCamel`: every run of characters that are not letters or digits
/// is dropped and the character after it is upper-cased, except that `_` before a digit stays
/// (`address_1` stays `address_1`). A name already in camelCase is returned as it is.
pub fn snake_to_camel(name: &str) -> String {
    // Step 1 of the original: mark every `_<digit>` with a `!` in front.
    let chars: Vec<char> = name.chars().collect();
    let mut marked = Vec::with_capacity(chars.len());
    let mut index = 0;
    while index < chars.len() {
        if chars[index] == '_' && chars.get(index + 1).is_some_and(char::is_ascii_digit) {
            marked.extend(['!', '_', chars[index + 1]]);
            index += 2;
        } else {
            marked.push(chars[index]);
            index += 1;
        }
    }

    // Step 2: `/[^a-zA-Z0-9!]+(.)/g` -> the captured character upper-cased.
    let mut out = String::new();
    let mut index = 0;
    while index < marked.len() {
        let c = marked[index];
        if c.is_ascii_alphanumeric() || c == '!' {
            out.push(c);
            index += 1;
            continue;
        }
        let mut end = index;
        while end < marked.len() && !(marked[end].is_ascii_alphanumeric() || marked[end] == '!') {
            end += 1;
        }
        // The `(.)` needs a character after the run: at least one. A run at the very end stays.
        if end == marked.len() {
            if end - index >= 2 {
                // `.` can take the last non-alphanumeric of the run itself.
                out.extend(&marked[index..end - 2]);
                out.extend(marked[end - 1].to_uppercase());
            } else {
                out.push(c);
            }
            index = end;
        } else {
            out.extend(marked[end].to_uppercase());
            index = end + 1;
        }
    }

    // Step 3: the markers become `_`.
    out.replace('!', "_")
}

/// A GraphQL field name: letters, digits and `_`, not starting with a digit.
pub fn is_name(text: &str) -> bool {
    let mut characters = text.chars();
    characters
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic() || first == '_')
        && characters.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// A column name as written in a mapping key, without the `*` that marks a required column.
pub fn column_name(key: &str) -> &str {
    let key = key.trim();
    key.strip_suffix('*').map(str::trim_end).unwrap_or(key)
}

/// Where the value of a column goes.
#[derive(Debug, Clone, PartialEq)]
pub enum Target {
    /// A property of the import model, by its camelCase name.
    Property(String),
    /// A belongs-to relation, set to the id of the related record whose `property` has the value
    /// of the column. `relation.property` in the mapping: the relation is assumed to have the name
    /// of the related model (`company.name` looks up `Company` records), as in the ActionsJS step.
    Relation {
        /// The relation field on the import model (camelCase).
        field: String,
        /// The related model (`Company`).
        model: String,
        /// The property of the related model to match on (camelCase).
        property: String,
    },
}

/// One row of the Import mapping or Update mapping.
#[derive(Debug, Clone, PartialEq)]
pub struct MappedColumn {
    /// The column name in the file, without a trailing `*`.
    pub column: String,
    /// The key ended in `*`: the column must be in the file when Validate required columns is on.
    pub required: bool,
    pub target: Target,
}

/// The rows of the Import mapping or Update mapping. `label` names the option in errors.
pub fn mapped_columns(
    entries: &[(String, Value)],
    label: &str,
) -> Result<Vec<MappedColumn>, String> {
    entries
        .iter()
        .filter(|(key, value)| !(key.trim().is_empty() && is_blank(value)))
        .map(|(key, value)| {
            let column = column_name(key);
            if column.is_empty() {
                return Err(format!(
                    "{label} has a row without a column name: fill in the column name from the file as the key"
                ));
            }
            let text = match value {
                Value::String(text) => text.trim(),
                _ => "",
            };
            if text.is_empty() {
                return Err(format!(
                    "The column \"{column}\" in {label} has no property: fill in its database name, e.g. first_name"
                ));
            }
            let invalid = || {
                format!(
                    "The property \"{text}\" of the column \"{column}\" in {label} is not valid: use a database name like first_name, or model.property for a belongs-to relation, e.g. company.name"
                )
            };
            // Checked before snake_to_camel, which drops other characters and would turn
            // `name } x` into the valid-looking `nameX`.
            let parts: Vec<&str> = text.split('.').map(str::trim).collect();
            if !parts.iter().all(|part| is_name(part)) {
                return Err(invalid());
            }
            let target = match parts.as_slice() {
                [property] => Target::Property(snake_to_camel(property)),
                [relation, property] => {
                    let field = snake_to_camel(relation);
                    Target::Relation {
                        model: capitalize(&field),
                        field,
                        property: snake_to_camel(property),
                    }
                }
                _ => return Err(invalid()),
            };
            Ok(MappedColumn {
                column: column.to_string(),
                required: key.trim().ends_with('*'),
                target,
            })
        })
        .collect()
}

/// The Default mapping: camelCase property name and the value to set on every record.
pub fn default_values(entries: &[(String, Value)]) -> Result<Vec<(String, Value)>, String> {
    entries
        .iter()
        .filter(|(key, value)| !(key.trim().is_empty() && is_blank(value)))
        .map(|(key, value)| {
            let key = key.trim();
            if !is_name(key) {
                return Err(format!(
                    "The key \"{key}\" in the Default mapping is not valid: use the database name of a property or relation, e.g. status"
                ));
            }
            Ok((snake_to_camel(key), value.clone()))
        })
        .collect()
}

fn is_blank(value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::String(text) => text.trim().is_empty(),
        _ => false,
    }
}

fn capitalize(name: &str) -> String {
    let mut characters = name.chars();
    match characters.next() {
        Some(first) => first.to_uppercase().chain(characters).collect(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn columns(map: &str) -> Result<Vec<MappedColumn>, String> {
        mapped_columns(&parse_map(map, "M").unwrap(), "the Import mapping")
    }

    #[test]
    fn a_map_keeps_the_order_it_was_entered_in() {
        let entries = parse_map(r#"{"Zeta": "z", "Alpha": "a", "Mid": "m"}"#, "M").unwrap();
        let keys: Vec<&str> = entries.iter().map(|(key, _)| key.as_str()).collect();
        assert_eq!(keys, ["Zeta", "Alpha", "Mid"]);
    }

    #[test]
    fn a_list_of_key_value_pairs_and_null_are_accepted() {
        assert_eq!(
            parse_map(r#"[{"key": "B", "value": "b"}]"#, "M").unwrap(),
            vec![("B".into(), json!("b"))]
        );
        assert!(parse_map("null", "M").unwrap().is_empty());
        assert!(parse_map("", "M").unwrap().is_empty());
        assert!(parse_map("[1]", "M").is_err());
    }

    #[test]
    fn snake_to_camel_matches_the_actionsjs_step() {
        assert_eq!(snake_to_camel("first_name"), "firstName");
        assert_eq!(snake_to_camel("address_1"), "address_1");
        assert_eq!(snake_to_camel("firstName"), "firstName");
    }

    #[test]
    fn properties_and_relations_are_read() {
        let columns = columns(
            r#"{"First name*": "first_name", "Company": "company.legal_name", "Kvk": "chamber_of_commerce.number"}"#,
        )
        .unwrap();
        assert_eq!(
            columns[0],
            MappedColumn {
                column: "First name".into(),
                required: true,
                target: Target::Property("firstName".into()),
            }
        );
        assert_eq!(
            columns[1].target,
            Target::Relation {
                field: "company".into(),
                model: "Company".into(),
                property: "legalName".into(),
            }
        );
        assert!(!columns[1].required);
        assert_eq!(
            columns[2].target,
            Target::Relation {
                field: "chamberOfCommerce".into(),
                model: "ChamberOfCommerce".into(),
                property: "number".into(),
            }
        );
    }

    #[test]
    fn invalid_rows_are_errors_and_blank_rows_are_skipped() {
        for map in [
            r#"{"A": ""}"#,
            r#"{"A": 3}"#,
            r#"{"A": "name } x { id"}"#,
            r#"{"A": "a.b.c"}"#,
            r#"{"A": "company..name"}"#,
            r#"{"*": "name"}"#,
            r#"{"": "name"}"#,
        ] {
            assert!(columns(map).is_err(), "{map}");
        }
        assert!(columns(r#"{"": ""}"#).unwrap().is_empty());
    }

    #[test]
    fn column_names_lose_the_required_marker() {
        assert_eq!(column_name(" Name* "), "Name");
        assert_eq!(column_name("Name *"), "Name");
        assert_eq!(column_name("Name"), "Name");
    }

    #[test]
    fn default_values_are_camel_cased_and_keep_their_json_type() {
        let entries =
            parse_map(r#"{"is_active": true, "status": "new", "company": 4}"#, "M").unwrap();
        assert_eq!(
            default_values(&entries).unwrap(),
            vec![
                ("isActive".into(), json!(true)),
                ("status".into(), json!("new")),
                ("company".into(), json!(4)),
            ]
        );
        let entries = parse_map(r#"{"a b": 1}"#, "M").unwrap();
        assert!(default_values(&entries).is_err());
    }
}
