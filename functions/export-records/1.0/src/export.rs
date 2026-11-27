//! Fetches every record that matches the filter, page by page, and writes them as a CSV or XLSX
//! file.

use serde_json::Value;

use crate::cell;
use crate::filter;
use crate::format::{self, Format};
use crate::mapping::{self, Column};

/// The records asked for per request. The Data API accepts a `take` of 5000 (an export of 200,000
/// records took 24 seconds on a canvas, 50,000 took 4.4); earlier it accepted at most 200
/// (docs/developer-learnings-log.md, 2026-10-08).
pub const PAGE_SIZE: usize = 5000;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FileType {
    Csv,
    Xlsx,
}

impl FileType {
    pub fn extension(self) -> &'static str {
        match self {
            Self::Csv => "csv",
            Self::Xlsx => "xlsx",
        }
    }
}

pub struct Params<'a> {
    pub file_type: FileType,
    pub model: &'a str,
    pub filter: &'a str,
    pub variables: &'a Value,
    /// The sort property's name (database or GraphQL name), if one was chosen.
    pub sort_field: Option<&'a str>,
    pub sort_order: &'a str,
    pub delimiter: &'a str,
    /// The sheet name of an XLSX file (the ActionsJS step used the target model's name).
    pub sheet: &'a str,
    pub mapping: &'a [(String, Value)],
    pub formats: &'a [(String, Value)],
    pub bom: bool,
    /// Put a `'` in front of CSV text that could be read as a formula.
    pub escape_formulas: bool,
}

/// The query for one page of `take` records.
pub fn build_query(params: &Params, skip: usize, take: usize) -> Result<(String, String), String> {
    let model = params.model.trim();
    if !mapping::is_name(model) {
        return Err(String::from("Select the model to export records from"));
    }
    let columns = mapping::columns(params.mapping)?;
    // `id` is always selected: it is how a record that comes back on two pages is noticed.
    let selection = format!("id {}", mapping::selection(&columns));

    let filter = filter::build(params.filter, params.variables)?;
    let order = match params.sort_order.trim().to_uppercase().as_str() {
        "" | "ASC" => "ASC",
        "DESC" => "DESC",
        other => return Err(format!("Unknown sort order \"{other}\": use ASC or DESC")),
    };
    // Pages are separate requests: without a sort order a record could be exported twice or not
    // at all, so the records are sorted by id when no sort property is chosen.
    let sort_field = match params.sort_field.map(str::trim).filter(|f| !f.is_empty()) {
        // Checked before the conversion, like the mapping paths: `snake_to_camel` would turn
        // `name } x` into the harmless-looking `nameX`.
        Some(field) => {
            if !mapping::is_name(field) {
                return Err(format!(
                    "The sort property \"{field}\" is not valid: use a property name with only letters, digits and _"
                ));
            }
            mapping::snake_to_camel(field)
        }
        None => String::from("id"),
    };

    let field = format!("all{model}");
    let query = format!(
        "query {{ {field}(where: {{ {filter}\n}}, sort: {{ field: {sort_field}, order: {order} }}, skip: {skip}, take: {take}) {{ results {{ {selection} }} totalCount }} }}"
    );
    Ok((query, field))
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

/// The `totalCount` of a response: a whole number of at least 0 (`250` or `250.0`). A missing or
/// `null` count is `None`. Any other value is an error: reading it as missing would end the export
/// at the first short page, which cuts it off when the Data API caps pages.
fn read_total(value: Option<&Value>) -> Result<Option<u64>, String> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(number)) => {
            let whole = number.as_u64().or_else(|| {
                number
                    .as_f64()
                    .filter(|float| *float >= 0.0 && float.fract() == 0.0 && *float < 1e15)
                    .map(|float| float as u64)
            });
            whole.map(Some).ok_or_else(|| {
                format!("The Data API returned a totalCount that is not a whole number: {number}")
            })
        }
        Some(_) => Err(String::from(
            "The Data API returned a totalCount that is not a number",
        )),
    }
}

