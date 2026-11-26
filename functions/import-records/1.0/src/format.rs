//! The Format mapping and the conversion of a cell into the JSON value sent to the Data API.
//!
//! A format is written as in the ActionsJS step: `checkbox`, `decimal` (or `price`), `number`,
//! `text`, or `Date, <format>` (also `Time, ` and `Datetime, `) with a date-fns format such as
//! `dd-MM-yyyy`. `Date|dd-MM-yyyy`, the notation of the export step, works too.

use calamine::{ExcelDateTime, ExcelDateTimeType};
use chrono::{NaiveDate, NaiveDateTime, NaiveTime, Timelike};
use serde_json::{Number, Value};

use crate::read::{Cell, datetime_text, number_text};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DateKind {
    Date,
    Time,
    Datetime,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Format {
    Text,
    Decimal,
    Number,
    Checkbox,
    Date {
        kind: DateKind,
        pattern: String,
        tokens: Vec<Token>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    Literal(String),
    Field(char, usize),
}

const FORMAT_HELP: &str = "use checkbox, decimal, number, text, or Date, <format> (also Time, and Datetime,), e.g. Date, dd-MM-yyyy";

/// Parses one Format mapping value. `Date` without a format means `dd-MM-yyyy`, as in the
/// ActionsJS step; `Time` and `Datetime` need one.
pub fn parse(column: &str, spec: &Value) -> Result<Format, String> {
    let Value::String(spec) = spec else {
        return Err(format!(
            "The format of the column \"{column}\" in the Format mapping is not text: {FORMAT_HELP}"
        ));
    };
    let (kind, pattern) = match spec.find([',', '|']) {
        Some(index) => (&spec[..index], spec[index + 1..].trim()),
        None => (spec.as_str(), ""),
    };
    let kind = kind.trim();
    let date = |kind: DateKind, default: Option<&str>| {
        let pattern = match (pattern, default) {
            ("", Some(default)) => default,
            ("", None) => {
                return Err(format!(
                    "The format \"{spec}\" of the column \"{column}\" has no date format after the comma, e.g. {}",
                    match kind {
                        DateKind::Time => "Time, HH:mm",
                        _ => "Datetime, dd-MM-yyyy HH:mm",
                    }
                ));
            }
            (pattern, _) => pattern,
        };
        Ok(Format::Date {
            kind,
            pattern: pattern.to_string(),
            tokens: tokenize(column, pattern)?,
        })
    };
    match kind.to_lowercase().as_str() {
        "text" => Ok(Format::Text),
        "decimal" | "price" => Ok(Format::Decimal),
        "number" => Ok(Format::Number),
        "checkbox" => Ok(Format::Checkbox),
        "date" => date(DateKind::Date, Some("dd-MM-yyyy")),
        "time" => date(DateKind::Time, None),
        "datetime" => date(DateKind::Datetime, None),
        _ => Err(format!(
            "The format \"{spec}\" of the column \"{column}\" has the unknown kind \"{kind}\": {FORMAT_HELP}"
        )),
    }
}

const SUPPORTED: &str = "y, M, d, H, h, m, s, S and a";

fn tokenize(column: &str, pattern: &str) -> Result<Vec<Token>, String> {
    let chars: Vec<char> = pattern.chars().collect();
    let mut tokens = Vec::new();
    let mut literal = String::new();
    let mut index = 0;
    while index < chars.len() {
        let c = chars[index];
        if c == '\'' {
            // `''` is a quote; `'text'` is literal text (an unclosed one runs to the end).
            if chars.get(index + 1) == Some(&'\'') {
                literal.push('\'');
                index += 2;
                continue;
            }
            index += 1;
            while index < chars.len() {
                if chars[index] == '\'' {
                    if chars.get(index + 1) == Some(&'\'') {
                        literal.push('\'');
                        index += 2;
                        continue;
                    }
                    index += 1;
                    break;
                }
                literal.push(chars[index]);
                index += 1;
            }
            continue;
        }
        if !c.is_ascii_alphabetic() {
            literal.push(c);
            index += 1;
            continue;
        }
        let mut count = 1;
        while chars.get(index + count) == Some(&c) {
            count += 1;
        }
        if !matches!(c, 'y' | 'M' | 'd' | 'H' | 'h' | 'm' | 's' | 'S' | 'a') {
            let hint = match c {
                'Y' => " (use y for the year, not Y)",
                'D' => " (use d for the day of the month, not D)",
                _ => "",
            };
            return Err(format!(
                "The date format of the column \"{column}\" uses \"{c}\", which is not supported{hint}. Supported letters: {SUPPORTED}. Put other text in single quotes, e.g. 'at'"
            ));
        }
        if !literal.is_empty() {
            tokens.push(Token::Literal(std::mem::take(&mut literal)));
        }
        tokens.push(Token::Field(c, count));
        index += count;
    }
    if !literal.is_empty() {
        tokens.push(Token::Literal(literal));
    }
    Ok(tokens)
}

const MONTHS: [&str; 12] = [
    "january",
    "february",
    "march",
    "april",
    "may",
    "june",
    "july",
    "august",
    "september",
    "october",
    "november",
    "december",
];

#[derive(Default)]
struct Parts {
    year: Option<i32>,
    month: Option<u32>,
    day: Option<u32>,
    hour: Option<u32>,
    pm: Option<bool>,
    minute: Option<u32>,
    second: Option<u32>,
    milli: Option<u32>,
}

/// Up to `max` digits at the start of `text`, and the rest.
fn digits(text: &str, max: usize) -> Option<(u32, &str)> {
    let length = text
        .bytes()
        .take(max)
        .take_while(u8::is_ascii_digit)
        .count();
    if length == 0 {
        return None;
    }
    Some((text[..length].parse().ok()?, &text[length..]))
}

fn starts_with_ignore_case(text: &str, prefix: &str) -> bool {
    text.len() >= prefix.len()
        && text.is_char_boundary(prefix.len())
        && text[..prefix.len()].eq_ignore_ascii_case(prefix)
}

/// Reads `text` with the format tokens. Numbers may have fewer digits than the pattern
/// (`d-M-yyyy` and `dd-MM-yyyy` both read `1-2-2024`); the whole text must be used.
fn read_parts(text: &str, tokens: &[Token]) -> Option<Parts> {
    let mut parts = Parts::default();
    let mut rest = text.trim();
    for token in tokens {
        match token {
            Token::Literal(literal) => {
                if literal.trim().is_empty() {
                    // Any run of spaces matches a space in the pattern.
                    let trimmed = rest.trim_start();
                    if trimmed.len() == rest.len() {
                        return None;
                    }
                    rest = trimmed;
                } else if starts_with_ignore_case(rest, literal) {
                    rest = &rest[literal.len()..];
                } else {
                    return None;
                }
            }
            Token::Field('y', 2) => {
                let (year, after) = digits(rest, 2)?;
                if rest.len() - after.len() != 2 {
                    return None;
                }
                // Two-digit years: 00-68 is 2000-2068, 69-99 is 1969-1999.
                parts.year = Some(if year <= 68 { 2000 } else { 1900 } + year as i32);
                rest = after;
            }
            Token::Field('y', _) => {
                let (year, after) = digits(rest, 4)?;
                parts.year = Some(year as i32);
                rest = after;
            }
            Token::Field('M', count) if *count >= 3 => {
                let index = MONTHS.iter().position(|month| {
                    (*count >= 4 && starts_with_ignore_case(rest, month))
                        || starts_with_ignore_case(rest, &month[..3])
                })?;
                let name = MONTHS[index];
                let length = if *count >= 4 && starts_with_ignore_case(rest, name) {
                    name.len()
                } else {
                    3
                };
                parts.month = Some(index as u32 + 1);
                rest = &rest[length..];
            }
            Token::Field('S', count) => {
                let (fraction, after) = digits(rest, *count)?;
                let length = rest.len() - after.len();
                // Milliseconds: `5` in `.5` is 500, `05` is 50.
                let milli = match length {
                    1 => fraction * 100,
                    2 => fraction * 10,
                    _ => fraction / 10u32.pow(length as u32 - 3),
                };
                parts.milli = Some(milli);
                rest = after;
            }
            Token::Field('a', _) => {
                let lower = rest.to_ascii_lowercase();
                let (pm, length) = if lower.starts_with("am") {
                    (false, 2)
                } else if lower.starts_with("pm") {
                    (true, 2)
                } else if lower.starts_with("a.m.") {
                    (false, 4)
                } else if lower.starts_with("p.m.") {
                    (true, 4)
                } else {
                    return None;
                };
                parts.pm = Some(pm);
                rest = &rest[length..];
            }
            Token::Field(field, _) => {
                let (value, after) = digits(rest, 2)?;
                match field {
                    'M' => parts.month = Some(value),
                    'd' => parts.day = Some(value),
                    'H' | 'h' => parts.hour = Some(value),
                    'm' => parts.minute = Some(value),
                    's' => parts.second = Some(value),
                    _ => return None,
                }
                rest = after;
            }
        }
    }
    rest.trim().is_empty().then_some(parts)
}

impl Parts {
    fn date(&self) -> Option<NaiveDate> {
        NaiveDate::from_ymd_opt(self.year?, self.month?, self.day?)
    }

    fn time(&self) -> Option<NaiveTime> {
        let mut hour = self.hour.unwrap_or(0);
        match self.pm {
            Some(true) if hour < 12 => hour += 12,
            Some(false) if hour == 12 => hour = 0,
            _ => {}
        }
        NaiveTime::from_hms_milli_opt(
            hour,
            self.minute.unwrap_or(0),
            self.second.unwrap_or(0),
            self.milli.unwrap_or(0),
        )
    }
}

fn has_date_fields(tokens: &[Token]) -> bool {
    let has = |letter: char| {
        tokens
            .iter()
            .any(|token| matches!(token, Token::Field(c, _) if *c == letter))
    };
    has('y') && has('M') && has('d')
}

/// ISO-like text (`2024-01-31`, `2024-01-31 13:45`, `2024-01-31T13:45:00`, `13:45`), accepted in
/// a date column next to its format.
fn parse_iso(text: &str) -> Option<NaiveDateTime> {
    let text = text.trim();
    for format in [
        "%Y-%m-%dT%H:%M:%S%.f",
        "%Y-%m-%d %H:%M:%S%.f",
        "%Y-%m-%dT%H:%M",
        "%Y-%m-%d %H:%M",
    ] {
        if let Ok(datetime) = NaiveDateTime::parse_from_str(text, format) {
            return Some(datetime);
        }
    }
    if let Ok(date) = NaiveDate::parse_from_str(text, "%Y-%m-%d") {
        return Some(date.and_time(NaiveTime::MIN));
    }
    None
}

fn output(kind: DateKind, datetime: NaiveDateTime) -> Value {
    let text = match kind {
        DateKind::Date => datetime.format("%Y-%m-%d").to_string(),
        DateKind::Time => datetime.format("%H:%M:%S").to_string(),
        DateKind::Datetime => datetime.format("%Y-%m-%d %H:%M:%S").to_string(),
    };
    Value::String(text)
}

fn excel_serial(serial: f64) -> Option<NaiveDateTime> {
    ExcelDateTime::new(serial, ExcelDateTimeType::DateTime, false).as_datetime()
}

fn convert_date(
    cell: &Cell,
    kind: DateKind,
    pattern: &str,
    tokens: &[Token],
) -> Result<Value, String> {
    let datetime = match cell {
        Cell::DateTime(datetime) => *datetime,
        // A date in an Excel cell that is not formatted as a date: its serial number.
        Cell::Integer(serial) => {
            excel_serial(*serial as f64).ok_or_else(|| format!("{serial} is not a date"))?
        }
        Cell::Number(serial) => {
            excel_serial(*serial).ok_or_else(|| format!("{serial} is not a date"))?
        }
        Cell::Text(text) => {
            let parts = read_parts(text, tokens);
            let from_pattern = parts.and_then(|parts| {
                let time = parts.time()?;
                match kind {
                    DateKind::Time if !has_date_fields(tokens) => {
                        Some(NaiveDate::from_ymd_opt(1970, 1, 1)?.and_time(time))
                    }
                    _ => Some(parts.date()?.and_time(time)),
                }
            });
            match from_pattern.or_else(|| parse_iso(text)) {
                Some(datetime) => datetime,
                None => {
                    return Err(format!(
                        "\"{text}\" is not a valid {} in the format {pattern}",
                        match kind {
                            DateKind::Date => "date",
                            DateKind::Time => "time",
                            DateKind::Datetime => "date and time",
                        }
                    ));
                }
            }
        }
        Cell::Bool(value) => return Err(format!("{value} is not a date")),
        Cell::Empty | Cell::Error(_) => unreachable!("handled by convert"),
    };
    // Excel keeps times as fractions of a day, which can land a millisecond off.
    let datetime = if datetime.nanosecond() >= 999_000_000 {
        datetime + chrono::Duration::milliseconds(1)
    } else {
        datetime
    };
    Ok(output(kind, datetime))
}

fn integer(value: i64) -> Value {
    Value::Number(value.into())
}

fn whole_number(number: f64) -> Option<i64> {
    (number.fract() == 0.0 && number.abs() < 9.0e15).then_some(number as i64)
}

/// A number written with `.` or `,` as the decimal separator, as a decimal string with a `.`.
/// When both occur, the last one is the decimal separator and the other one groups thousands
/// (`1.234,56` and `1,234.56`). Several of the same are thousands separators (`1.234.567`).
fn decimal_text(text: &str) -> Option<String> {
    let cleaned: String = text
        .trim()
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '\u{a0}')
        .collect();
    let cleaned = cleaned.strip_prefix('+').unwrap_or(&cleaned);
    let last_dot = cleaned.rfind('.');
    let last_comma = cleaned.rfind(',');
    let separator = match (last_dot, last_comma) {
        (Some(dot), Some(comma)) => Some(if dot > comma { '.' } else { ',' }),
        (Some(_), None) if cleaned.matches('.').count() == 1 => Some('.'),
        (None, Some(_)) if cleaned.matches(',').count() == 1 => Some(','),
        _ => None,
    };
    let mut normalized = String::new();
    for c in cleaned.chars() {
        match c {
            '.' | ',' if Some(c) == separator => normalized.push('.'),
            '.' | ',' => {}
            _ => normalized.push(c),
        }
    }
    let unsigned = normalized.strip_prefix('-').unwrap_or(&normalized);
    let (whole, fraction) = unsigned.split_once('.').unwrap_or((unsigned, ""));
    let valid = !(whole.is_empty() && fraction.is_empty())
        && whole.bytes().all(|b| b.is_ascii_digit())
        && fraction.bytes().all(|b| b.is_ascii_digit())
        && !(unsigned.contains('.') && fraction.is_empty());
    if !valid {
        return None;
    }
    let sign = if normalized.starts_with('-') { "-" } else { "" };
    let whole = if whole.is_empty() { "0" } else { whole };
    // As in the ActionsJS step, a decimal without a fraction gets `.00`.
    let fraction = if fraction.is_empty() { "00" } else { fraction };
    Some(format!("{sign}{whole}.{fraction}"))
}

/// The value of a cell for the Data API. An empty cell is `null`. An error message does not name
/// the row or the column; the caller adds them.
pub fn convert(cell: &Cell, format: Option<&Format>) -> Result<Value, String> {
    match cell {
        Cell::Empty => return Ok(Value::Null),
        Cell::Error(error) => return Err(format!("the cell has the error {error}")),
        _ => {}
    }
    match format {
        None => Ok(match cell {
            Cell::Text(text) => Value::String(text.clone()),
            Cell::Integer(number) => integer(*number),
            Cell::Number(number) => match whole_number(*number) {
                Some(whole) => integer(whole),
                None => Number::from_f64(*number).map_or(Value::Null, Value::Number),
            },
            Cell::Bool(value) => Value::Bool(*value),
            Cell::DateTime(datetime) => Value::String(datetime_text(datetime)),
            Cell::Empty | Cell::Error(_) => unreachable!(),
        }),
        Some(Format::Text) => Ok(Value::String(match cell {
            Cell::Text(text) => text.clone(),
            Cell::Integer(number) => number.to_string(),
            Cell::Number(number) => number_text(*number),
            Cell::Bool(value) => value.to_string(),
            Cell::DateTime(datetime) => datetime_text(datetime),
            Cell::Empty | Cell::Error(_) => unreachable!(),
        })),
        Some(Format::Decimal) => {
            let text = match cell {
                Cell::Text(text) => text.clone(),
                Cell::Integer(number) => number.to_string(),
                Cell::Number(number) => number_text(*number),
                Cell::Bool(_) | Cell::DateTime(_) => String::new(),
                Cell::Empty | Cell::Error(_) => unreachable!(),
            };
            decimal_text(&text)
                .map(Value::String)
                .ok_or_else(|| format!("\"{}\" is not a decimal number", cell_text(cell)))
        }
        Some(Format::Number) => {
            let number = match cell {
                Cell::Integer(number) => Some(*number),
                Cell::Number(number) => whole_number(*number),
                Cell::Text(text) => {
                    let text = text.trim();
                    let text = text.strip_prefix('+').unwrap_or(text);
                    text.parse::<i64>()
                        .ok()
                        .or_else(|| text.parse::<f64>().ok().and_then(whole_number))
                }
                _ => None,
            };
            number
                .map(integer)
                .ok_or_else(|| format!("\"{}\" is not a whole number", cell_text(cell)))
        }
        Some(Format::Checkbox) => {
            let value = match cell {
                Cell::Bool(value) => Some(*value),
                Cell::Integer(1) => Some(true),
                Cell::Integer(0) => Some(false),
                Cell::Number(number) if *number == 1.0 => Some(true),
                Cell::Number(number) if *number == 0.0 => Some(false),
                Cell::Text(text) => match text.trim().to_lowercase().as_str() {
                    "true" | "1" | "yes" | "y" | "ja" | "j" => Some(true),
                    "false" | "0" | "no" | "n" | "nee" => Some(false),
                    _ => None,
                },
                _ => None,
            };
            value.map(Value::Bool).ok_or_else(|| {
                format!(
                    "\"{}\" is not a checkbox value: use true/false, 1/0 or yes/no",
                    cell_text(cell)
                )
            })
        }
        Some(Format::Date {
            kind,
            pattern,
            tokens,
        }) => convert_date(cell, *kind, pattern, tokens),
    }
}

/// A cell as text, for an error message.
fn cell_text(cell: &Cell) -> String {
    match cell {
        Cell::Empty => String::new(),
        Cell::Text(text) | Cell::Error(text) => text.clone(),
        Cell::Integer(number) => number.to_string(),
        Cell::Number(number) => number_text(*number),
        Cell::Bool(value) => value.to_string(),
        Cell::DateTime(datetime) => datetime_text(datetime),
    }
}

/// How two values are compared when matching a value from the file to a record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchKind {
    Text,
    Number,
    Decimal,
}

