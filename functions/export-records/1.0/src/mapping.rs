//! The Map options: Property mapping (column name -> property path) and Format mapping (column
//! name -> format), and turning the property paths into a Data API selection.

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

/// One export column: its header and the camelCase path to its value.
#[derive(Debug, Clone, PartialEq)]
pub struct Column {
    pub header: String,
    pub path: Vec<String>,
}

/// The columns of the Property mapping, in order.
pub fn columns(mapping: &[(String, Value)]) -> Result<Vec<Column>, String> {
    if mapping.is_empty() {
        return Err(String::from(
            "The Property mapping is empty: add a column name (key) and a property (value), e.g. Name -> name",
        ));
    }
    mapping
        .iter()
        .map(|(header, value)| {
            let text = match value {
                Value::String(text) => text.trim(),
                _ => "",
            };
            if text.is_empty() {
                return Err(format!(
                    "The column \"{header}\" in the Property mapping has no property: fill in its database name, e.g. created_at"
                ));
            }
            // Checked before snake_to_camel, which drops other characters and would turn
            // `name } x` into the valid-looking `nameX`.
            let parts: Vec<&str> = text.split('.').map(str::trim).collect();
            let path: Vec<String> = parts.iter().map(|part| snake_to_camel(part)).collect();
            if !parts.iter().all(|part| is_name(part)) || !path.iter().all(|part| is_name(part)) {
                return Err(format!(
                    "The property \"{text}\" of the column \"{header}\" is not valid: use a database name like created_at, or relation.property for a relation"
                ));
            }
            Ok(Column {
                header: header.clone(),
                path,
            })
        })
        .collect()
}

/// The selection for the columns' paths, merged into a tree: `id name company { name }`.
pub fn selection(columns: &[Column]) -> String {
    #[derive(Default)]
    struct Node {
        children: Vec<(String, Node)>,
    }
    fn insert(node: &mut Node, path: &[String]) {
        let Some((first, rest)) = path.split_first() else {
            return;
        };
        let index = match node.children.iter().position(|(name, _)| name == first) {
            Some(index) => index,
            None => {
                node.children.push((first.clone(), Node::default()));
                node.children.len() - 1
            }
        };
        insert(&mut node.children[index].1, rest);
    }
    fn render(node: &Node) -> String {
        node.children
            .iter()
            .map(|(name, child)| {
                if child.children.is_empty() {
                    name.clone()
                } else {
                    format!("{name} {{ {} }}", render(child))
                }
            })
            .collect::<Vec<_>>()
            .join(" ")
    }

    let mut root = Node::default();
    for column in columns {
        insert(&mut root, &column.path);
    }
    render(&root)
}

