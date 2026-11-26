//! Turns the rows of the file into records and writes them to the Data API: new records with
//! `createMany<Model>`, existing ones (deduplication) with `upsertMany<Model>` and their `id`.
//!
//! Every value is converted before anything is written, so a value that cannot be converted fails
//! the step without importing anything. The writes themselves are not one transaction: when a
//! write fails, the earlier ones stay.

use std::collections::HashMap;

use serde_json::{Map, Value, json};

use crate::format::{self, Format, MatchKind};
use crate::mapping::{self, MappedColumn, Target};
use crate::read::Table;

/// The most records the Data API returns per request.
pub const PAGE_SIZE: usize = 200;
/// The most records sent in one createMany or upsertMany request.
pub const WRITE_CHUNK: usize = 500;

/// Sends one GraphQL request (query, variables as JSON) and returns the response text.
pub type Request<'a> = &'a mut dyn FnMut(&str, &str) -> Result<String, String>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UniqueType {
    Text,
    Number,
    Decimal,
    Id,
}

impl UniqueType {
    fn match_kind(self) -> MatchKind {
        match self {
            Self::Text => MatchKind::Text,
            Self::Number | Self::Id => MatchKind::Number,
            Self::Decimal => MatchKind::Decimal,
        }
    }
}

#[derive(Debug)]
pub struct Params {
    pub model: String,
    pub import: Vec<MappedColumn>,
    pub update: Vec<MappedColumn>,
    /// Column name -> format.
    pub formats: Vec<(String, Format)>,
    /// camelCase property -> value, set on every record.
    pub defaults: Vec<(String, Value)>,
    /// The column name (as in the Import mapping) and type that identify an existing record.
    pub dedup: Option<(String, UniqueType)>,
    pub validate_required: bool,
}

/// The options as they arrive, before they are checked.
pub struct Options<'a> {
    pub model: &'a str,
    pub import_mapping: &'a str,
    pub format_mapping: &'a str,
    pub default_mapping: &'a str,
    pub deduplicate: bool,
    pub unique_column: Option<&'a str>,
    pub unique_type: UniqueType,
    pub update_mapping: &'a str,
    pub validate_required: bool,
}

impl Params {
    /// Checks every option. Runs before the file is downloaded, so a configuration mistake does
    /// not wait for the download.
    pub fn new(options: Options) -> Result<Self, String> {
        let model = options.model.trim();
        if !mapping::is_name(model) {
            return Err(String::from("Select the model to import the records into"));
        }

        let import = mapping::mapped_columns(
            &mapping::parse_map(options.import_mapping, "The Import mapping")?,
            "the Import mapping",
        )?;
        if import.is_empty() {
            return Err(String::from(
                "The Import mapping is empty: add a column name from the file (key) and a property (value), e.g. First name -> first_name",
            ));
        }
        let update = mapping::mapped_columns(
            &mapping::parse_map(options.update_mapping, "The Update mapping")?,
            "the Update mapping",
        )?;
        let defaults = mapping::default_values(&mapping::parse_map(
            options.default_mapping,
            "The Default mapping",
        )?)?;

        let mut formats = Vec::new();
        for (key, spec) in mapping::parse_map(options.format_mapping, "The Format mapping")? {
            let column = mapping::column_name(&key);
            if column.is_empty() && spec.as_str().is_none_or(|s| s.trim().is_empty()) {
                continue;
            }
            let known = import.iter().chain(&update).any(|c| c.column == column);
            if !known {
                return Err(format!(
                    "The column \"{column}\" in the Format mapping is not in the Import mapping or the Update mapping: use the same column name"
                ));
            }
            formats.push((column.to_string(), format::parse(column, &spec)?));
        }

        let dedup = if options.deduplicate {
            let column = options
                .unique_column
                .map(mapping::column_name)
                .filter(|column| !column.is_empty())
                .ok_or(
                    "Deduplicate records is on: fill in the Unique record identifier with the column name that identifies a record",
                )?;
            let mapped = import
                .iter()
                .find(|c| c.column.to_lowercase() == column.to_lowercase())
                .ok_or_else(|| {
                    format!(
                        "The Unique record identifier \"{column}\" is not a column in the Import mapping: add the column and its property there"
                    )
                })?;
            if !matches!(mapped.target, Target::Property(_)) {
                return Err(format!(
                    "The Unique record identifier \"{column}\" is mapped to a relation: map it to a property of the model"
                ));
            }
            Some((mapped.column.clone(), options.unique_type))
        } else {
            None
        };

        Ok(Self {
            model: model.to_string(),
            import,
            update,
            formats,
            defaults,
            dedup,
            validate_required: options.validate_required,
        })
    }

