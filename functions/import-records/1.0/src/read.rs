//! Reads the downloaded file into a header row and data rows of cells.

use std::io::Cursor;

use calamine::{Data, Reader};
use chrono::{NaiveDate, NaiveDateTime, NaiveTime};

/// One cell of the file. A CSV file only gives `Empty` and `Text`; an Excel file keeps the type of
/// the cell.
#[derive(Debug, Clone, PartialEq)]
pub enum Cell {
    Empty,
    Text(String),
    Integer(i64),
    Number(f64),
    Bool(bool),
    DateTime(NaiveDateTime),
    /// An Excel error value such as `#N/A`.
    Error(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    /// The row number in the file (1 is the header row), for error messages.
    pub number: usize,
    pub cells: Vec<Cell>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Table {
    /// The column names, trimmed.
    pub headers: Vec<String>,
    /// The data rows. Rows with only empty cells are left out.
    pub rows: Vec<Row>,
}

impl Table {
    /// The index of a column by its name. The first column with that name wins.
    pub fn column(&self, name: &str) -> Option<usize> {
        self.headers.iter().position(|header| header == name)
    }
}

/// The start of a zip file (xlsx, ods) and of an old binary Office file (xls).
fn looks_like_excel(bytes: &[u8]) -> bool {
    bytes.starts_with(b"PK\x03\x04") || bytes.starts_with(&[0xD0, 0xCF, 0x11, 0xE0])
}

/// The text of a CSV file: UTF-8, or UTF-16 with a byte order mark. A file that is not valid UTF-8
/// is read as Windows-1252, the encoding Excel uses for "CSV (comma delimited)" on Windows.
fn decode(bytes: &[u8]) -> String {
    if let Some((encoding, bom_length)) = encoding_rs::Encoding::for_bom(bytes) {
        let (text, _) = encoding.decode_without_bom_handling(&bytes[bom_length..]);
        return text.into_owned();
    }
    match std::str::from_utf8(bytes) {
        Ok(text) => text.to_string(),
        Err(_) => {
            let (text, _) = encoding_rs::WINDOWS_1252.decode_without_bom_handling(bytes);
            text.into_owned()
        }
    }
}

/// The 1-based line number of a byte offset (`\r\n`, `\n` and a lone `\r` each end a line).
fn line_at(text: &str, offset: usize) -> usize {
    let before = &text.as_bytes()[..offset.min(text.len())];
    let lone_returns = before
        .iter()
        .enumerate()
        .filter(|(index, byte)| **byte == b'\r' && before.get(index + 1) != Some(&b'\n'))
        .count();
    before.iter().filter(|byte| **byte == b'\n').count() + lone_returns + 1
}

const DELIMITERS: [u8; 4] = [b',', b';', b'\t', b'|'];

/// The delimiter that occurs most often (outside quotes) in the first line, `,` when there is
/// none. On a tie the earlier one in `,` `;` tab `|` wins.
pub fn detect_delimiter(text: &str) -> u8 {
    let mut counts = [0usize; DELIMITERS.len()];
    let mut in_quotes = false;
    // The first line with text: blank lines at the start are skipped.
    let start = text.trim_start_matches(['\r', '\n']);
    for byte in start.bytes() {
        match byte {
            b'"' => in_quotes = !in_quotes,
            b'\n' | b'\r' if !in_quotes => break,
            _ if !in_quotes => {
                if let Some(index) = DELIMITERS.iter().position(|&d| d == byte) {
                    counts[index] += 1;
                }
            }
            _ => {}
        }
    }
    let mut best = 0;
    for index in 1..DELIMITERS.len() {
        if counts[index] > counts[best] {
            best = index;
        }
    }
    DELIMITERS[best]
}

/// Reads a CSV file. `delimiter` is `None` for automatic detection.
pub fn read_csv(bytes: &[u8], delimiter: Option<u8>) -> Result<Table, String> {
    if looks_like_excel(bytes) {
        return Err(String::from(
            "The import file is an Excel file, not a CSV file: set Import file type to Excel",
        ));
    }
    let text = decode(bytes);
    let delimiter = delimiter.unwrap_or_else(|| detect_delimiter(&text));
    let mut reader = csv::ReaderBuilder::new()
        .delimiter(delimiter)
        .has_headers(false)
        .flexible(true)
        .from_reader(text.as_bytes());

    let mut headers = None;
    let mut rows = Vec::new();
    for record in reader.records() {
        let record = record.map_err(|error| {
            let line = error
                .position()
                .map(|position| format!(" at line {}", line_at(&text, position.byte() as usize)))
                .unwrap_or_default();
            format!("The CSV file could not be read{line}: {error}")
        })?;
        // The csv crate's own line count skips blank lines and miscounts `\r\n`, so the row
        // number is counted from the byte offset of the record.
        let number = record
            .position()
            .map(|position| line_at(&text, position.byte() as usize))
            .unwrap_or(0);
        if record.iter().all(|field| field.trim().is_empty()) {
            continue;
        }
        if headers.is_none() {
            headers = Some(
                record
                    .iter()
                    .map(|field| field.trim().to_string())
                    .collect(),
            );
            continue;
        }
        let cells = record
            .iter()
            .map(|field| {
                if field.is_empty() {
                    Cell::Empty
                } else {
                    Cell::Text(field.to_string())
                }
            })
            .collect();
        rows.push(Row { number, cells });
    }

    let headers = headers.ok_or("The import file is empty: it has no header row")?;
    Ok(Table { headers, rows })
}

/// Reads the first sheet of an Excel (xlsx, xlsm, xls) or OpenDocument (ods) file. The first row
/// with a value is the header row.
pub fn read_excel(bytes: Vec<u8>) -> Result<Table, String> {
    let mut workbook = calamine::open_workbook_auto_from_rs(Cursor::new(bytes)).map_err(|error| {
        format!(
            "The import file could not be read as an Excel file ({error}): check that it is an xlsx, xls or ods file, or set Import file type to CSV"
        )
    })?;
    let range = workbook
        .worksheet_range_at(0)
        .ok_or("The Excel file has no sheets")?
        .map_err(|error| format!("The first sheet of the Excel file could not be read: {error}"))?;
    let first_row = range.start().map(|(row, _)| row as usize).unwrap_or(0);

    let mut headers = None;
    let mut rows = Vec::new();
    for (index, cells) in range.rows().enumerate() {
        let cells: Vec<Cell> = cells.iter().map(cell_of).collect();
        if cells.iter().all(|cell| *cell == Cell::Empty) {
            continue;
        }
        if headers.is_none() {
            headers = Some(cells.iter().map(header_text).collect());
            continue;
        }
        rows.push(Row {
            number: first_row + index + 1,
            cells,
        });
    }

    let headers =
        headers.ok_or("The first sheet of the Excel file is empty: it has no header row")?;
    Ok(Table { headers, rows })
}

fn cell_of(data: &Data) -> Cell {
    match data {
        Data::Empty => Cell::Empty,
        Data::String(text) if text.is_empty() => Cell::Empty,
        Data::String(text) => Cell::Text(text.clone()),
        Data::Int(number) => Cell::Integer(*number),
        Data::Float(number) => Cell::Number(*number),
        Data::Bool(value) => Cell::Bool(*value),
        Data::DateTime(value) => match value.as_datetime() {
            Some(datetime) => Cell::DateTime(datetime),
            None => Cell::Number(value.as_f64()),
        },
        Data::DateTimeIso(text) => match parse_iso(text) {
            Some(datetime) => Cell::DateTime(datetime),
            None => Cell::Text(text.clone()),
        },
        Data::DurationIso(text) => Cell::Text(text.clone()),
        Data::Error(error) => Cell::Error(error.to_string()),
    }
}

fn parse_iso(text: &str) -> Option<NaiveDateTime> {
    NaiveDateTime::parse_from_str(text, "%Y-%m-%dT%H:%M:%S%.f")
        .ok()
        .or_else(|| {
            NaiveDate::parse_from_str(text, "%Y-%m-%d")
                .ok()
                .map(|date| date.and_time(NaiveTime::MIN))
        })
        .or_else(|| {
            NaiveTime::parse_from_str(text, "%H:%M:%S%.f")
                .ok()
                .map(|time| {
                    NaiveDate::from_ymd_opt(1899, 12, 30)
                        .unwrap()
                        .and_time(time)
                })
        })
}

/// The text of a header cell, trimmed.
fn header_text(cell: &Cell) -> String {
    match cell {
        Cell::Empty => String::new(),
        Cell::Text(text) | Cell::Error(text) => text.trim().to_string(),
        Cell::Integer(number) => number.to_string(),
        Cell::Number(number) => number_text(*number),
        Cell::Bool(value) => value.to_string(),
        Cell::DateTime(datetime) => datetime_text(datetime),
    }
}

/// A number as JavaScript writes it in a template string: `12` for 12.0, `12.5` for 12.5.
pub fn number_text(number: f64) -> String {
    if number.fract() == 0.0 && number.abs() < 1e15 {
        format!("{}", number as i64)
    } else {
        format!("{number}")
    }
}

/// `2024-01-31` when the time is midnight, else `2024-01-31 13:45:00`.
pub fn datetime_text(datetime: &NaiveDateTime) -> String {
    if datetime.time() == NaiveTime::MIN {
        datetime.format("%Y-%m-%d").to_string()
    } else {
        datetime.format("%Y-%m-%d %H:%M:%S").to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(value: &str) -> Cell {
        Cell::Text(value.into())
    }

    #[test]
    fn the_delimiter_is_the_most_common_one_in_the_first_line() {
        assert_eq!(detect_delimiter("a;b;c\n1,2,3,4,5"), b';');
        assert_eq!(detect_delimiter("a,b\n"), b',');
        assert_eq!(detect_delimiter("a\tb\tc"), b'\t');
        assert_eq!(detect_delimiter("\"x;y;z\",b\n"), b',');
        assert_eq!(detect_delimiter("single"), b',');
        assert_eq!(detect_delimiter("a|b;c"), b';');
    }

    #[test]
    fn a_csv_file_is_read_with_trimmed_headers_and_row_numbers() {
        let table = read_csv(
            "\u{feff} Name ;Age\r\nAnn;30\r\n\r\n;\r\n\"B;o\"\"b\";\r\nCarl\n".as_bytes(),
            None,
        )
        .unwrap();
        assert_eq!(table.headers, ["Name", "Age"]);
        assert_eq!(
            table.rows,
            vec![
                Row {
                    number: 2,
                    cells: vec![text("Ann"), text("30")]
                },
                Row {
                    number: 5,
                    cells: vec![text("B;o\"b"), Cell::Empty]
                },
                Row {
                    number: 6,
                    cells: vec![text("Carl")]
                },
            ]
        );
    }

    #[test]
    fn a_chosen_delimiter_is_used() {
        let table = read_csv(b"a,b;c\n1,2;3", Some(b';')).unwrap();
        assert_eq!(table.headers, ["a,b", "c"]);
    }

    #[test]
    fn windows_1252_and_utf_16_files_are_decoded() {
        let table = read_csv(b"Caf\xe9\nna\xefve", None).unwrap();
        assert_eq!(table.headers, ["Café"]);
        assert_eq!(table.rows[0].cells, [text("naïve")]);

        let mut utf16 = vec![0xFF, 0xFE];
        for unit in "Naam\nÉén".encode_utf16() {
            utf16.extend(unit.to_le_bytes());
        }
        let table = read_csv(&utf16, None).unwrap();
        assert_eq!(table.headers, ["Naam"]);
        assert_eq!(table.rows[0].cells, [text("Één")]);
    }

    #[test]
    fn an_empty_csv_file_or_an_excel_file_as_csv_is_an_error() {
        assert!(read_csv(b"", None).unwrap_err().contains("no header row"));
        assert!(
            read_csv(b"\n ; \n", None)
                .unwrap_err()
                .contains("no header row")
        );
        assert!(
            read_csv(b"PK\x03\x04rest", None)
                .unwrap_err()
                .contains("set Import file type to Excel")
        );
    }

    #[test]
    fn a_csv_file_as_excel_is_an_error() {
        assert!(
            read_excel(b"a,b\n1,2".to_vec())
                .unwrap_err()
                .contains("set Import file type to CSV")
        );
    }

    #[test]
    fn numbers_are_written_like_javascript() {
        assert_eq!(number_text(12.0), "12");
        assert_eq!(number_text(-3.0), "-3");
        assert_eq!(number_text(12.5), "12.5");
    }
}

#[cfg(test)]
mod excel_tests {
    use super::*;
    use rust_xlsxwriter::{ExcelDateTime, Format, Workbook};

    #[test]
    fn the_first_sheet_of_an_excel_file_is_read_with_cell_types() {
        let mut workbook = Workbook::new();
        let sheet = workbook.add_worksheet();
        let date = Format::new().set_num_format("dd-mm-yyyy");
        // The table starts at B3: the row numbers follow the sheet.
        sheet.write(2, 1, " Name ").unwrap();
        sheet.write(2, 2, "Age").unwrap();
        sheet.write(2, 3, "Born").unwrap();
        sheet.write(2, 4, "Ok").unwrap();
        sheet.write(3, 1, "Ann").unwrap();
        sheet.write(3, 2, 30).unwrap();
        sheet
            .write_datetime_with_format(3, 3, ExcelDateTime::from_ymd(1994, 1, 31).unwrap(), &date)
            .unwrap();
        sheet.write(3, 4, true).unwrap();
        sheet.write(5, 1, "Bob").unwrap();
        sheet.write(5, 2, 2.5).unwrap();
        workbook
            .add_worksheet()
            .write(0, 0, "second sheet")
            .unwrap();
        let bytes = workbook.save_to_buffer().unwrap();

        let table = read_excel(bytes).unwrap();
        assert_eq!(table.headers, ["Name", "Age", "Born", "Ok"]);
        assert_eq!(
            table.rows,
            vec![
                Row {
                    number: 4,
                    cells: vec![
                        Cell::Text("Ann".into()),
                        Cell::Number(30.0),
                        Cell::DateTime(
                            NaiveDate::from_ymd_opt(1994, 1, 31)
                                .unwrap()
                                .and_time(NaiveTime::MIN)
                        ),
                        Cell::Bool(true),
                    ]
                },
                Row {
                    number: 6,
                    cells: vec![
                        Cell::Text("Bob".into()),
                        Cell::Number(2.5),
                        Cell::Empty,
                        Cell::Empty
                    ]
                },
            ]
        );
    }
}