/// The records of one page and the total count. A missing or `null` node or `results` is an
/// error, not an empty page: it would otherwise give a header-only or cut-off file without a
/// word. A real empty result is `results: []` with `totalCount: 0`.
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
    let total = read_total(node.get("totalCount"))?;
    let records = match node.get_mut("results").map(Value::take) {
        Some(Value::Array(records)) => records,
        Some(Value::Null) | None => {
            return Err(format!(
                "The Data API returned no results list for `{field}`"
            ));
        }
        Some(_) => return Err(format!("The Data API results for `{field}` are not a list")),
    };
    Ok((records, total))
}

/// The most rows below the header an Excel sheet holds.
pub const XLSX_MAX_ROWS: u64 = 1_048_575;

/// Fetches every matching record with `send`, a page at a time.
///
/// It keeps asking until it has `totalCount` records or a page comes back empty, so a Data API
/// that returns fewer records per page than asked does not cut the export short. When the last
/// page is empty while `totalCount` says more records exist, the step fails instead of writing an
/// incomplete file. Only when the response has no `totalCount` does a page with fewer records than
/// asked end the export.
///
/// Pages are asked for with `PAGE_SIZE` records. A record that comes back on two pages (equal values in the sort property, so the order differs between requests) fails the
/// step: the file would have duplicates and miss records.
pub fn fetch_all(
    params: &Params,
    mut send: impl FnMut(&str) -> Result<String, String>,
) -> Result<Vec<Value>, String> {
    validate(params)?;
    let mut records: Vec<Value> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    loop {
        let (query, field) = build_query(params, records.len(), PAGE_SIZE)?;
        let response = send(&query).map_err(|error| host_error_message(&error))?;
        let (page, total) = read_page(&response, &field)?;
        if let Some(total) = total
            && params.file_type == FileType::Xlsx
            && total > XLSX_MAX_ROWS
        {
            return Err(format!(
                "{total} records match, more than an Excel sheet holds (at most 1,048,575): \
                 narrow the filter or export to CSV"
            ));
        }
        let received = page.len();
        for record in &page {
            if let Some(id) = record.get("id").filter(|id| !id.is_null())
                && !seen.insert(id.to_string())
            {
                return Err(format!(
                    "The record with id {id} came back on two pages, so the export would have \
                     duplicates and miss records. The sort property has equal values (or records \
                     changed during the export): sort by a property with unique values, or leave \
                     the sort property empty to sort by id"
                ));
            }
        }
        records.extend(page);
        let fetched = records.len() as u64;
        match total {
            Some(total) if fetched >= total => return Ok(records),
            Some(total) if received == 0 => {
                return Err(format!(
                    "The Data API stopped returning records after {fetched} of {total}: \
                     the export is incomplete"
                ));
            }
            Some(_) => {}
            None if received < PAGE_SIZE => return Ok(records),
            None => {}
        }
    }
}

/// The format of each column, in column order. A Format mapping key that is no column header, or
/// a value that is not text, is a configuration mistake and fails the step.
fn column_formats(
    specs: &[(String, Value)],
    columns: &[Column],
) -> Result<Vec<Option<Format>>, String> {
    // A blank row left in the Map option (no key, no value) is not a mistake: skip it.
    let blank = |(key, value): &&(String, Value)| {
        key.trim().is_empty()
            && match value {
                Value::Null => true,
                Value::String(text) => text.trim().is_empty(),
                _ => false,
            }
    };
    let specs: Vec<&(String, Value)> = specs.iter().filter(|entry| !blank(entry)).collect();
    for (key, _) in &specs {
        if !columns.iter().any(|column| column.header == *key) {
            let headers: Vec<&str> = columns.iter().map(|c| c.header.as_str()).collect();
            return Err(format!(
                "The Format mapping has the column \"{key}\", which is not a column of the Property mapping. Use one of its keys (the column names): {}",
                headers.join(", ")
            ));
        }
    }
    columns
        .iter()
        .map(|column| {
            match specs.iter().find(|(key, _)| *key == column.header) {
                None => Ok(None),
                Some((_, Value::String(spec))) => format::parse(&column.header, spec).map(Some),
                Some((_, other)) => Err(format!(
                    "The format of the column \"{}\" must be text like Date|dd-MM-yyyy, not {other}",
                    column.header
                )),
            }
        })
        .collect()
}

/// Checks the options that do not need the Data API (the model, filter, sort, mapping and
/// formats), so a configuration mistake fails before the first request.
pub fn validate(params: &Params) -> Result<(), String> {
    build_query(params, 0, PAGE_SIZE)?;
    let columns = mapping::columns(params.mapping)?;
    column_formats(params.formats, &columns)?;
    Ok(())
}

