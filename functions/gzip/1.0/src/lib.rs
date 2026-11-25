mod bindings {
    wit_bindgen::generate!({ generate_all });

    use crate::Gzip;
    export!(Gzip);
}

use std::borrow::Cow;
use std::io::Write;

use base64::Engine;
use base64::alphabet::STANDARD;
use base64::engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig};
use flate2::{Compression, GzBuilder};

use crate::bindings::exports::betty_blocks::gzip::gzip::Guest;

/// The most bytes this step accepts after decoding its input. A guard against payloads that
/// would exhaust the component's memory; the platform's own limits for a text value have not
/// been measured, so this is a conservative upper bound, not a documented platform limit.
#[cfg(not(test))]
const MAX_INPUT_BYTES: usize = 32 * 1024 * 1024;
/// A lower limit for unit tests, so the size checks run fast in a debug build.
#[cfg(test)]
const MAX_INPUT_BYTES: usize = 64 * 1024;

/// Standard base64 (RFC 4648 section 4: `+` and `/`). Output is always padded; input is accepted
/// with or without padding, because some systems drop the trailing `=`.
const BASE64: GeneralPurpose = GeneralPurpose::new(
    &STANDARD,
    GeneralPurposeConfig::new()
        .with_encode_padding(true)
        .with_decode_padding_mode(DecodePaddingMode::Indifferent),
);

#[derive(Clone, Copy, Debug, PartialEq)]
enum Encoding {
    Utf8,
    Base64,
}

fn parse_encoding(value: &str, label: &str, default: Encoding) -> Result<Encoding, String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "" => Ok(default),
        "utf8" | "utf-8" => Ok(Encoding::Utf8),
        "base64" => Ok(Encoding::Base64),
        other => Err(format!(
            "\"{label}\" must be \"utf8\" or \"base64\", found \"{other}\" — pick one of the two values in the step's \"{label}\" option"
        )),
    }
}

fn too_large(size: usize) -> String {
    format!(
        "the input is {size} bytes after decoding, which is more than the {MAX_INPUT_BYTES} bytes this step accepts — split the data or send a smaller file"
    )
}

/// Decodes base64 text, ignoring line breaks and other whitespace (MIME-style base64 wraps at
/// 76 characters, and copied values often end in a newline).
fn decode_base64(data: &str) -> Result<Vec<u8>, String> {
    let compact: Cow<str> = if data.contains(|c: char| c.is_ascii_whitespace()) {
        Cow::Owned(data.split_ascii_whitespace().collect())
    } else {
        Cow::Borrowed(data)
    };
    // Every 4 base64 characters hold 3 bytes; trailing `=` padding holds none.
    let padding = compact.bytes().rev().take_while(|&b| b == b'=').count();
    let decoded_size = (compact.len() - padding) * 3 / 4;
    if decoded_size > MAX_INPUT_BYTES {
        return Err(too_large(decoded_size));
    }
    BASE64.decode(compact.as_bytes()).map_err(|error| {
        let hint = if compact.contains(['-', '_']) {
            " The text contains \"-\" or \"_\", which suggests URL-safe base64: only standard base64 (with \"+\" and \"/\") is supported."
        } else {
            " If the data is plain text, set \"Input encoding\" to utf8."
        };
        format!("\"Data\" is not valid base64: {error}.{hint}")
    })
}

fn decode_input(data: &str, encoding: Encoding) -> Result<Cow<'_, [u8]>, String> {
    let bytes = match encoding {
        Encoding::Utf8 => Cow::Borrowed(data.as_bytes()),
        Encoding::Base64 => Cow::Owned(decode_base64(data)?),
    };
    if bytes.len() > MAX_INPUT_BYTES {
        return Err(too_large(bytes.len()));
    }
    Ok(bytes)
}

