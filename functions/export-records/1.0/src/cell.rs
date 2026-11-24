//! Writing the rows as a CSV file (like the `export-to-csv` 0.2 package the ActionsJS step used)
//! or as an XLSX file (like SheetJS `json_to_sheet`).

use rust_xlsxwriter::Workbook;
use serde_json::{Number, Value};

/// JavaScript's `String(number)` for a JSON number: `1.0` prints as `1`, `1e21` as `1e+21`.
pub fn number_to_string(number: &Number) -> String {
    if number.is_i64() || number.is_u64() {
        return number.to_string();
    }
    let float = number.as_f64().unwrap_or(0.0);
    if float.fract() == 0.0 && float.abs() < 1e21 {
        return format!("{float:.0}");
    }
    let text = float.to_string();
    if float.abs() >= 1e21 {
        // Rust prints 1e21 as 1000000000000000000000; JavaScript as 1e+21.
        let formatted = format!("{float:e}");
        return match formatted.split_once('e') {
            Some((mantissa, exponent)) if !exponent.starts_with('-') => {
                format!("{mantissa}e+{exponent}")
            }
            _ => formatted,
        };
    }
    text
}

/// A value that is not text, a number or a boolean (a list or an object, e.g. a has-many
/// relation) is written as JSON. (The ActionsJS step wrote `[object Object]`.)
fn other_to_string(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_default()
}

/// `export-to-csv` 0.2: text is put in double quotes with every `"` doubled, `true`/`false` print
/// as `TRUE`/`FALSE`, numbers as they are. A missing value is empty text, so `""`.
fn csv_value(value: Option<&Value>) -> String {
    let quote = |text: &str| format!("\"{}\"", text.replace('"', "\"\""));
    match value {
        None | Some(Value::Null) => quote(""),
        Some(Value::String(text)) => quote(text),
        Some(Value::Bool(true)) => String::from("TRUE"),
        Some(Value::Bool(false)) => String::from("FALSE"),
        Some(Value::Number(number)) => number_to_string(number),
        Some(other) => quote(&other_to_string(other)),
    }
}

/// A header is written as it is, like `export-to-csv`, unless it would break the file (it holds
/// the delimiter, a quote or a line break): then it is quoted.
fn csv_header(header: &str, delimiter: &str) -> String {
    if header.contains(delimiter) || header.contains(['"', '\n', '\r']) {
        format!("\"{}\"", header.replace('"', "\"\""))
    } else {
        header.to_string()
    }
}

/// The CSV text: an optional BOM, the header row, then one line per row, each ending in `\r\n`.
pub fn csv(headers: &[String], rows: &[Vec<Option<Value>>], delimiter: &str, bom: bool) -> String {
    let mut out = String::new();
    if bom {
        out.push('\u{feff}');
    }
    let header_line: Vec<String> = headers.iter().map(|h| csv_header(h, delimiter)).collect();
    out.push_str(&header_line.join(delimiter));
    out.push_str("\r\n");
    for row in rows {
        let line: Vec<String> = row.iter().map(|value| csv_value(value.as_ref())).collect();
        out.push_str(&line.join(delimiter));
        out.push_str("\r\n");
    }
    out
}

/// An Excel sheet name: at most 31 characters, none of `[]:*?/\`.
fn sheet_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .filter(|c| !matches!(c, '[' | ']' | ':' | '*' | '?' | '/' | '\\'))
        .take(31)
        .collect();
    let cleaned = cleaned.trim_matches('\'').to_string();
    if cleaned.is_empty() {
        String::from("Sheet1")
    } else {
        cleaned
    }
}