/// The values of a to-many relation in one cell are separated by a line break.
const MULTI_VALUE_SEPARATOR: &str = "\n";

/// The file's bytes: the header row, then one row per record with the formats applied.
pub fn write_file(params: &Params, records: &[Value]) -> Result<Vec<u8>, String> {
    let columns: Vec<Column> = mapping::columns(params.mapping)?;
    let headers: Vec<String> = columns.iter().map(|c| c.header.clone()).collect();
    let formats = column_formats(params.formats, &columns)?;

    let mut rows = Vec::with_capacity(records.len());
    // Real Excel dates for the single-valued date columns (the text in `rows` stays as the
    // fallback and for the column width).
    let mut dates = Vec::new();
    let excel = matches!(params.file_type, FileType::Xlsx);
    for record in records {
        let mut row = Vec::with_capacity(columns.len());
        let mut date_row = Vec::with_capacity(columns.len());
        for (column, format) in columns.iter().zip(&formats) {
            let (values, multi) = mapping::lookup_all(record, &column.path);
            let mut date = None;
            row.push(if multi {
                // A to-many relation: every value on its own line in one cell.
                let mut lines = Vec::with_capacity(values.len());
                for value in values {
                    let value = format::apply(&column.header, value, format.as_ref())?;
                    lines.push(cell::value_to_text(&value));
                }
                (!lines.is_empty()).then(|| Value::String(lines.join(MULTI_VALUE_SEPARATOR)))
            } else {
                match values.first() {
                    Some(value) => {
                        let formatted = format::apply(&column.header, value, format.as_ref())?;
                        if let (true, Some(Format::Date(tokens)), Value::String(text)) =
                            (excel, format, value)
                        {
                            date = format::excel_date(text, tokens);
                        }
                        Some(formatted)
                    }
                    None => None,
                }
            });
            date_row.push(date);
        }
        rows.push(row);
        dates.push(date_row);
    }

    match params.file_type {
        FileType::Csv => {
            let delimiter = if params.delimiter.is_empty() {
                ","
            } else {
                params.delimiter
            };
            Ok(cell::csv(
                &headers,
                &rows,
                delimiter,
                params.bom,
                params.escape_formulas,
            )
            .into_bytes())
        }
        FileType::Xlsx => cell::xlsx(&headers, &rows, &dates, params.sheet),
    }
}