    fn format_of(&self, column: &str) -> Option<&Format> {
        self.formats
            .iter()
            .find(|(name, _)| name == column)
            .map(|(_, format)| format)
    }

    fn match_kind_of(&self, column: &str) -> MatchKind {
        match self.format_of(column) {
            Some(Format::Number) => MatchKind::Number,
            Some(Format::Decimal) => MatchKind::Decimal,
            _ => MatchKind::Text,
        }
    }
}

#[derive(Debug, Default, PartialEq)]
pub struct Summary {
    pub created: usize,
    pub updated: usize,
    /// Rows left out because their unique column was empty (deduplication only).
    pub skipped: usize,
    pub unique_column: String,
    /// Relation values for which no related record was found.
    pub unmatched: usize,
}

impl Summary {
    pub fn text(&self) -> String {
        let mut text = format!(
            "Records created: {}, records updated: {}",
            self.created, self.updated
        );
        if self.skipped > 0 {
            text += &format!(
                "; rows skipped because \"{}\" is empty: {}",
                self.unique_column, self.skipped
            );
        }
        if self.unmatched > 0 {
            text += &format!(
                "; relation values without a matching record: {}",
                self.unmatched
            );
        }
        text
    }
}

/// The values of one row, by column name. A column that is not in the file has no entry.
type RowValues = HashMap<String, Value>;

pub fn run(table: &Table, params: &Params, request: Request) -> Result<Summary, String> {
    let index_of = |column: &str| {
        table
            .column(column)
            .or_else(|| table.column(&format!("{column}*")))
    };
    let all_columns: Vec<&MappedColumn> = params.import.iter().chain(&params.update).collect();

    if params.validate_required {
        let mut missing: Vec<&str> = all_columns
            .iter()
            .filter(|c| c.required && index_of(&c.column).is_none())
            .map(|c| c.column.as_str())
            .collect();
        missing.dedup();
        if !missing.is_empty() {
            return Err(format!(
                "Required columns are missing from the file: {}. Nothing was imported",
                missing.join(", ")
            ));
        }
    }
    if params.import.iter().all(|c| index_of(&c.column).is_none()) {
        return Err(format!(
            "None of the columns of the Import mapping are in the file. The file has the columns: {}. Check the column names and the Delimiter",
            table.headers.join(", ")
        ));
    }
    if let Some((column, _)) = &params.dedup
        && index_of(column).is_none()
    {
        return Err(format!(
            "The Unique record identifier \"{column}\" is not a column in the file. Nothing was imported"
        ));
    }

    // Every value is converted first: a bad value fails the step before anything is written.
    let mut columns: Vec<(&str, usize)> = Vec::new();
    for column in &all_columns {
        if let Some(index) = index_of(&column.column)
            && !columns.iter().any(|(name, _)| *name == column.column)
        {
            columns.push((&column.column, index));
        }
    }
    let mut rows: Vec<(usize, RowValues)> = Vec::with_capacity(table.rows.len());
    for row in &table.rows {
        let mut values = RowValues::new();
        for (name, index) in &columns {
            let cell = row.cells.get(*index).unwrap_or(&crate::read::Cell::Empty);
            let value = format::convert(cell, params.format_of(name)).map_err(|error| {
                format!(
                    "Row {}, column \"{name}\": {error}. Nothing was imported",
                    row.number
                )
            })?;
            values.insert(name.to_string(), value);
        }
        rows.push((row.number, values));
    }

    let lookups = look_up_relations(params, &rows, request)?;

    let mut summary = Summary::default();
    let mut creates: Vec<(usize, Map<String, Value>)> = Vec::new();
    let mut updates: Vec<(usize, Map<String, Value>)> = Vec::new();

    let existing = match &params.dedup {
        Some((column, kind)) => {
            summary.unique_column = column.clone();
            Some(look_up_existing(params, column, *kind, &rows, request)?)
        }
        None => None,
    };

    for (number, values) in &rows {
        let build = |mapping: &[MappedColumn], unmatched: &mut usize| {
            let mut record = Map::new();
            for (key, value) in &params.defaults {
                record.insert(key.clone(), value.clone());
            }
            for column in mapping {
                let Some(value) = values.get(&column.column) else {
                    continue;
                };
                match &column.target {
                    Target::Property(property) => {
                        record.insert(property.clone(), value.clone());
                    }
                    Target::Relation {
                        field,
                        model,
                        property,
                    } => {
                        let id = format::match_key(value, params.match_kind_of(&column.column))
                            .and_then(|key| {
                                lookups.get(&(model.clone(), property.clone()))?.get(&key)
                            });
                        match id {
                            Some(id) => record.insert(field.clone(), id.clone()),
                            None => {
                                if !value.is_null() {
                                    *unmatched += 1;
                                }
                                // As in the ActionsJS step: no related record is an empty object.
                                record.insert(field.clone(), json!({}))
                            }
                        };
                    }
                }
            }
            record
        };

        let Some((existing, (column, kind))) = existing.as_ref().zip(params.dedup.as_ref()) else {
            creates.push((*number, build(&params.import, &mut summary.unmatched)));
            continue;
        };
        let key =
            unique_key(values.get(column).unwrap_or(&Value::Null), *kind).map_err(|error| {
                format!("Row {number}, column \"{column}\": {error}. Nothing was imported")
            })?;
        let Some(key) = key else {
            summary.skipped += 1;
            continue;
        };
        match existing.get(&key) {
            Some(id) => {
                let mapping = if params.update.is_empty() {
                    &params.import
                } else {
                    &params.update
                };
                let mut record = build(mapping, &mut summary.unmatched);
                record.insert(String::from("id"), id.clone());
                updates.push((*number, record));
            }
            None => creates.push((*number, build(&params.import, &mut summary.unmatched))),
        }
    }

    // createMany does not accept an id.
    for (_, record) in &mut creates {
        record.remove("id");
    }

    let model = &params.model;
    write(
        request,
        &format!("createMany{model}"),
        &creates,
        &mut summary,
        true,
    )?;
    write(
        request,
        &format!("upsertMany{model}"),
        &updates,
        &mut summary,
        false,
    )?;
    Ok(summary)
}