/// The text two equal values share: `12`, `"12"` and `12.0` are all `12` for a number or a
/// decimal (and `12.50` is `12.5` for a decimal). Text is compared exactly. `None` for `null`.
pub fn match_key(value: &Value, kind: MatchKind) -> Option<String> {
    let text = match value {
        Value::Null => return None,
        Value::String(text) => text.clone(),
        Value::Number(number) => number.to_string(),
        Value::Bool(value) => value.to_string(),
        other => other.to_string(),
    };
    match kind {
        MatchKind::Text => Some(text),
        MatchKind::Number | MatchKind::Decimal => {
            Some(text.trim().parse::<f64>().map(number_text).unwrap_or(text))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn text(value: &str) -> Cell {
        Cell::Text(value.into())
    }

    fn format(spec: &str) -> Format {
        parse("C", &json!(spec)).unwrap()
    }

    fn convert_text(value: &str, spec: &str) -> Result<Value, String> {
        convert(&text(value), Some(&format(spec)))
    }

    #[test]
    fn formats_are_parsed_like_the_actionsjs_step() {
        assert_eq!(format("checkbox"), Format::Checkbox);
        assert_eq!(format(" Decimal "), Format::Decimal);
        assert_eq!(format("price"), Format::Decimal);
        assert_eq!(format("number"), Format::Number);
        assert_eq!(format("Text"), Format::Text);
        let Format::Date { kind, pattern, .. } = format("Date") else {
            panic!()
        };
        assert_eq!((kind, pattern.as_str()), (DateKind::Date, "dd-MM-yyyy"));
        let Format::Date { kind, pattern, .. } = format("Datetime, dd/MM/yyyy HH:mm") else {
            panic!()
        };
        assert_eq!(
            (kind, pattern.as_str()),
            (DateKind::Datetime, "dd/MM/yyyy HH:mm")
        );
        let Format::Date { pattern, .. } = format("Date|MM/dd/yyyy") else {
            panic!()
        };
        assert_eq!(pattern, "MM/dd/yyyy");
    }

    #[test]
    fn a_wrong_format_is_an_error() {
        assert!(parse("C", &json!("Dates, dd")).is_err());
        assert!(parse("C", &json!("Time")).is_err());
        assert!(parse("C", &json!("Datetime,")).is_err());
        assert!(parse("C", &json!(3)).is_err());
        assert!(
            parse("C", &json!("Date, DD-MM-YYYY"))
                .unwrap_err()
                .contains("use d for the day")
        );
    }

    #[test]
    fn dates_are_read_with_their_pattern() {
        assert_eq!(
            convert_text("31-01-2024", "Date").unwrap(),
            json!("2024-01-31")
        );
        assert_eq!(
            convert_text("1-2-2024", "Date").unwrap(),
            json!("2024-02-01")
        );
        assert_eq!(
            convert_text("01/31/24", "Date, MM/dd/yy").unwrap(),
            json!("2024-01-31")
        );
        assert_eq!(
            convert_text("31 jan 2024", "Date, d MMM yyyy").unwrap(),
            json!("2024-01-31")
        );
        assert_eq!(
            convert_text("31 January 2024", "Date, d MMMM yyyy").unwrap(),
            json!("2024-01-31")
        );
        assert_eq!(
            convert_text("20240131", "Date, yyyyMMdd").unwrap(),
            json!("2024-01-31")
        );
        assert_eq!(
            convert_text("31-01-2024 13:45", "Datetime, dd-MM-yyyy HH:mm").unwrap(),
            json!("2024-01-31 13:45:00")
        );
        assert_eq!(
            convert_text("31-01-2024  1:05 pm", "Datetime, dd-MM-yyyy h:mm a").unwrap(),
            json!("2024-01-31 13:05:00")
        );
        assert_eq!(
            convert_text("12:30 AM", "Time, hh:mm a").unwrap(),
            json!("00:30:00")
        );
        assert_eq!(
            convert_text("13:45:10", "Time, HH:mm:ss").unwrap(),
            json!("13:45:10")
        );
        assert_eq!(
            convert_text("31-01-2024 om 9:00", "Datetime, dd-MM-yyyy 'om' H:mm").unwrap(),
            json!("2024-01-31 09:00:00")
        );
        // An ISO date is accepted next to the pattern.
        assert_eq!(
            convert_text("2024-01-31", "Date").unwrap(),
            json!("2024-01-31")
        );
        // A datetime pattern on a Date column keeps only the date.
        assert_eq!(
            convert_text("31-01-2024 13:45", "Date, dd-MM-yyyy HH:mm").unwrap(),
            json!("2024-01-31")
        );
    }

    #[test]
    fn invalid_dates_are_errors() {
        assert_eq!(
            convert_text("31-02-2024", "Date").unwrap_err(),
            "\"31-02-2024\" is not a valid date in the format dd-MM-yyyy"
        );
        assert!(convert_text("31-01-2024x", "Date").is_err());
        assert!(convert_text("2024", "Date").is_err());
        assert!(convert_text("25:00", "Time, HH:mm").is_err());
    }

    #[test]
    fn excel_dates_ignore_the_pattern() {
        let datetime = NaiveDate::from_ymd_opt(2024, 1, 31)
            .unwrap()
            .and_hms_opt(13, 45, 0)
            .unwrap();
        let cell = Cell::DateTime(datetime);
        assert_eq!(
            convert(&cell, Some(&format("Date"))).unwrap(),
            json!("2024-01-31")
        );
        assert_eq!(
            convert(&cell, Some(&format("Datetime, d"))).unwrap(),
            json!("2024-01-31 13:45:00")
        );
        assert_eq!(
            convert(&Cell::Number(45322.5), Some(&format("Datetime, d"))).unwrap(),
            json!("2024-01-31 12:00:00")
        );
        assert_eq!(
            convert(&Cell::Integer(45322), Some(&format("Date"))).unwrap(),
            json!("2024-01-31")
        );
    }

    #[test]
    fn decimals_use_a_dot_and_keep_their_digits() {
        assert_eq!(convert_text("12,5", "decimal").unwrap(), json!("12.5"));
        assert_eq!(convert_text("12", "decimal").unwrap(), json!("12.00"));
        assert_eq!(convert_text("-0.125", "decimal").unwrap(), json!("-0.125"));
        assert_eq!(
            convert_text("1.234,56", "decimal").unwrap(),
            json!("1234.56")
        );
        assert_eq!(
            convert_text("1,234.56", "decimal").unwrap(),
            json!("1234.56")
        );
        assert_eq!(
            convert_text("1.234.567", "decimal").unwrap(),
            json!("1234567.00")
        );
        assert_eq!(convert_text(" + 7 ", "decimal").unwrap(), json!("7.00"));
        assert_eq!(convert_text(",5", "decimal").unwrap(), json!("0.5"));
        assert_eq!(
            convert(&Cell::Number(3.25), Some(&Format::Decimal)).unwrap(),
            json!("3.25")
        );
        for bad in ["abc", "1.2.3,4,5", "12.", "-", "1e5"] {
            assert!(convert_text(bad, "decimal").is_err(), "{bad}");
        }
    }

    #[test]
    fn numbers_must_be_whole() {
        assert_eq!(convert_text("42", "number").unwrap(), json!(42));
        assert_eq!(convert_text(" -3 ", "number").unwrap(), json!(-3));
        assert_eq!(convert_text("12.0", "number").unwrap(), json!(12));
        assert_eq!(
            convert(&Cell::Number(7.0), Some(&Format::Number)).unwrap(),
            json!(7)
        );
        assert_eq!(
            convert_text("12.5", "number").unwrap_err(),
            "\"12.5\" is not a whole number"
        );
        assert!(convert_text("abc", "number").is_err());
    }

    #[test]
    fn checkboxes() {
        for yes in ["true", "TRUE", "1", "yes", "Ja"] {
            assert_eq!(convert_text(yes, "checkbox").unwrap(), json!(true), "{yes}");
        }
        for no in ["false", "0", "No", "nee"] {
            assert_eq!(convert_text(no, "checkbox").unwrap(), json!(false), "{no}");
        }
        assert_eq!(
            convert(&Cell::Integer(1), Some(&Format::Checkbox)).unwrap(),
            json!(true)
        );
        assert!(convert_text("maybe", "checkbox").is_err());
    }

    #[test]
    fn unformatted_cells_keep_their_type_and_text_makes_text() {
        assert_eq!(convert(&text("x"), None).unwrap(), json!("x"));
        assert_eq!(convert(&Cell::Number(5.0), None).unwrap(), json!(5));
        assert_eq!(convert(&Cell::Number(5.5), None).unwrap(), json!(5.5));
        assert_eq!(convert(&Cell::Bool(true), None).unwrap(), json!(true));
        assert_eq!(convert(&Cell::Empty, None).unwrap(), Value::Null);
        assert_eq!(
            convert(&Cell::Empty, Some(&Format::Number)).unwrap(),
            Value::Null
        );
        assert_eq!(
            convert(&Cell::Number(5.0), Some(&Format::Text)).unwrap(),
            json!("5")
        );
        assert_eq!(
            convert(&Cell::Error("#N/A".into()), None).unwrap_err(),
            "the cell has the error #N/A"
        );
    }

    #[test]
    fn match_keys_compare_numbers_by_value() {
        assert_eq!(
            match_key(&json!("12"), MatchKind::Number),
            Some("12".into())
        );
        assert_eq!(match_key(&json!(12), MatchKind::Number), Some("12".into()));
        assert_eq!(
            match_key(&json!("12.50"), MatchKind::Decimal),
            Some("12.5".into())
        );
        assert_eq!(
            match_key(&json!(12.5), MatchKind::Decimal),
            Some("12.5".into())
        );
        assert_eq!(match_key(&json!("Ab"), MatchKind::Text), Some("Ab".into()));
        assert_eq!(match_key(&Value::Null, MatchKind::Text), None);
    }
}
