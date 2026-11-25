//! Writing the rows as a CSV file (like the `export-to-csv` 0.2 package the ActionsJS step used)
//! or as an XLSX file (like SheetJS `json_to_sheet`).

use rust_xlsxwriter::Workbook;
use serde_json::{Number, Value};

/// Close to JavaScript's `String(number)` for a JSON number: `1.0` prints as `1`, `1e21` as
/// `1e+21`. It differs for floats below 1e-6 (`1e-7` prints as `0.0000001`, JavaScript `1e-7`)
/// and for `-0.0` (`-0`, JavaScript `0`).
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

/// Whether a spreadsheet program could read the text as a formula: it starts with `=`, `+`, `-`,
/// `@`, a tab or a carriage return (the OWASP list). Text that is only a number, like `-12,5` from
/// a Decimal format, cannot run anything and is left alone.
fn looks_like_formula(text: &str) -> bool {
    if !text.starts_with(['=', '+', '-', '@', '\t', '\r']) {
        return false;
    }
    // A sign followed by only digits, spaces, `.` and `,` (and maybe a closing `%`) has no
    // operator, reference or function in it: `-1,234.56`, `+31 6 1234 5678` and `-5%` cannot run.
    let digits = text.strip_prefix(['+', '-']).unwrap_or(text);
    let digits = digits.strip_suffix('%').unwrap_or(digits);
    let is_number = digits.starts_with(|c: char| c.is_ascii_digit())
        && digits
            .chars()
            .all(|c| c.is_ascii_digit() || matches!(c, '.' | ',' | ' '));
    !is_number
}

/// `export-to-csv` 0.2: text is put in double quotes with every `"` doubled, `true`/`false` print
/// as `TRUE`/`FALSE`, numbers as they are. A missing value is empty text, so `""`. With
/// `escape_formulas`, text that could be read as a formula gets a `'` in front, so Excel shows it
/// as text instead of running it.
fn csv_value(value: Option<&Value>, escape_formulas: bool) -> String {
    let quote = |text: &str| format!("\"{}\"", text.replace('"', "\"\""));
    match value {
        None | Some(Value::Null) => quote(""),
        Some(Value::String(text)) if escape_formulas && looks_like_formula(text) => {
            quote(&format!("'{text}"))
        }
        Some(Value::String(text)) => quote(text),
        Some(Value::Bool(true)) => String::from("TRUE"),
        Some(Value::Bool(false)) => String::from("FALSE"),
        Some(Value::Number(number)) => number_to_string(number),
        Some(other) => quote(&other_to_string(other)),
    }
}

/// A header is written as it is, like `export-to-csv`, unless it would break the file (it holds
/// the delimiter, a quote or a line break): then it is quoted. With `escape_formulas` a header
/// that could run as a formula gets a `'` in front, like a value.
fn csv_header(header: &str, delimiter: &str, escape_formulas: bool) -> String {
    let escaped;
    let header = if escape_formulas && looks_like_formula(header) {
        escaped = format!("'{header}");
        escaped.as_str()
    } else {
        header
    };
    if header.contains(delimiter) || header.contains(['"', '\n', '\r']) {
        format!("\"{}\"", header.replace('"', "\"\""))
    } else {
        header.to_string()
    }
}

/// The CSV text: an optional BOM, the header row, then one line per row, each ending in `\r\n`.
pub fn csv(
    headers: &[String],
    rows: &[Vec<Option<Value>>],
    delimiter: &str,
    bom: bool,
    escape_formulas: bool,
) -> String {
    let mut out = String::new();
    if bom {
        out.push('\u{feff}');
    }
    let header_line: Vec<String> = headers
        .iter()
        .map(|h| csv_header(h, delimiter, escape_formulas))
        .collect();
    out.push_str(&header_line.join(delimiter));
    out.push_str("\r\n");
    for row in rows {
        let line: Vec<String> = row
            .iter()
            .map(|value| csv_value(value.as_ref(), escape_formulas))
            .collect();
        out.push_str(&line.join(delimiter));
        out.push_str("\r\n");
    }
    out
}

/// An Excel sheet name: at most 31 characters (Excel counts UTF-16 units, so an emoji counts as
/// two), none of `[]:*?/\`.
fn sheet_name(name: &str) -> String {
    let mut cleaned = String::new();
    let mut units = 0;
    for c in name
        .chars()
        .filter(|c| !matches!(c, '[' | ']' | ':' | '*' | '?' | '/' | '\\'))
    {
        units += c.len_utf16();
        if units > 31 {
            break;
        }
        cleaned.push(c);
    }
    let cleaned = cleaned.trim_matches('\'').to_string();
    if cleaned.is_empty() {
        String::from("Sheet1")
    } else {
        cleaned
    }
}