/// The value of the unique column as a key, typed by the Unique record type. `None` for an empty
/// value (the row is skipped).
fn unique_key(value: &Value, kind: UniqueType) -> Result<Option<String>, String> {
    let key = match value {
        Value::Null => return Ok(None),
        Value::String(text) if text.trim().is_empty() => return Ok(None),
        _ => format::match_key(value, kind.match_kind()),
    };
    if matches!(kind, UniqueType::Number | UniqueType::Id)
        && let Some(key) = &key
        && key.parse::<i64>().is_err()
    {
        return Err(format!(
            "\"{key}\" is not a whole number, but the Unique record type is {}",
            if kind == UniqueType::Id {
                "Id"
            } else {
                "Number"
            }
        ));
    }
    if kind == UniqueType::Decimal
        && let Some(key) = &key
        && key.parse::<f64>().is_err()
    {
        return Err(format!(
            "\"{key}\" is not a decimal number, but the Unique record type is Decimal"
        ));
    }
    Ok(key)
}

/// The value sent in a filter for a key: a number for Number and Id, text otherwise.
fn filter_value(key: &str, kind: MatchKind, original: &Value) -> Value {
    match kind {
        MatchKind::Number => key.parse::<i64>().map_or(original.clone(), Value::from),
        MatchKind::Decimal => match original {
            Value::Number(_) => original.clone(),
            _ => Value::String(key.to_string()),
        },
        MatchKind::Text => match original {
            Value::String(_) => original.clone(),
            _ => Value::String(key.to_string()),
        },
    }
}

type Lookups = HashMap<(String, String), HashMap<String, Value>>;