/// The values at `path` in a record, looking through to-many relations: a list on the way (a
/// has-many or has-and-belongs-to-many relation) contributes the value of every item. The flag is
/// `true` when a list was met, so the caller joins the values into one cell. `null` values are
/// left out.
pub fn lookup_all<'a>(record: &'a Value, path: &[String]) -> (Vec<&'a Value>, bool) {
    fn walk<'a>(value: &'a Value, path: &[String], out: &mut Vec<&'a Value>, multi: &mut bool) {
        if let Value::Array(items) = value {
            *multi = true;
            for item in items {
                walk(item, path, out, multi);
            }
            return;
        }
        match path.split_first() {
            None if !value.is_null() => out.push(value),
            None => {}
            Some((key, rest)) => {
                if let Some(next) = value.as_object().and_then(|object| object.get(key)) {
                    walk(next, rest, out, multi);
                }
            }
        }
    }
    let mut out = Vec::new();
    let mut multi = false;
    walk(record, path, &mut out, &mut multi);
    (out, multi)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_map_keeps_the_order_it_was_entered_in() {
        let entries = parse_map(r#"{"Zeta": "z", "Alpha": "a", "Mid": "m"}"#, "M").unwrap();
        let keys: Vec<&str> = entries.iter().map(|(key, _)| key.as_str()).collect();
        assert_eq!(keys, ["Zeta", "Alpha", "Mid"]);
    }

    #[test]
    fn a_list_of_key_value_pairs_is_accepted() {
        let entries = parse_map(
            r#"[{"key": "B", "value": "b"}, {"key": "A", "value": 1}]"#,
            "M",
        )
        .unwrap();
        assert_eq!(
            entries,
            vec![("B".into(), json!("b")), ("A".into(), json!(1))]
        );
    }

    #[test]
    fn empty_and_null_maps_have_no_entries() {
        assert!(parse_map("", "M").unwrap().is_empty());
        assert!(parse_map("null", "M").unwrap().is_empty());
        assert!(parse_map("{}", "M").unwrap().is_empty());
    }

    #[test]
    fn other_json_is_an_error() {
        assert!(parse_map("\"text\"", "M").is_err());
        assert!(parse_map("[1, 2]", "M").is_err());
        assert!(parse_map("{", "M").is_err());
        assert!(parse_map("{} x", "M").is_err());
    }

    #[test]
    fn snake_to_camel_matches_the_actionsjs_step() {
        assert_eq!(snake_to_camel("first_name"), "firstName");
        assert_eq!(snake_to_camel("created_at"), "createdAt");
        assert_eq!(snake_to_camel("firstName"), "firstName");
        assert_eq!(snake_to_camel("id"), "id");
        assert_eq!(snake_to_camel("address_1"), "address_1");
        assert_eq!(snake_to_camel("line_2_text"), "line_2Text");
        assert_eq!(snake_to_camel("a__b"), "aB");
    }

    #[test]
    fn columns_are_camel_cased_paths() {
        let mapping = parse_map(
            r#"{"Name": "first_name", "Company": "company.legal_name"}"#,
            "M",
        )
        .unwrap();
        let columns = columns(&mapping).unwrap();
        assert_eq!(columns[0].path, ["firstName"]);
        assert_eq!(columns[1].path, ["company", "legalName"]);
    }

    #[test]
    fn a_column_without_a_valid_property_is_an_error() {
        for mapping in [
            r#"{"A": ""}"#,
            r#"{"A": 3}"#,
            r#"{"A": "name } other { id"}"#,
            r#"{"A": "company..name"}"#,
        ] {
            let mapping = parse_map(mapping, "M").unwrap();
            assert!(columns(&mapping).is_err(), "{mapping:?}");
        }
        assert!(columns(&[]).is_err());
    }

    #[test]
    fn the_selection_merges_shared_relations() {
        let mapping = parse_map(
            r#"{"A": "id", "B": "company.name", "C": "company.city.name", "D": "company.id", "E": "id"}"#,
            "M",
        )
        .unwrap();
        assert_eq!(
            selection(&columns(&mapping).unwrap()),
            "id company { name city { name } id }"
        );
    }

    #[test]
    fn lookup_all_looks_through_to_many_relations() {
        let record = json!({
            "name": "Ann",
            "departments": [
                {"description": "Sales, EMEA", "head": {"name": "Cy"}},
                {"description": null, "head": null},
                {"description": "Support", "head": {"name": "Di"}}
            ],
            "tags": ["a", "b"],
            "none": [],
            "f": false
        });
        let path = |p: &str| p.split('.').map(String::from).collect::<Vec<_>>();
        let (values, multi) = lookup_all(&record, &path("departments.description"));
        assert_eq!(values, [&json!("Sales, EMEA"), &json!("Support")]);
        assert!(multi);
        let (values, _) = lookup_all(&record, &path("departments.head.name"));
        assert_eq!(values, [&json!("Cy"), &json!("Di")]);
        let (values, multi) = lookup_all(&record, &path("tags"));
        assert_eq!(values, [&json!("a"), &json!("b")]);
        assert!(multi);
        assert_eq!(lookup_all(&record, &path("none")), (vec![], true));
        assert_eq!(
            lookup_all(&record, &path("name")),
            (vec![&json!("Ann")], false)
        );
        assert_eq!(
            lookup_all(&record, &path("f")),
            (vec![&json!(false)], false)
        );
        assert_eq!(lookup_all(&record, &path("x.y")), (vec![], false));
    }
}