/// Gzip-compresses `input`. The header's modification time is 0 and no file name is stored,
/// so the same input always gives byte-identical output.
fn compress(input: &[u8]) -> Result<Vec<u8>, String> {
    let mut encoder = GzBuilder::new().mtime(0).write(
        Vec::with_capacity(input.len() / 4 + 64),
        Compression::default(),
    );
    encoder
        .write_all(input)
        .and_then(|()| encoder.finish())
        .map_err(|error| format!("compressing the data failed: {error}"))
}

/// The result is always base64: gzip output starts with 1f 8b, which is never valid UTF-8.
fn run(data: &str, input_encoding: &str) -> Result<String, String> {
    let input_encoding = parse_encoding(input_encoding, "Input encoding", Encoding::Utf8)?;
    let input = decode_input(data, input_encoding)?;
    Ok(BASE64.encode(compress(&input)?))
}

struct Gzip;

impl Guest for Gzip {
    fn gzip(data: Option<String>, input_encoding: String) -> Result<String, String> {
        run(data.as_deref().unwrap_or_default(), &input_encoding)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_starts_with_the_gzip_magic_bytes() {
        let result = run("<xml/>", "utf8").unwrap();
        assert!(result.starts_with("H4sI"), "result was: {result}");
    }

    #[test]
    fn same_input_gives_identical_output() {
        let first = run("hello hello hello", "utf8").unwrap();
        let second = run("hello hello hello", "utf8").unwrap();
        assert_eq!(first, second);
    }

    #[test]
    fn header_has_mtime_zero() {
        let compressed = BASE64.decode(run("abc", "utf8").unwrap()).unwrap();
        assert_eq!(&compressed[..3], &[0x1f, 0x8b, 0x08]);
        assert_eq!(&compressed[4..8], &[0, 0, 0, 0]);
    }

    #[test]
    fn base64_input_matches_utf8_input() {
        let xml = "<Document><Amount>12.50</Amount></Document>";
        let as_base64 = BASE64.encode(xml);
        assert_eq!(
            run(&as_base64, "base64").unwrap(),
            run(xml, "utf8").unwrap()
        );
    }

    #[test]
    fn base64_input_with_line_breaks_and_no_padding_is_accepted() {
        let xml = "<Document>é</Document>";
        let encoded = BASE64.encode(xml);
        let wrapped = format!(
            "{}\r\n{}\n",
            &encoded[..10],
            encoded[10..].trim_end_matches('=')
        );
        assert_eq!(run(&wrapped, "base64").unwrap(), run(xml, "utf8").unwrap());
    }

    #[test]
    fn empty_encoding_uses_the_default() {
        assert_eq!(run("abc", "").unwrap(), run("abc", "utf8").unwrap());
    }

    #[test]
    fn padded_base64_of_exactly_the_limit_is_accepted() {
        for size in [MAX_INPUT_BYTES, MAX_INPUT_BYTES - 2] {
            let encoded = BASE64.encode(vec![0u8; size]);
            assert!(encoded.ends_with('='), "test needs padding, size {size}");
            assert_eq!(decode_base64(&encoded).unwrap().len(), size);
        }
        let error = decode_base64(&BASE64.encode(vec![0u8; MAX_INPUT_BYTES + 1])).unwrap_err();
        assert!(error.contains("more than"), "error was: {error}");
    }

    #[test]
    fn invalid_base64_is_a_clear_error() {
        let error = run("not base64!", "base64").unwrap_err();
        assert!(error.contains("not valid base64"), "error was: {error}");
    }

    #[test]
    fn url_safe_base64_gets_a_hint() {
        let error = run("ab-_", "base64").unwrap_err();
        assert!(error.contains("URL-safe"), "error was: {error}");
    }

    #[test]
    fn unknown_encoding_is_a_clear_error() {
        let error = run("abc", "hex").unwrap_err();
        assert!(error.contains("\"Input encoding\""), "error was: {error}");
    }

    #[test]
    fn oversized_input_is_rejected() {
        let error = decode_input(&"a".repeat(MAX_INPUT_BYTES + 1), Encoding::Utf8).unwrap_err();
        assert!(error.contains("more than"), "error was: {error}");
    }
}
