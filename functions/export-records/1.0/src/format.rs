//! The Format mapping: `Date|dd-MM-yyyy` (also `Datetime|...` and `Time|...`) and `Decimal|,`
//! (also `Price|,`).
//!
//! The ActionsJS step formatted dates with date-fns 2. This is the date-fns format syntax for the
//! tokens a file export needs (y, M, d, E, Q, H, h, m, s, S, a); any other letter is an error
//! instead of being formatted, so a pattern that worked in the ActionsJS step (e.g. with `do`)
//! can fail here. Datetimes are formatted in UTC (the ActionsJS step used the server's time zone).

use chrono::{DateTime, Datelike, NaiveDate, NaiveDateTime, NaiveTime, Timelike};
use serde_json::Value;

/// A parsed format for one column.
#[derive(Debug, Clone, PartialEq)]
pub enum Format {
    /// date-fns format tokens.
    Date(Vec<Token>),
    /// Replace the first `.` of the value with this text.
    Decimal(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    Literal(String),
    Field(char, usize),
}

const FORMAT_HELP: &str = "use Date|<format> (also Datetime| and Time|), e.g. Date|dd-MM-yyyy, or Decimal|<separator> (also Price|), e.g. Decimal|,";

/// Parses one Format mapping value. A value without `|`, with an unknown kind or with an empty
/// format is a configuration mistake and fails the step (the ActionsJS step ignored it and
/// exported the value unformatted). The format is trimmed, so `Date| dd-MM-yyyy` adds no space.
pub fn parse(column: &str, spec: &str) -> Result<Format, String> {
    let Some((kind, pattern)) = spec.split_once('|') else {
        return Err(format!(
            "The format \"{spec}\" of the column \"{column}\" has no |: {FORMAT_HELP}"
        ));
    };
    let pattern = pattern.trim();
    let kind = kind.trim();
    if pattern.is_empty() {
        return Err(format!(
            "The format \"{spec}\" of the column \"{column}\" is empty after the |: {FORMAT_HELP}"
        ));
    }
    match kind.to_lowercase().as_str() {
        "date" | "datetime" | "time" => Ok(Format::Date(tokenize(column, pattern)?)),
        "decimal" | "price" => Ok(Format::Decimal(pattern.to_string())),
        _ => Err(format!(
            "The format \"{spec}\" of the column \"{column}\" has the unknown kind \"{kind}\": {FORMAT_HELP}"
        )),
    }
}

const SUPPORTED: &str = "y, M, d, E, H, h, m, s, S, a and Q";

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
        if !matches!(
            c,
            'y' | 'M' | 'd' | 'E' | 'H' | 'h' | 'm' | 's' | 'S' | 'a' | 'Q'
        ) {
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

/// A date, a time or both, read from a Data API value.
enum Moment {
    Date(NaiveDate),
    Time(NaiveTime),
    DateTime(NaiveDateTime),
}

fn parse_moment(text: &str) -> Option<Moment> {
    let text = text.trim();
    if let Ok(date) = NaiveDate::parse_from_str(text, "%Y-%m-%d") {
        return Some(Moment::Date(date));
    }
    if let Ok(datetime) = DateTime::parse_from_rfc3339(text) {
        return Some(Moment::DateTime(datetime.naive_utc()));
    }
    for format in [
        "%Y-%m-%dT%H:%M:%S%.f",
        "%Y-%m-%d %H:%M:%S%.f",
        "%Y-%m-%dT%H:%M",
    ] {
        if let Ok(datetime) = NaiveDateTime::parse_from_str(text, format) {
            return Some(Moment::DateTime(datetime));
        }
    }
    for format in ["%H:%M:%S%.f", "%H:%M"] {
        if let Ok(time) = NaiveTime::parse_from_str(text, format) {
            return Some(Moment::Time(time));
        }
    }
    None
}

const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];
const DAYS: [&str; 7] = [
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
    "Sunday",
];

fn padded(number: i64, width: usize) -> String {
    if number < 0 {
        format!("-{:0width$}", -number)
    } else {
        format!("{number:0width$}")
    }
}

/// Formats a date/time text with date-fns tokens.
fn format_moment(column: &str, text: &str, tokens: &[Token]) -> Result<String, String> {
    let moment = parse_moment(text).ok_or_else(|| {
        format!("The value \"{text}\" of the column \"{column}\" is not a date, time or datetime")
    })?;
    let (date, time) = match moment {
        Moment::Date(date) => (Some(date), NaiveTime::MIN),
        Moment::Time(time) => (None, time),
        Moment::DateTime(datetime) => (Some(datetime.date()), datetime.time()),
    };
    let need_date = || {
        date.ok_or_else(|| {
            format!(
                "The value \"{text}\" of the column \"{column}\" is a time, so its format can only use H, h, m, s, S and a"
            )
        })
    };

    let mut out = String::new();
    for token in tokens {
        let (letter, count) = match token {
            Token::Literal(text) => {
                out.push_str(text);
                continue;
            }
            Token::Field(letter, count) => (*letter, *count),
        };
        let piece = match letter {
            'y' => {
                let year = i64::from(need_date()?.year());
                if count == 2 {
                    padded(year.rem_euclid(100), 2)
                } else {
                    // Bounded like d and H: a very long run of y would otherwise ask for a
                    // padding width the formatter cannot handle.
                    padded(year, count.min(9))
                }
            }
            'M' => {
                let month = need_date()?.month() as usize;
                match count {
                    1 | 2 => padded(month as i64, count),
                    3 => MONTHS[month - 1][..3].to_string(),
                    4 => MONTHS[month - 1].to_string(),
                    _ => MONTHS[month - 1][..1].to_string(),
                }
            }
            'd' => padded(i64::from(need_date()?.day()), count.min(2)),
            'E' => {
                let day = DAYS[need_date()?.weekday().num_days_from_monday() as usize];
                match count {
                    1..=3 => day[..3].to_string(),
                    4 => day.to_string(),
                    5 => day[..1].to_string(),
                    _ => day[..2].to_string(),
                }
            }
            'Q' => {
                let quarter = (need_date()?.month() - 1) / 3 + 1;
                match count {
                    1 | 2 => padded(i64::from(quarter), count),
                    3 => format!("Q{quarter}"),
                    _ => format!(
                        "{} quarter",
                        ["1st", "2nd", "3rd", "4th"][quarter as usize - 1]
                    ),
                }
            }
            'H' => padded(i64::from(time.hour()), count.min(2)),
            'h' => {
                let hour = time.hour() % 12;
                padded(i64::from(if hour == 0 { 12 } else { hour }), count.min(2))
            }
            'm' => padded(i64::from(time.minute()), count.min(2)),
            's' => padded(i64::from(time.second()), count.min(2)),
            'S' => {
                let nanos = format!("{:09}", time.nanosecond() % 1_000_000_000);
                let mut digits: String = nanos.chars().take(count).collect();
                while digits.len() < count {
                    digits.push('0');
                }
                digits
            }
            'a' => {
                let pm = time.hour() >= 12;
                match count {
                    1 | 2 => (if pm { "PM" } else { "AM" }).to_string(),
                    3 => (if pm { "pm" } else { "am" }).to_string(),
                    4 => (if pm { "p.m." } else { "a.m." }).to_string(),
                    _ => (if pm { "p" } else { "a" }).to_string(),
                }
            }
            _ => unreachable!("tokenize only accepts supported letters"),
        };
        out.push_str(&piece);
    }
    Ok(out)
}

/// The value after formatting. Only text and numbers are formatted; other values (and values
/// without a format) are returned as they are.
pub fn apply(column: &str, value: &Value, format: Option<&Format>) -> Result<Value, String> {
    let text = match value {
        Value::String(text) => text.clone(),
        Value::Number(number) => crate::cell::number_to_string(number),
        _ => return Ok(value.clone()),
    };
    match format {
        None => Ok(value.clone()),
        Some(Format::Date(tokens)) => Ok(Value::String(format_moment(column, &text, tokens)?)),
        // Only a number, or text that is one, gets the separator: `v1.2-beta` stays as it is.
        Some(Format::Decimal(separator)) if text.trim().parse::<f64>().is_ok() => {
            Ok(Value::String(text.replacen('.', separator, 1)))
        }
        Some(Format::Decimal(_)) => Ok(value.clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn date(pattern: &str, value: &str) -> Result<String, String> {
        let format = parse("C", &format!("Date|{pattern}"))?;
        apply("C", &json!(value), Some(&format)).map(|v| v.as_str().unwrap().to_string())
    }

    #[test]
    fn date_tokens_format_like_date_fns() {
        assert_eq!(date("dd-MM-yyyy", "2024-03-07").unwrap(), "07-03-2024");
        assert_eq!(date("d/M/yy", "2024-03-07").unwrap(), "7/3/24");
        assert_eq!(date("MMM MMMM", "2024-03-07").unwrap(), "Mar March");
        assert_eq!(date("EEE EEEE", "2024-03-07").unwrap(), "Thu Thursday");
        assert!(date("Qo", "2024-03-07").is_err());
        assert_eq!(date("QQQ", "2024-11-07").unwrap(), "Q4");
        assert_eq!(
            date("yyyy-MM-dd HH:mm:ss", "2024-03-07T14:05:09Z").unwrap(),
            "2024-03-07 14:05:09"
        );
        assert_eq!(date("h:mm a", "2024-03-07T00:05:00Z").unwrap(), "12:05 AM");
        assert_eq!(date("h:mm a", "2024-03-07T13:05:00Z").unwrap(), "1:05 PM");
        assert_eq!(
            date("HH:mm:ss.SSS", "2024-03-07T13:05:00.25Z").unwrap(),
            "13:05:00.250"
        );
    }

    #[test]
    fn datetimes_with_an_offset_are_formatted_in_utc() {
        assert_eq!(
            date("dd-MM-yyyy HH:mm", "2024-03-07T00:30:00+02:00").unwrap(),
            "06-03-2024 22:30"
        );
    }

    #[test]
    fn quoted_text_is_literal() {
        assert_eq!(
            date("d MMM yyyy 'at' HH:mm", "2024-03-07T14:05:00Z").unwrap(),
            "7 Mar 2024 at 14:05"
        );
        assert_eq!(
            date("'o''clock' H", "2024-03-07T14:05:00Z").unwrap(),
            "o'clock 14"
        );
    }

    #[test]
    fn a_time_value_formats_time_tokens_only() {
        assert_eq!(date("HH:mm", "09:30:00").unwrap(), "09:30");
        assert!(date("dd-MM-yyyy", "09:30:00").is_err());
    }

    #[test]
    fn unsupported_letters_and_bad_values_are_errors() {
        assert!(
            date("DD-MM-YYYY", "2024-03-07")
                .unwrap_err()
                .contains("not D")
        );
        assert!(date("dd-MM-yyyy", "not a date").is_err());
    }

    #[test]
    fn decimal_replaces_the_first_point() {
        let format = parse("C", "Decimal|,").unwrap();
        assert_eq!(
            apply("C", &json!(12.5), Some(&format)).unwrap(),
            json!("12,5")
        );
        assert_eq!(
            apply("C", &json!("3.25"), Some(&format)).unwrap(),
            json!("3,25")
        );
        assert_eq!(apply("C", &json!(7), Some(&format)).unwrap(), json!("7"));
        let price = parse("C", "price| , ").unwrap();
        assert_eq!(price, Format::Decimal(",".into()));
    }

    #[test]
    fn unknown_or_incomplete_formats_are_errors_that_name_the_column() {
        let error = parse("Born", "Date:dd-MM-yyyy").unwrap_err();
        assert!(
            error.starts_with("The format \"Date:dd-MM-yyyy\" of the column \"Born\" has no |"),
            "{error}"
        );
        assert!(
            parse("Born", "Date|")
                .unwrap_err()
                .contains("is empty after the |")
        );
        assert!(
            parse("Born", "Date|  ")
                .unwrap_err()
                .contains("is empty after the |")
        );
        assert!(
            parse("Born", "Dates|dd-MM-yyyy")
                .unwrap_err()
                .contains("the unknown kind \"Dates\"")
        );
    }

    #[test]
    fn the_date_format_is_trimmed() {
        assert_eq!(date(" dd-MM-yyyy ", "2024-03-07").unwrap(), "07-03-2024");
    }

    #[test]
    fn datetime_and_time_kinds_format_like_date() {
        let datetime = parse("C", "Datetime|dd-MM-yyyy HH:mm").unwrap();
        assert_eq!(
            apply("C", &json!("2024-03-07T14:05:09Z"), Some(&datetime)).unwrap(),
            json!("07-03-2024 14:05")
        );
        let time = parse("C", "Time|hh:mm a").unwrap();
        assert_eq!(
            apply("C", &json!("09:30:00"), Some(&time)).unwrap(),
            json!("09:30 AM")
        );
    }

    #[test]
    fn more_tokens_format_like_date_fns() {
        assert_eq!(date("yy", "1999-03-07").unwrap(), "99");
        assert_eq!(date("S SS", "2024-03-07T13:05:00.256Z").unwrap(), "2 25");
        assert_eq!(date("hh a", "2024-03-07T12:00:00Z").unwrap(), "12 PM");
        assert_eq!(date("hh a", "2024-03-07T00:00:00Z").unwrap(), "12 AM");
        assert_eq!(date("h aaa", "2024-03-07T15:00:00Z").unwrap(), "3 pm");
        assert_eq!(date("h aaaa", "2024-03-07T15:00:00Z").unwrap(), "3 p.m.");
        assert_eq!(date("h aaaaa", "2024-03-07T03:00:00Z").unwrap(), "3 a");
    }

    #[test]
    fn a_very_long_year_token_does_not_panic() {
        let pattern = "y".repeat(65_536);
        assert_eq!(date(&pattern, "2024-03-07").unwrap(), "000002024");
    }

    #[test]
    fn decimal_leaves_text_that_is_not_a_number_alone() {
        let format = parse("C", "Decimal|,").unwrap();
        for text in ["v1.2-beta", "a.b@x.com"] {
            assert_eq!(
                apply("C", &json!(text), Some(&format)).unwrap(),
                json!(text)
            );
        }
    }

    #[test]
    fn decimal_replaces_only_the_first_point() {
        let format = parse("C", "Decimal|,").unwrap();
        assert_eq!(
            apply("C", &json!("1234.5"), Some(&format)).unwrap(),
            json!("1234,5")
        );
    }

    #[test]
    fn booleans_and_objects_are_not_formatted() {
        let format = parse("C", "Decimal|,").unwrap();
        assert_eq!(
            apply("C", &json!(true), Some(&format)).unwrap(),
            json!(true)
        );
    }
}