fn look_up_relations(
    params: &Params,
    rows: &[(usize, RowValues)],
    request: Request,
) -> Result<Lookups, String> {
    let mut lookups = Lookups::new();
    let mut done: Vec<(String, String)> = Vec::new();
    for column in params.import.iter().chain(&params.update) {
        let Target::Relation {
            model, property, ..
        } = &column.target
        else {
            continue;
        };
        let kind = params.match_kind_of(&column.column);
        let mut values: Vec<(String, Value)> = Vec::new();
        for (_, row) in rows {
            let Some(value) = row.get(&column.column) else {
                continue;
            };
            if let Some(key) = format::match_key(value, kind)
                && !values.iter().any(|(existing, _)| *existing == key)
            {
                let filter = filter_value(&key, kind, value);
                values.push((key, filter));
            }
        }
        let pair = (model.clone(), property.clone());
        let found = fetch_matches(request, model, property, kind, &values)?;
        if done.contains(&pair) {
            lookups.entry(pair).or_default().extend(found);
        } else {
            done.push(pair.clone());
            lookups.insert(pair, found);
        }
    }
    Ok(lookups)
}

fn look_up_existing(
    params: &Params,
    column: &str,
    kind: UniqueType,
    rows: &[(usize, RowValues)],
    request: Request,
) -> Result<HashMap<String, Value>, String> {
    let Some(Target::Property(property)) = params
        .import
        .iter()
        .find(|c| c.column == column)
        .map(|c| &c.target)
    else {
        unreachable!("checked in Params::new");
    };
    let mut values: Vec<(String, Value)> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for (number, row) in rows {
        let value = row.get(column).unwrap_or(&Value::Null);
        let key = unique_key(value, kind).map_err(|error| {
            format!("Row {number}, column \"{column}\": {error}. Nothing was imported")
        })?;
        if let Some(key) = key
            && seen.insert(key.clone())
        {
            let filter = filter_value(&key, kind.match_kind(), value);
            values.push((key, filter));
        }
    }
    fetch_matches(request, &params.model, property, kind.match_kind(), &values)
}

/// The id of the record of `model` whose `property` has each value, keyed by `match_key`. When
/// several records have the same value, the first one returned wins.
fn fetch_matches(
    request: Request,
    model: &str,
    property: &str,
    kind: MatchKind,
    values: &[(String, Value)],
) -> Result<HashMap<String, Value>, String> {
    let mut matches = HashMap::new();
    let field = format!("all{model}");
    for chunk in values.chunks(PAGE_SIZE) {
        let filters: Vec<Value> = chunk.iter().map(|(_, value)| value.clone()).collect();
        let mut condition = Map::new();
        // As in the ActionsJS step, decimals are matched with `eq` one by one, not with `in`.
        let filter = if kind == MatchKind::Decimal {
            let alternatives: Vec<Value> = filters
                .into_iter()
                .map(|value| {
                    let mut single = Map::new();
                    single.insert(property.to_string(), json!({ "eq": value }));
                    Value::Object(single)
                })
                .collect();
            json!({ "_or": alternatives })
        } else {
            condition.insert(property.to_string(), json!({ "in": filters }));
            Value::Object(condition)
        };
        let variables = json!({ "where": filter }).to_string();

        let mut skip = 0;
        loop {
            let query = format!(
                "query($where: {model}FilterInput) {{ {field}(where: $where, skip: {skip}, take: {PAGE_SIZE}) {{ results {{ id {property} }} totalCount }} }}"
            );
            let response = request(&query, &variables).map_err(|error| {
                format!(
                    "Looking up {model} records by {property} failed: {}. Nothing was imported",
                    host_error_message(&error)
                )
            })?;
            let (records, total) = read_page(&response, &field)
                .map_err(|error| format!("{error}. Nothing was imported"))?;
            let count = records.len();
            for record in records {
                let key = record
                    .get(property)
                    .and_then(|value| format::match_key(value, kind));
                if let (Some(key), Some(id)) = (key, record.get("id")) {
                    matches.entry(key).or_insert_with(|| id.clone());
                }
            }
            skip += count;
            if count == 0 || total.is_none_or(|total| skip as u64 >= total) {
                break;
            }
        }
    }
    Ok(matches)
}

fn write(
    request: Request,
    mutation: &str,
    records: &[(usize, Map<String, Value>)],
    summary: &mut Summary,
    create: bool,
) -> Result<(), String> {
    let query = format!("mutation {{ {mutation}(input: $input) {{ id }} }}");
    for chunk in records.chunks(WRITE_CHUNK) {
        let input: Vec<&Map<String, Value>> = chunk.iter().map(|(_, record)| record).collect();
        let variables = json!({ "input": input }).to_string();
        let result = request(&query, &variables)
            .map_err(|error| host_error_message(&error))
            .and_then(|response| check_mutation(&response, mutation));
        if let Err(error) = result {
            let first = chunk.first().map_or(0, |(number, _)| *number);
            let last = chunk.last().map_or(0, |(number, _)| *number);
            return Err(format!(
                "{} the records of rows {first} to {last} failed: {error}. Written before this: {} created, {} updated",
                if create { "Creating" } else { "Updating" },
                summary.created,
                summary.updated
            ));
        }
        if create {
            summary.created += chunk.len();
        } else {
            summary.updated += chunk.len();
        }
    }
    Ok(())
}

