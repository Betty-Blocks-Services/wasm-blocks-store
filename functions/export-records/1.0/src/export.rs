//! Fetches every record that matches the filter, page by page, and writes them as a CSV or XLSX
//! file.

use serde_json::Value;

use crate::cell;
use crate::filter;
use crate::format::{self, Format};
use crate::mapping::{self, Column};

/// The most records the Data API returns per request.
pub const PAGE_SIZE: usize = 200;

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
}

/// The query for one page.
pub fn build_query(params: &Params, skip: usize) -> Result<(String, String), String> {
    let model = params.model.trim();
    if !mapping::is_name(model) {
        return Err(String::from("Select the model to export records from"));
    }
    let columns = mapping::columns(params.mapping)?;
    let selection = mapping::selection(&columns);

    let filter = filter::build(params.filter, params.variables)?;
    let order = match params.sort_order.trim().to_uppercase().as_str() {
        "" | "ASC" => "ASC",
        "DESC" => "DESC",
        other => return Err(format!("Unknown sort order \"{other}\": use ASC or DESC")),
    };
    // Pages are separate requests: without a sort order a record could be exported twice or not
    // at all, so the records are sorted by id when no sort property is chosen.
    let sort_field = match params.sort_field.map(str::trim).filter(|f| !f.is_empty()) {
        Some(field) => {
            let field = mapping::snake_to_camel(field);
            if !mapping::is_name(&field) {
                return Err(format!("The sort property \"{field}\" is not valid"));
            }
            field
        }
        None => String::from("id"),
    };

    let field = format!("all{model}");
    let query = format!(
        "query {{ {field}(where: {{ {filter}\n}}, sort: {{ field: {sort_field}, order: {order} }}, skip: {skip}, take: {PAGE_SIZE}) {{ results {{ {selection} }} totalCount }} }}"
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
        .ok_or_else(|| format!("The Data API response has no data for `{field}`"))?;
    let total = node.get("totalCount").and_then(Value::as_u64);
    let records = match node.get_mut("results").map(Value::take) {
        Some(Value::Array(records)) => records,
        Some(Value::Null) | None => Vec::new(),
        Some(_) => return Err(String::from("The Data API results are not a list")),
    };
    Ok((records, total))
}

/// Fetches every matching record with `send`, a page of `PAGE_SIZE` at a time.
pub fn fetch_all(
    params: &Params,
    mut send: impl FnMut(&str) -> Result<String, String>,
) -> Result<Vec<Value>, String> {
    let mut records = Vec::new();
    loop {
        let (query, field) = build_query(params, records.len())?;
        let response = send(&query).map_err(|error| host_error_message(&error))?;
        let (page, total) = read_page(&response, &field)?;
        let full = page.len() == PAGE_SIZE;
        records.extend(page);
        let done = total.is_some_and(|total| records.len() as u64 >= total);
        if !full || done {
            return Ok(records);
        }
    }
}

/// The file's bytes: the header row, then one row per record with the formats applied.
pub fn write_file(params: &Params, records: &[Value]) -> Result<Vec<u8>, String> {
    let columns: Vec<Column> = mapping::columns(params.mapping)?;
    let headers: Vec<String> = columns.iter().map(|c| c.header.clone()).collect();

    let mut formats: Vec<Option<Format>> = Vec::with_capacity(columns.len());
    for column in &columns {
        let spec = params
            .formats
            .iter()
            .find(|(key, _)| *key == column.header)
            .and_then(|(_, value)| value.as_str());
        formats.push(match spec {
            Some(spec) => format::parse(&column.header, spec)?,
            None => None,
        });
    }

    let mut rows = Vec::with_capacity(records.len());
    for record in records {
        let mut row = Vec::with_capacity(columns.len());
        for (column, format) in columns.iter().zip(&formats) {
            row.push(match mapping::lookup(record, &column.path) {
                Some(value) => Some(format::apply(&column.header, value, format.as_ref())?),
                None => None,
            });
        }
        rows.push(row);
    }

    match params.file_type {
        FileType::Csv => {
            let delimiter = if params.delimiter.is_empty() {
                ","
            } else {
                params.delimiter
            };
            Ok(cell::csv(&headers, &rows, delimiter, params.bom).into_bytes())
        }
        FileType::Xlsx => cell::xlsx(&headers, &rows, params.sheet),
    }
}

/// The file name with its extension, without a doubled extension when the name already has it.
pub fn full_file_name(name: Option<&str>, file_type: FileType) -> String {
    let extension = file_type.extension();
    let name = name
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .unwrap_or("export");
    let suffix = format!(".{extension}");
    let base = if name.to_lowercase().ends_with(&suffix) {
        &name[..name.len() - suffix.len()]
    } else {
        name
    };
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
        let (query, field) = build_query(&p, 400).unwrap();
        assert_eq!(field, "allPerson");
        assert_eq!(
            query,
            "query { allPerson(where: { type: { eq: \"x\" }\n}, sort: { field: lastName, order: DESC }, skip: 400, take: 200) { results { firstName company { name } } totalCount } }"
        );
    }

    #[test]
    fn without_a_sort_property_the_records_are_sorted_by_id() {
        let mapping = parse_map(r#"{"Id": "id"}"#, "M").unwrap();
        let (query, _) = build_query(&params(&mapping, &[], &json!({})), 0).unwrap();
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
            let count = (450 - skip).min(PAGE_SIZE);
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
    }
}