/// The XLSX file: one sheet with the header row and the rows below it. Numbers are number cells,
/// booleans boolean cells and everything else text (an empty value leaves the cell empty).
pub fn xlsx(
    headers: &[String],
    rows: &[Vec<Option<Value>>],
    sheet: &str,
) -> Result<Vec<u8>, String> {
    let error =
        |error: rust_xlsxwriter::XlsxError| format!("Could not write the Excel file: {error}");
    let mut workbook = Workbook::new();
    let worksheet = workbook.add_worksheet();
    worksheet.set_name(sheet_name(sheet)).map_err(error)?;

    for (column, header) in headers.iter().enumerate() {
        let column = u16::try_from(column).map_err(|_| "Too many columns for an Excel file")?;
        worksheet.write_string(0, column, header).map_err(error)?;
    }
    for (index, row) in rows.iter().enumerate() {
        let row_number = u32::try_from(index + 1)
            .ok()
            .filter(|&n| n < 1_048_576)
            .ok_or("Too many records for an Excel sheet (at most 1,048,575)")?;
        for (column, value) in row.iter().enumerate() {
            let column = column as u16;
            match value {
                None | Some(Value::Null) => {}
                Some(Value::String(text)) if text.is_empty() => {}
                Some(Value::String(text)) => {
                    worksheet
                        .write_string(row_number, column, text)
                        .map_err(error)?;
                }
                Some(Value::Bool(flag)) => {
                    worksheet
                        .write_boolean(row_number, column, *flag)
                        .map_err(error)?;
                }
                Some(Value::Number(number)) => {
                    worksheet
                        .write_number(row_number, column, number.as_f64().unwrap_or(0.0))
                        .map_err(error)?;
                }
                Some(other) => {
                    worksheet
                        .write_string(row_number, column, other_to_string(other))
                        .map_err(error)?;
                }
            }
        }
    }
    workbook.save_to_buffer().map_err(error)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn numbers_print_like_javascript() {
        let print = |v: Value| number_to_string(v.as_number().unwrap());
        assert_eq!(print(json!(1)), "1");
        assert_eq!(print(json!(1.0)), "1");
        assert_eq!(print(json!(-2.5)), "-2.5");
        assert_eq!(print(json!(0.1)), "0.1");
        assert_eq!(print(json!(1e21)), "1e+21");
    }

    #[test]
    fn csv_matches_export_to_csv() {
        let headers = vec!["Id".to_string(), "Name".to_string(), "Active".to_string()];
        let rows = vec![
            vec![Some(json!(1)), Some(json!("Say \"hi\"")), Some(json!(true))],
            vec![Some(json!(2.5)), None, Some(json!(false))],
        ];
        assert_eq!(
            csv(&headers, &rows, ";", true),
            "\u{feff}Id;Name;Active\r\n1;\"Say \"\"hi\"\"\";TRUE\r\n2.5;\"\";FALSE\r\n"
        );
    }

    #[test]
    fn a_header_that_would_break_the_file_is_quoted() {
        let headers = vec!["a,b".to_string(), "c\"d".to_string(), "e;f".to_string()];
        assert_eq!(csv(&headers, &[], ",", false), "\"a,b\",\"c\"\"d\",e;f\r\n");
    }

    #[test]
    fn lists_and_objects_are_written_as_json() {
        let rows = vec![vec![Some(json!([{"id": 1}]))]];
        assert_eq!(
            csv(&["L".into()], &rows, ",", false),
            "L\r\n\"[{\"\"id\"\":1}]\"\r\n"
        );
    }

    #[test]
    fn sheet_names_are_made_valid() {
        assert_eq!(sheet_name("Export"), "Export");
        assert_eq!(sheet_name("a/b:c"), "abc");
        assert_eq!(sheet_name(&"x".repeat(40)).len(), 31);
        assert_eq!(sheet_name("[]"), "Sheet1");
    }

    #[test]
    fn xlsx_is_a_zip_file() {
        let rows = vec![vec![
            Some(json!(1)),
            Some(json!("a")),
            Some(json!(true)),
            None,
        ]];
        let headers: Vec<String> = ["N", "T", "B", "E"].iter().map(|s| s.to_string()).collect();
        let bytes = xlsx(&headers, &rows, "Export").unwrap();
        assert_eq!(&bytes[..2], b"PK");
    }
}