fn check_mutation(response: &str, mutation: &str) -> Result<(), String> {
    let parsed: Value = serde_json::from_str(response)
        .map_err(|error| format!("the Data API response is not valid JSON: {error}"))?;
    if let Some(messages) = graphql_error_messages(&parsed) {
        return Err(messages);
    }
    match parsed.get("data").and_then(|data| data.get(mutation)) {
        Some(Value::Array(_)) => Ok(()),
        _ => Err(format!("the Data API returned no result for {mutation}")),
    }
}

/// The messages of a GraphQL error envelope (`{"errors": [{"message": ...}]}`), if it is one.
fn graphql_error_messages(response: &Value) -> Option<String> {
    let errors = response.get("errors")?.as_array()?;
    if errors.is_empty() {
        return None;
    }
    let messages: Vec<String> = errors
        .iter()
        .map(|error| match error.get("message").and_then(Value::as_str) {
            Some(message) => message.to_string(),
            None => String::from("The Data API returned an error without a message"),
        })
        .collect();
    Some(messages.join("; "))
}

/// The host reports a backend error as `Err(<the GraphQL error envelope as a JSON string>)`.
fn host_error_message(raw: &str) -> String {
    serde_json::from_str::<Value>(raw)
        .ok()
        .and_then(|parsed| graphql_error_messages(&parsed))
        .unwrap_or_else(|| raw.to_string())
}