/// The longest text an Excel cell holds.
const XLSX_MAX_CELL_CHARS: usize = 32_767;

/// Integers beyond 2^53 cannot be stored exactly as an Excel number.
fn is_exact_in_excel(number: &Number) -> bool {
    const LIMIT: u64 = 1 << 53;
    match (number.as_i64(), number.as_u64()) {
        (Some(integer), _) => integer.unsigned_abs() <= LIMIT,
        (None, Some(integer)) => integer <= LIMIT,
        _ => true,
    }
}

/// Excel refuses text over 32,767 characters: name the column and record instead of the
/// writer's generic error.
fn check_cell_length(text: &str, header: &str, index: usize) -> Result<(), String> {
    let length = text.encode_utf16().count();
    if length > XLSX_MAX_CELL_CHARS {
        return Err(format!(
            "The value in column \"{header}\" of record {} has {length} characters, more than an \
             Excel cell holds (at most 32,767): export to CSV instead",
            index + 1
        ));
    }
    Ok(())
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
                    check_cell_length(text, &headers[usize::from(column)], index)?;
                    worksheet
                        .write_string(row_number, column, text)
                        .map_err(error)?;
                }
                Some(Value::Bool(flag)) => {
                    worksheet
                        .write_boolean(row_number, column, *flag)
                        .map_err(error)?;
                }
                Some(Value::Number(number)) if is_exact_in_excel(number) => {
                    worksheet
                        .write_number(row_number, column, number.as_f64().unwrap_or(0.0))
                        .map_err(error)?;
                }
                // Excel stores numbers as doubles: a larger integer would be rounded, so it is
                // written as text with all its digits.
                Some(Value::Number(number)) => {
                    worksheet
                        .write_string(row_number, column, number.to_string())
                        .map_err(error)?;
                }
                Some(other) => {
                    let text = other_to_string(other);
                    check_cell_length(&text, &headers[usize::from(column)], index)?;
                    worksheet
                        .write_string(row_number, column, text)
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
            csv(&headers, &rows, ";", true, true),
            "\u{feff}Id;Name;Active\r\n1;\"Say \"\"hi\"\"\";TRUE\r\n2.5;\"\";FALSE\r\n"
        );
    }

    #[test]
    fn a_header_that_would_break_the_file_is_quoted() {
        let headers = vec!["a,b".to_string(), "c\"d".to_string(), "e;f".to_string()];
        assert_eq!(
            csv(&headers, &[], ",", false, true),
            "\"a,b\",\"c\"\"d\",e;f\r\n"
        );
    }

    #[test]
    fn lists_and_objects_are_written_as_json() {
        let rows = vec![vec![Some(json!([{"id": 1}]))]];
        assert_eq!(
            csv(&["L".into()], &rows, ",", false, true),
            "L\r\n\"[{\"\"id\"\":1}]\"\r\n"
        );
    }

    #[test]
    fn text_that_could_run_as_a_formula_gets_a_quote_in_front() {
        let cells = [
            "=HYPERLINK(\"http://x\")",
            "+1+cmd",
            "-2+3",
            "@SUM(A1)",
            "\tx",
            "-",
        ];
        let rows: Vec<Vec<Option<Value>>> =
            cells.iter().map(|cell| vec![Some(json!(cell))]).collect();
        assert_eq!(
            csv(&["C".into()], &rows, ";", false, true),
            "C\r\n\"'=HYPERLINK(\"\"http://x\"\")\"\r\n\"'+1+cmd\"\r\n\"'-2+3\"\r\n\"'@SUM(A1)\"\r\n\"'\tx\"\r\n\"'-\"\r\n"
        );
        assert_eq!(
            csv(&["C".into()], &rows[..1], ";", false, false),
            "C\r\n\"=HYPERLINK(\"\"http://x\"\")\"\r\n"
        );
    }

    #[test]
    fn numbers_and_plain_text_are_not_escaped() {
        let rows: Vec<Vec<Option<Value>>> = [
            json!("-12,5"),
            json!("+31"),
            json!("-0.5"),
            json!(-3),
            json!("a=b"),
            json!("+31 6 1234 5678"),
            json!("-1,234.56"),
            json!("-5%"),
        ]
        .into_iter()
        .map(|value| vec![Some(value)])
        .collect();
        assert_eq!(
            csv(&["C".into()], &rows, ";", false, true),
            "C\r\n\"-12,5\"\r\n\"+31\"\r\n\"-0.5\"\r\n-3\r\n\"a=b\"\r\n\"+31 6 1234 5678\"\r\n\"-1,234.56\"\r\n\"-5%\"\r\n"
        );
        // Not a number once something else follows the digits.
        for text in ["-1+1", "-5%x", "+1 A1", "- none -", "-%"] {
            assert!(looks_like_formula(text), "{text}");
        }
    }

    #[test]
    fn a_header_that_could_run_as_a_formula_is_escaped_too() {
        let headers = vec!["=1+1".to_string(), "-Total".to_string(), "Name".to_string()];
        assert_eq!(
            csv(&headers, &[], ";", false, true),
            "'=1+1;'-Total;Name\r\n"
        );
        assert_eq!(
            csv(&headers, &[], ";", false, false),
            "=1+1;-Total;Name\r\n"
        );
    }

    #[test]
    fn sheet_names_are_made_valid() {
        assert_eq!(sheet_name("Export"), "Export");
        assert_eq!(sheet_name("a/b:c"), "abc");
        assert_eq!(sheet_name(&"x".repeat(40)).len(), 31);
        assert_eq!(sheet_name("[]"), "Sheet1");
        assert_eq!(sheet_name(&"😀".repeat(20)), "😀".repeat(15));
    }

    #[test]
    fn integers_excel_would_round_are_kept_exact() {
        let exact = |v: Value| is_exact_in_excel(v.as_number().unwrap());
        assert!(exact(json!(9_007_199_254_740_992_u64)));
        assert!(exact(json!(-9_007_199_254_740_992_i64)));
        assert!(exact(json!(1.5e300)));
        assert!(!exact(json!(9_007_199_254_740_993_u64)));
        assert!(!exact(json!(u64::MAX)));
        assert!(!exact(json!(i64::MIN)));
    }

    #[test]
    fn text_too_long_for_an_excel_cell_names_the_column() {
        let rows = vec![vec![Some(json!("x".repeat(32_768)))]];
        let error = xlsx(&["Notes".into()], &rows, "Export").unwrap_err();
        assert!(
            error.starts_with("The value in column \"Notes\" of record 1 has 32768 characters"),
            "{error}"
        );
        let rows = vec![vec![Some(json!("x".repeat(32_767)))]];
        assert!(xlsx(&["Notes".into()], &rows, "Export").is_ok());
    }

    /// The text of one file inside the XLSX zip.
    fn part(bytes: &[u8], name: &str) -> String {
        use std::io::Read;
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
        let mut text = String::new();
        archive
            .by_name(name)
            .unwrap()
            .read_to_string(&mut text)
            .unwrap();
        text
    }

    #[test]
    fn xlsx_cells_have_the_right_types() {
        let headers: Vec<String> = ["N", "T", "B", "E", "Big"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let rows = vec![
            vec![
                Some(json!(1.5)),
                Some(json!("Ann")),
                Some(json!(true)),
                None,
                Some(json!(9_007_199_254_740_993_u64)),
            ],
            vec![
                Some(json!(-2)),
                Some(json!("")),
                Some(json!(false)),
                Some(Value::Null),
                Some(json!(1)),
            ],
        ];
        let bytes = xlsx(&headers, &rows, "People").unwrap();
        let sheet = part(&bytes, "xl/worksheets/sheet1.xml");
        let strings = part(&bytes, "xl/sharedStrings.xml");
        let workbook = part(&bytes, "xl/workbook.xml");

        assert!(workbook.contains(r#"name="People""#), "{workbook}");
        // Three rows: the header and two records.
        assert_eq!(sheet.matches("<row ").count(), 3, "{sheet}");
        assert!(sheet.contains(r#"<c r="A2"><v>1.5</v></c>"#), "{sheet}");
        assert!(sheet.contains(r#"<c r="A3"><v>-2</v></c>"#), "{sheet}");
        assert!(sheet.contains(r#"<c r="C2" t="b"><v>1</v></c>"#), "{sheet}");
        assert!(sheet.contains(r#"<c r="C3" t="b"><v>0</v></c>"#), "{sheet}");
        // Empty values and empty text leave no cell.
        for cell in ["D2", "B3", "D3"] {
            assert!(
                !sheet.contains(&format!(r#"r="{cell}""#)),
                "{cell}: {sheet}"
            );
        }
        // Text, and the integer Excel would round, are shared strings.
        assert!(sheet.contains(r#"<c r="B2" t="s">"#), "{sheet}");
        assert!(sheet.contains(r#"<c r="E2" t="s">"#), "{sheet}");
        assert!(sheet.contains(r#"<c r="E3"><v>1</v></c>"#), "{sheet}");
        assert!(strings.contains("<t>Ann</t>"), "{strings}");
        assert!(strings.contains("<t>9007199254740993</t>"), "{strings}");
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