/// The file name with its extension, without a doubled extension when the name already has it.
/// Path separators and control characters become `_`, so the name cannot point into a folder, and
/// a name that is nothing but the extension (`.csv`) becomes `export`.
pub fn full_file_name(name: Option<&str>, file_type: FileType) -> String {
    let extension = file_type.extension();
    let name: String = name
        .unwrap_or_default()
        .trim()
        .chars()
        .map(|c| {
            if matches!(c, '/' | '\\') || c.is_control() {
                '_'
            } else {
                c
            }
        })
        .collect();
    let suffix = format!(".{extension}");
    let base = if name.to_lowercase().ends_with(&suffix) {
        &name[..name.len() - suffix.len()]
    } else {
        &name
    };
    let base = base.trim();
    let base = if base.is_empty() { "export" } else { base };
    format!("{base}.{extension}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mapping::parse_map;
    use serde_json::json;

    fn params<'a>(
        mapping: &'a [(String, Value)],
        formats: &'a [(String, Value)],
        variables: &'a Value,
    ) -> Params<'a> {
        Params {
            file_type: FileType::Csv,
            model: "Person",
            filter: "",
            variables,
            sort_field: None,
            sort_order: "ASC",
            delimiter: ";",
            sheet: "Export",
            mapping,
            formats,
            bom: false,
            escape_formulas: true,
        }
    }

    fn page(records: Vec<Value>, total: usize) -> String {
        json!({"data": {"allPerson": {"results": records, "totalCount": total}}}).to_string()
    }

    #[test]
    fn the_query_selects_the_mapped_properties() {
        let mapping =
            parse_map(r#"{"Name": "first_name", "Company": "company.name"}"#, "M").unwrap();
        let variables = json!({"t": "x"});
        let mut p = params(&mapping, &[], &variables);
        p.filter = "type: { eq: \"{{t}}\" }";
        p.sort_field = Some("last_name");
        p.sort_order = "DESC";
        let (query, field) = build_query(&p, 400, 200).unwrap();
        assert_eq!(field, "allPerson");
        assert_eq!(
            query,
            "query { allPerson(where: { type: { eq: \"x\" }\n}, sort: { field: lastName, order: DESC }, skip: 400, take: 200) { results { id firstName company { name } } totalCount } }"
        );
    }

    #[test]
    fn without_a_sort_property_the_records_are_sorted_by_id() {
        let mapping = parse_map(r#"{"Id": "id"}"#, "M").unwrap();
        let (query, _) = build_query(&params(&mapping, &[], &json!({})), 0, 200).unwrap();
        assert!(
            query.contains("where: { \n}, sort: { field: id, order: ASC }"),
            "{query}"
        );
    }

    #[test]
    fn all_pages_are_fetched() {
        let mapping = parse_map(r#"{"Id": "id"}"#, "M").unwrap();
        let variables = json!({});
        let p = params(&mapping, &[], &variables);
        let mut skips = Vec::new();
        let records = fetch_all(&p, |query| {
            let skip: usize = query
                .split("skip: ")
                .nth(1)
                .unwrap()
                .split(',')
                .next()
                .unwrap()
                .parse()
                .unwrap();
            skips.push(skip);
            let count = (450 - skip).min(200);
            Ok(page(
                (skip..skip + count).map(|id| json!({"id": id})).collect(),
                450,
            ))
        })
        .unwrap();
        assert_eq!(records.len(), 450);
        assert_eq!(skips, [0, 200, 400]);
        assert_eq!(records[449], json!({"id": 449}));
    }

    #[test]
    fn a_full_last_page_does_not_ask_for_another() {
        let mapping = parse_map(r#"{"Id": "id"}"#, "M").unwrap();
        let variables = json!({});
        let mut calls = 0;
        let records = fetch_all(&params(&mapping, &[], &variables), |_| {
            calls += 1;
            Ok(page(
                (0..PAGE_SIZE).map(|id| json!({"id": id})).collect(),
                PAGE_SIZE,
            ))
        })
        .unwrap();
        assert_eq!((records.len(), calls), (PAGE_SIZE, 1));
    }

    fn skip_of(query: &str) -> usize {
        query
            .split("skip: ")
            .nth(1)
            .unwrap()
            .split(',')
            .next()
            .unwrap()
            .parse()
            .unwrap()
    }

    #[test]
    fn a_data_api_that_caps_pages_lower_still_gives_every_record() {
        let mapping = parse_map(r#"{"Id": "id"}"#, "M").unwrap();
        let variables = json!({});
        let mut skips = Vec::new();
        let records = fetch_all(&params(&mapping, &[], &variables), |query| {
            let skip = skip_of(query);
            skips.push(skip);
            let count = (250 - skip).min(100);
            Ok(page(
                (skip..skip + count).map(|id| json!({"id": id})).collect(),
                250,
            ))
        })
        .unwrap();
        assert_eq!(records.len(), 250);
        assert_eq!(skips, [0, 100, 200]);
    }

    fn take_of(query: &str) -> usize {
        query
            .split("take: ")
            .nth(1)
            .unwrap()
            .split(')')
            .next()
            .unwrap()
            .parse()
            .unwrap()
    }

    #[test]
    fn pages_are_asked_for_with_the_big_page_size() {
        let mapping = parse_map(r#"{"Id": "id"}"#, "M").unwrap();
        let variables = json!({});
        let mut takes = Vec::new();
        let records = fetch_all(&params(&mapping, &[], &variables), |query| {
            takes.push(take_of(query));
            let skip = skip_of(query);
            let count = (12_000 - skip).min(take_of(query));
            Ok(page(
                (skip..skip + count).map(|id| json!({"id": id})).collect(),
                12_000,
            ))
        })
        .unwrap();
        assert_eq!(records.len(), 12_000);
        assert_eq!(takes, [PAGE_SIZE; 3]);
    }

    #[test]
    fn an_error_from_the_data_api_fails_the_step_at_once() {
        let mapping = parse_map(r#"{"Id": "id"}"#, "M").unwrap();
        let variables = json!({});
        let mut calls = 0;
        let error = fetch_all(&params(&mapping, &[], &variables), |_| {
            calls += 1;
            Ok(r#"{"errors": [{"message": "no permission"}]}"#.to_string())
        })
        .unwrap_err();
        assert_eq!(error, "no permission");
        assert_eq!(calls, 1);
    }

    #[test]
    fn a_record_on_two_pages_fails_the_step() {
        let mapping = parse_map(r#"{"Id": "id"}"#, "M").unwrap();
        let variables = json!({});
        let mut p = params(&mapping, &[], &variables);
        p.sort_field = Some("status");
        let error = fetch_all(&p, |query| {
            let skip = skip_of(query);
            // Record 99 is on both pages: the sort order changed between the requests.
            let ids: Vec<usize> = if skip == 0 {
                (0..100).collect()
            } else {
                (99..199).collect()
            };
            Ok(page(ids.iter().map(|id| json!({"id": id})).collect(), 200))
        })
        .unwrap_err();
        assert!(
            error.starts_with("The record with id 99 came back on two pages"),
            "{error}"
        );
    }

    #[test]
    fn a_total_count_that_is_not_a_whole_number_is_an_error_or_missing() {
        let read = |value: Value| read_total(Some(&value));
        assert_eq!(read(json!(250)), Ok(Some(250)));
        assert_eq!(read(json!(250.0)), Ok(Some(250)));
        assert_eq!(read(json!(0)), Ok(Some(0)));
        assert_eq!(read(Value::Null), Ok(None));
        assert_eq!(read_total(None), Ok(None));
        for bad in [json!("250"), json!(-1), json!(2.5), json!(true), json!([1])] {
            assert!(read(bad.clone()).is_err(), "{bad}");
        }
    }

    #[test]
    fn an_empty_page_before_the_total_fails_the_step() {
        let mapping = parse_map(r#"{"Id": "id"}"#, "M").unwrap();
        let variables = json!({});
        let error = fetch_all(&params(&mapping, &[], &variables), |query| {
            let skip = skip_of(query);
            let count = if skip == 0 { 100 } else { 0 };
            Ok(page((0..count).map(|id| json!({"id": id})).collect(), 300))
        })
        .unwrap_err();
        assert_eq!(
            error,
            "The Data API stopped returning records after 100 of 300: the export is incomplete"
        );
    }

    #[test]
    fn without_a_total_count_a_short_page_ends_the_export() {
        let mapping = parse_map(r#"{"Id": "id"}"#, "M").unwrap();
        let variables = json!({});
        let mut calls = 0;
        let records = fetch_all(&params(&mapping, &[], &variables), |query| {
            calls += 1;
            let skip = skip_of(query);
            let count = if skip == 0 { PAGE_SIZE } else { 5 };
            let results: Vec<Value> = (skip..skip + count).map(|id| json!({"id": id})).collect();
            Ok(json!({"data": {"allPerson": {"results": results}}}).to_string())
        })
        .unwrap();
        assert_eq!((records.len(), calls), (PAGE_SIZE + 5, 2));
    }

    #[test]
    fn too_many_records_for_excel_fail_before_fetching_them_all() {
        let mapping = parse_map(r#"{"Id": "id"}"#, "M").unwrap();
        let variables = json!({});
        let mut p = params(&mapping, &[], &variables);
        p.file_type = FileType::Xlsx;
        let mut calls = 0;
        let error = fetch_all(&p, |_| {
            calls += 1;
            Ok(page(vec![json!({"id": 1})], 2_000_000))
        })
        .unwrap_err();
        assert_eq!(calls, 1);
        assert!(error.starts_with("2000000 records match"), "{error}");
    }

    #[test]
    fn a_null_or_missing_node_or_results_is_an_error() {
        for response in [
            r#"{"data": {"allPerson": null}}"#,
            r#"{"data": {"allPerson": {}}}"#,
            r#"{"data": {"allPerson": {"results": null, "totalCount": 3}}}"#,
            r#"{"data": {}}"#,
            r#"{"data": null}"#,
            r#"{}"#,
        ] {
            assert!(read_page(response, "allPerson").is_err(), "{response}");
        }
        assert_eq!(
            read_page(r#"{"data": {"allPerson": {"results": 5}}}"#, "allPerson").unwrap_err(),
            "The Data API results for `allPerson` are not a list"
        );
        assert!(
            read_page("not json", "allPerson")
                .unwrap_err()
                .starts_with("The Data API response is not valid JSON")
        );
    }

    #[test]
    fn an_empty_errors_list_is_not_an_error() {
        let response =
            r#"{"errors": [], "data": {"allPerson": {"results": [{"id": 1}], "totalCount": 1}}}"#;
        assert_eq!(
            read_page(response, "allPerson").unwrap(),
            (vec![json!({"id": 1})], Some(1))
        );
    }

    #[test]
    fn invalid_model_sort_order_or_sort_property_is_an_error() {
        let mapping = parse_map(r#"{"Id": "id"}"#, "M").unwrap();
        let variables = json!({});
        let mut p = params(&mapping, &[], &variables);
        p.model = "Person { x }";
        assert_eq!(
            build_query(&p, 0, 200).unwrap_err(),
            "Select the model to export records from"
        );
        let mut p = params(&mapping, &[], &variables);
        p.sort_order = "UP";
        assert!(
            build_query(&p, 0, 200)
                .unwrap_err()
                .starts_with("Unknown sort order")
        );
        let mut p = params(&mapping, &[], &variables);
        p.sort_field = Some("name } x");
        assert!(
            build_query(&p, 0, 200)
                .unwrap_err()
                .starts_with("The sort property")
        );
    }

    #[test]
    fn a_bad_format_fails_before_the_first_request() {
        let mapping = parse_map(r#"{"Born": "birth_date"}"#, "M").unwrap();
        let variables = json!({});
        for (formats, expected) in [
            (r#"{"Born": "Date|DD-MM-YYYY"}"#, "uses \"D\""),
            (
                r#"{"born": "Date|dd-MM-yyyy"}"#,
                "not a column of the Property mapping",
            ),
            (r#"{"Born": 5}"#, "must be text"),
            (r#"{"Born": null}"#, "must be text"),
            (r#"{"Born": "Date:dd"}"#, "has no |"),
            (r#"{"Born": "Dates|dd"}"#, "unknown kind"),
            (r#"{"Born": "Date|"}"#, "empty after the |"),
            (r#"{"": "Date|dd"}"#, "not a column of the Property mapping"),
        ] {
            let formats = parse_map(formats, "F").unwrap();
            let mut calls = 0;
            let error = fetch_all(&params(&mapping, &formats, &variables), |_| {
                calls += 1;
                Ok(page(vec![], 0))
            })
            .unwrap_err();
            assert_eq!(calls, 0, "{error}");
            assert!(error.contains(expected), "{error}");
        }
    }

    #[test]
    fn a_blank_format_mapping_row_is_skipped() {
        let mapping = parse_map(r#"{"Born": "birth_date"}"#, "M").unwrap();
        let formats = parse_map(r#"{"Born": "Date|dd-MM-yyyy", "": "", " ": null}"#, "F").unwrap();
        let variables = json!({});
        let p = params(&mapping, &formats, &variables);
        let records = vec![json!({"birthDate": "1990-05-17"})];
        assert_eq!(
            String::from_utf8(write_file(&p, &records).unwrap()).unwrap(),
            "Born\r\n\"17-05-1990\"\r\n"
        );
    }

    #[test]
    fn no_records_gives_a_file_with_only_the_header() {
        let mapping = parse_map(r#"{"Id": "id", "Name": "name"}"#, "M").unwrap();
        let variables = json!({});
        let p = params(&mapping, &[], &variables);
        let records = fetch_all(&p, |_| Ok(page(vec![], 0))).unwrap();
        assert_eq!(write_file(&p, &records).unwrap(), b"Id;Name\r\n");
    }

    #[test]
    fn errors_from_the_data_api_fail_the_step() {
        let mapping = parse_map(r#"{"Id": "idx"}"#, "M").unwrap();
        let variables = json!({});
        let p = params(&mapping, &[], &variables);
        let envelope = r#"{"errors": [{"message": "field 'idx' not found"}]}"#;
        assert_eq!(
            fetch_all(&p, |_| Err(envelope.to_string())).unwrap_err(),
            "field 'idx' not found"
        );
        assert_eq!(
            fetch_all(&p, |_| Ok(envelope.to_string())).unwrap_err(),
            "field 'idx' not found"
        );
    }

    #[test]
    fn rows_follow_the_mapping_with_formats_applied() {
        let mapping = parse_map(
            r#"{"Born": "birth_date", "Name": "name", "Amount": "amount", "Company": "company.name", "Active": "active"}"#,
            "M",
        )
        .unwrap();
        let formats =
            parse_map(r#"{"Born": "Date|dd-MM-yyyy", "Amount": "Decimal|,"}"#, "F").unwrap();
        let variables = json!({});
        let p = params(&mapping, &formats, &variables);
        let records = vec![
            json!({"birthDate": "1990-05-17", "name": "Ann", "amount": 12.5, "company": {"name": "Acme"}, "active": true}),
            json!({"birthDate": null, "name": "Bob", "amount": 3, "company": null, "active": false}),
        ];
        let text = String::from_utf8(write_file(&p, &records).unwrap()).unwrap();
        assert_eq!(
            text,
            "Born;Name;Amount;Company;Active\r\n\"17-05-1990\";\"Ann\";\"12,5\";\"Acme\";TRUE\r\n\"\";\"Bob\";\"3\";\"\";FALSE\r\n"
        );
    }

    #[test]
    fn a_to_many_relation_is_written_one_value_per_line() {
        let mapping = parse_map(
            r#"{"Name": "name", "Departments": "departments.description", "Since": "departments.since"}"#,
            "M",
        )
        .unwrap();
        let formats = parse_map(r#"{"Since": "Date|dd-MM-yyyy"}"#, "F").unwrap();
        let variables = json!({});
        let p = params(&mapping, &formats, &variables);
        let records = vec![
            json!({"name": "Ann", "departments": [
                {"description": "Sales, EMEA", "since": "2024-03-07"},
                {"description": "Say \"hi\"", "since": null}]}),
            json!({"name": "Bob", "departments": []}),
        ];
        let text = String::from_utf8(write_file(&p, &records).unwrap()).unwrap();
        assert_eq!(
            text,
            "Name;Departments;Since\r\n\"Ann\";\"Sales, EMEA\nSay \"\"hi\"\"\";\"07-03-2024\"\r\n\"Bob\";\"\";\"\"\r\n"
        );
    }

    #[test]
    fn excel_dates_are_numbers_and_csv_dates_stay_text() {
        let mapping = parse_map(r#"{"Born": "birth_date", "Seen": "seen_at"}"#, "M").unwrap();
        let formats = parse_map(r#"{"Born": "Date|dd-MM-yyyy", "Seen": "Date|QQQ"}"#, "F").unwrap();
        let variables = json!({});
        let mut p = params(&mapping, &formats, &variables);
        p.file_type = FileType::Xlsx;
        let records = vec![json!({"birthDate": "2024-03-07", "seenAt": "2024-03-07"})];
        let bytes = write_file(&p, &records).unwrap();
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
        let mut sheet = String::new();
        std::io::Read::read_to_string(
            &mut archive.by_name("xl/worksheets/sheet1.xml").unwrap(),
            &mut sheet,
        )
        .unwrap();
        // Born is a number cell; Seen (the quarter has no Excel format) stays text.
        assert!(sheet.contains("<v>45358</v>"), "{sheet}");
        assert!(sheet.contains(r#"<c r="B2" t="s">"#), "{sheet}");
    }

    #[test]
    fn file_names_get_one_extension() {
        assert_eq!(full_file_name(Some("report"), FileType::Csv), "report.csv");
        assert_eq!(
            full_file_name(Some("report.CSV"), FileType::Csv),
            "report.csv"
        );
        assert_eq!(
            full_file_name(Some("report.csv"), FileType::Xlsx),
            "report.csv.xlsx"
        );
        assert_eq!(full_file_name(None, FileType::Xlsx), "export.xlsx");
        assert_eq!(full_file_name(Some("  "), FileType::Csv), "export.csv");
        // Nothing but the extension, and a name that points into a folder.
        assert_eq!(full_file_name(Some(".csv"), FileType::Csv), "export.csv");
        assert_eq!(full_file_name(Some(" .CSV "), FileType::Csv), "export.csv");
        assert_eq!(full_file_name(Some("a/b\\c"), FileType::Csv), "a_b_c.csv");
        assert_eq!(full_file_name(Some("../x"), FileType::Csv), ".._x.csv");
    }
}