/// The records of one page and the total count.
fn read_page(response: &str, field: &str) -> Result<(Vec<Value>, Option<u64>), String> {
    let mut parsed: Value = serde_json::from_str(response)
        .map_err(|error| format!("The Data API response is not valid JSON: {error}"))?;
    if let Some(messages) = graphql_error_messages(&parsed) {
        return Err(messages);
    }
    let node = parsed
        .get_mut("data")
        .and_then(|data| data.get_mut(field))
        .filter(|node| node.is_object())
        .ok_or_else(|| {
            format!(
                "The Data API returned no data for `{field}`: check the model and its permissions"
            )
        })?;
    let total = node.get("totalCount").and_then(Value::as_u64);
    match node.get_mut("results").map(Value::take) {
        Some(Value::Array(records)) => Ok((records, total)),
        _ => Err(format!(
            "The Data API returned no results list for `{field}`"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::read::{Cell, Row};

    fn table(headers: &[&str], rows: &[&[&str]]) -> Table {
        Table {
            headers: headers.iter().map(|h| h.to_string()).collect(),
            rows: rows
                .iter()
                .enumerate()
                .map(|(index, cells)| Row {
                    number: index + 2,
                    cells: cells
                        .iter()
                        .map(|c| {
                            if c.is_empty() {
                                Cell::Empty
                            } else {
                                Cell::Text(c.to_string())
                            }
                        })
                        .collect(),
                })
                .collect(),
        }
    }

    fn options<'a>(import: &'a str) -> Options<'a> {
        Options {
            model: "Person",
            import_mapping: import,
            format_mapping: "",
            default_mapping: "",
            deduplicate: false,
            unique_column: None,
            unique_type: UniqueType::Text,
            update_mapping: "",
            validate_required: false,
        }
    }

    /// A Data API that answers lookups from `records` (model -> records) and records every
    /// mutation.
    struct Mock {
        records: Vec<(&'static str, Value)>,
        mutations: Vec<(String, Value)>,
        queries: Vec<(String, Value)>,
        fail_mutation: Option<&'static str>,
    }

    impl Mock {
        fn new(records: Vec<(&'static str, Value)>) -> Self {
            Self {
                records,
                mutations: Vec::new(),
                queries: Vec::new(),
                fail_mutation: None,
            }
        }

        fn answer(&mut self, query: &str, variables: &str) -> Result<String, String> {
            let variables: Value = serde_json::from_str(variables).unwrap();
            if let Some(rest) = query.strip_prefix("mutation { ") {
                let name = rest.split('(').next().unwrap().to_string();
                if self.fail_mutation.is_some_and(|fail| name == fail) {
                    return Err(r#"{"errors":[{"message":"validation failed"}]}"#.into());
                }
                let count = variables["input"].as_array().unwrap().len();
                self.mutations
                    .push((name.clone(), variables["input"].clone()));
                let ids: Vec<Value> = (0..count).map(|i| json!({ "id": i })).collect();
                return Ok(json!({ "data": { name: ids } }).to_string());
            }
            self.queries.push((query.to_string(), variables.clone()));
            let model = query
                .split("FilterInput")
                .next()
                .unwrap()
                .trim_start_matches("query($where: ");
            let field = format!("all{model}");
            // Only `in` and `_or`/`eq` filters on one property are used.
            let filter = variables["where"].as_object().unwrap();
            let matches = |record: &Value| -> bool {
                if let Some(alternatives) = filter.get("_or") {
                    return alternatives.as_array().unwrap().iter().any(|alt| {
                        let (property, condition) = alt.as_object().unwrap().iter().next().unwrap();
                        // `eq` is only used for decimals, which the database compares by value.
                        format::match_key(&record[property], MatchKind::Decimal)
                            == format::match_key(&condition["eq"], MatchKind::Decimal)
                    });
                }
                let (property, condition) = filter.iter().next().unwrap();
                condition["in"]
                    .as_array()
                    .unwrap()
                    .contains(&record[property])
            };
            let results: Vec<Value> = self
                .records
                .iter()
                .filter(|(m, record)| *m == model && matches(record))
                .map(|(_, record)| record.clone())
                .collect();
            Ok(
                json!({ "data": { field: { "results": results, "totalCount": results.len() } } })
                    .to_string(),
            )
        }
    }

    fn run_with(table: &Table, params: &Params, mock: &mut Mock) -> Result<Summary, String> {
        run(table, params, &mut |query, variables| {
            mock.answer(query, variables)
        })
    }

    #[test]
    fn rows_become_created_records_with_formats_and_defaults() {
        let mut options = options(
            r#"{"Name": "first_name", "Age": "age", "Born": "birth_date", "Active": "is_active"}"#,
        );
        options.format_mapping =
            r#"{"Age": "number", "Born": "Date, dd-MM-yyyy", "Active": "checkbox"}"#;
        options.default_mapping = r#"{"source": "import"}"#;
        let params = Params::new(options).unwrap();
        let table = table(
            &["Name", "Age", "Born", "Active", "Other"],
            &[
                &["Ann", "30", "31-01-1994", "yes", "x"],
                &["Bob", "", "", "0"],
            ],
        );
        let mut mock = Mock::new(vec![]);
        let summary = run_with(&table, &params, &mut mock).unwrap();
        assert_eq!(summary.text(), "Records created: 2, records updated: 0");
        assert_eq!(
            mock.mutations,
            vec![(
                "createManyPerson".into(),
                json!([
                    {"firstName": "Ann", "age": 30, "birthDate": "1994-01-31", "isActive": true, "source": "import"},
                    {"firstName": "Bob", "age": null, "birthDate": null, "isActive": false, "source": "import"},
                ])
            )]
        );
    }

    #[test]
    fn a_bad_value_fails_before_anything_is_written() {
        let mut options = options(r#"{"Age": "age"}"#);
        options.format_mapping = r#"{"Age": "number"}"#;
        let params = Params::new(options).unwrap();
        let mut mock = Mock::new(vec![]);
        let error =
            run_with(&table(&["Age"], &[&["1"], &["two"]]), &params, &mut mock).unwrap_err();
        assert_eq!(
            error,
            "Row 3, column \"Age\": \"two\" is not a whole number. Nothing was imported"
        );
        assert!(mock.mutations.is_empty());
    }

    #[test]
    fn missing_columns_are_left_out_or_fail_when_required() {
        let params = Params::new(options(r#"{"Name": "name", "Email*": "email"}"#)).unwrap();
        let mut mock = Mock::new(vec![]);
        run_with(&table(&["Name"], &[&["Ann"]]), &params, &mut mock).unwrap();
        assert_eq!(mock.mutations[0].1, json!([{"name": "Ann"}]));

        let mut required = options(r#"{"Name": "name", "Email*": "email"}"#);
        required.validate_required = true;
        let params = Params::new(required).unwrap();
        assert_eq!(
            run_with(
                &table(&["Name"], &[&["Ann"]]),
                &params,
                &mut Mock::new(vec![])
            )
            .unwrap_err(),
            "Required columns are missing from the file: Email. Nothing was imported"
        );
        // The column may also be named with the * in the file.
        run_with(
            &table(&["Name", "Email*"], &[&["Ann", "a@b.nl"]]),
            &params,
            &mut mock,
        )
        .unwrap();
        assert_eq!(
            mock.mutations[1].1,
            json!([{"name": "Ann", "email": "a@b.nl"}])
        );
    }

    #[test]
    fn no_mapped_column_in_the_file_points_at_the_delimiter() {
        let params = Params::new(options(r#"{"Name": "name"}"#)).unwrap();
        let error = run_with(
            &table(&["Name;Age"], &[&["Ann;3"]]),
            &params,
            &mut Mock::new(vec![]),
        )
        .unwrap_err();
        assert!(
            error.contains("The file has the columns: Name;Age"),
            "{error}"
        );
        assert!(error.contains("Delimiter"), "{error}");
    }

    #[test]
    fn relations_are_looked_up_by_property() {
        let params = Params::new(options(
            r#"{"Name": "name", "Company": "company.legal_name"}"#,
        ))
        .unwrap();
        let mut mock = Mock::new(vec![
            ("Company", json!({"id": 7, "legalName": "Acme"})),
            ("Company", json!({"id": 8, "legalName": "Beta"})),
        ]);
        let summary = run_with(
            &table(
                &["Name", "Company"],
                &[
                    &["Ann", "Acme"],
                    &["Bob", "Nope"],
                    &["Cy", ""],
                    &["Di", "Acme"],
                ],
            ),
            &params,
            &mut mock,
        )
        .unwrap();
        assert_eq!(
            summary.text(),
            "Records created: 4, records updated: 0; relation values without a matching record: 1"
        );
        assert_eq!(
            mock.queries,
            vec![(
                "query($where: CompanyFilterInput) { allCompany(where: $where, skip: 0, take: 200) { results { id legalName } totalCount } }".into(),
                json!({"where": {"legalName": {"in": ["Acme", "Nope"]}}})
            )]
        );
        assert_eq!(
            mock.mutations[0].1,
            json!([
                {"name": "Ann", "company": 7},
                {"name": "Bob", "company": {}},
                {"name": "Cy", "company": {}},
                {"name": "Di", "company": 7},
            ])
        );
    }

    #[test]
    fn deduplication_updates_matched_records_and_skips_empty_keys() {
        let mut options = options(r#"{"Code": "code", "Name": "name"}"#);
        options.deduplicate = true;
        options.unique_column = Some("code");
        options.unique_type = UniqueType::Number;
        options.update_mapping = r#"{"Name": "display_name"}"#;
        options.default_mapping = r#"{"imported": true}"#;
        let params = Params::new(options).unwrap();
        let mut mock = Mock::new(vec![("Person", json!({"id": 41, "code": 2}))]);
        let summary = run_with(
            &table(
                &["Code", "Name"],
                &[
                    &["1", "New"],
                    &["2", "Old"],
                    &["", "Empty"],
                    &["2.0", "Again"],
                ],
            ),
            &params,
            &mut mock,
        )
        .unwrap();
        assert_eq!(
            summary.text(),
            "Records created: 1, records updated: 2; rows skipped because \"Code\" is empty: 1"
        );
        // Numbers are sent as numbers and each value once.
        assert_eq!(
            mock.queries[0].1,
            json!({"where": {"code": {"in": [1, 2]}}})
        );
        assert_eq!(
            mock.mutations,
            vec![
                (
                    "createManyPerson".into(),
                    json!([{"code": "1", "name": "New", "imported": true}])
                ),
                (
                    "upsertManyPerson".into(),
                    json!([
                        {"id": 41, "displayName": "Old", "imported": true},
                        {"id": 41, "displayName": "Again", "imported": true},
                    ])
                ),
            ]
        );
    }

    #[test]
    fn deduplication_without_an_update_mapping_updates_with_the_import_mapping() {
        let mut options = options(r#"{"Email": "email", "Name": "name"}"#);
        options.deduplicate = true;
        options.unique_column = Some("Email");
        let params = Params::new(options).unwrap();
        let mut mock = Mock::new(vec![("Person", json!({"id": "5", "email": "a@b.nl"}))]);
        run_with(
            &table(&["Email", "Name"], &[&["a@b.nl", "Ann"]]),
            &params,
            &mut mock,
        )
        .unwrap();
        assert_eq!(
            mock.mutations,
            vec![(
                "upsertManyPerson".into(),
                json!([{"id": "5", "email": "a@b.nl", "name": "Ann"}])
            )]
        );
    }

    #[test]
    fn decimal_keys_are_matched_with_eq() {
        let mut options = options(r#"{"Price": "price"}"#);
        options.deduplicate = true;
        options.unique_column = Some("Price");
        options.unique_type = UniqueType::Decimal;
        options.format_mapping = r#"{"Price": "decimal"}"#;
        let params = Params::new(options).unwrap();
        let mut mock = Mock::new(vec![("Person", json!({"id": 1, "price": "12.50"}))]);
        let summary = run_with(
            &table(&["Price"], &[&["12,50"], &["3"]]),
            &params,
            &mut mock,
        )
        .unwrap();
        assert_eq!(summary.text(), "Records created: 1, records updated: 1");
        assert_eq!(
            mock.queries[0].1,
            json!({"where": {"_or": [{"price": {"eq": "12.5"}}, {"price": {"eq": "3"}}]}})
        );
    }

    #[test]
    fn a_failed_write_says_what_was_written() {
        let mut options = options(r#"{"Code": "code"}"#);
        options.deduplicate = true;
        options.unique_column = Some("Code");
        let params = Params::new(options).unwrap();
        let mut mock = Mock::new(vec![("Person", json!({"id": 1, "code": "a"}))]);
        mock.fail_mutation = Some("upsertManyPerson");
        let error = run_with(&table(&["Code"], &[&["a"], &["b"]]), &params, &mut mock).unwrap_err();
        assert_eq!(
            error,
            "Updating the records of rows 2 to 2 failed: validation failed. Written before this: 1 created, 0 updated"
        );
    }

    #[test]
    fn configuration_mistakes_are_found_before_the_download() {
        let check = |options: Options| Params::new(options).unwrap_err();
        assert!(check(options("")).contains("The Import mapping is empty"));
        let mut o = options(r#"{"A": "a"}"#);
        o.model = "";
        assert!(check(o).contains("Select the model"));
        let mut o = options(r#"{"A": "a"}"#);
        o.format_mapping = r#"{"B": "number"}"#;
        assert!(check(o).contains("\"B\" in the Format mapping is not in the Import mapping"));
        let mut o = options(r#"{"A": "a"}"#);
        o.deduplicate = true;
        assert!(check(o).contains("fill in the Unique record identifier"));
        let mut o = options(r#"{"A": "a"}"#);
        o.deduplicate = true;
        o.unique_column = Some("B");
        assert!(check(o).contains("\"B\" is not a column in the Import mapping"));
        let mut o = options(r#"{"A": "company.name"}"#);
        o.deduplicate = true;
        o.unique_column = Some("A");
        assert!(check(o).contains("mapped to a relation"));
        // A format may name a required column with its *.
        let mut o = options(r#"{"A*": "a"}"#);
        o.format_mapping = r#"{"A*": "number"}"#;
        assert!(Params::new(o).is_ok());
    }

    #[test]
    fn records_are_written_in_chunks() {
        let params = Params::new(options(r#"{"N": "n"}"#)).unwrap();
        let numbers: Vec<String> = (0..WRITE_CHUNK + 1).map(|n| n.to_string()).collect();
        let rows: Vec<Vec<&str>> = numbers.iter().map(|n| vec![n.as_str()]).collect();
        let rows: Vec<&[&str]> = rows.iter().map(Vec::as_slice).collect();
        let mut mock = Mock::new(vec![]);
        let summary = run_with(&table(&["N"], &rows), &params, &mut mock).unwrap();
        assert_eq!(summary.created, WRITE_CHUNK + 1);
        assert_eq!(mock.mutations.len(), 2);
        assert_eq!(mock.mutations[1].1.as_array().unwrap().len(), 1);
    }
}
