mod bindings {
    wit_bindgen::generate!({ generate_all });

    use crate::Gunzip;
    export!(Gunzip);
}

use std::borrow::Cow;
use std::io::Read;

use base64::Engine;
use base64::alphabet::STANDARD;
use base64::engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig};
use flate2::bufread::GzDecoder;

use crate::bindings::exports::betty_blocks::gunzip::gunzip::Guest;

/// The most bytes this step accepts after decoding its input, and the most it will produce when
/// decompressing (a small gzip stream can expand to gigabytes). A guard against payloads that
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
            " Data must be gzip-compressed data encoded as base64 (it starts with \"H4sI\"): plain text, or raw gzip bytes passed as text, cannot be decompressed."
        };
        format!("\"Data\" is not valid base64: {error}.{hint}")
    })
}

fn encode_output(bytes: Vec<u8>, encoding: Encoding) -> Result<String, String> {
    match encoding {
        Encoding::Base64 => Ok(BASE64.encode(bytes)),
        Encoding::Utf8 => String::from_utf8(bytes).map_err(|error| {
            format!(
                "the result is not valid UTF-8 text (invalid byte at position {}) — set \"Output encoding\" to base64",
                error.utf8_error().valid_up_to()
            )
        }),
    }
}

/// The two bytes every gzip stream starts with (RFC 1952).
const GZIP_MAGIC: [u8; 2] = [0x1f, 0x8b];

/// Decompresses one gzip stream, or several concatenated ones, and stops with an error once the
/// result would pass `MAX_INPUT_BYTES`. Like `gzip -d`, it ignores anything after the last
/// member that does not start with the gzip signature, such as the NUL padding some systems
/// add to fill a block.
fn decompress(input: &[u8]) -> Result<Vec<u8>, String> {
    if input.is_empty() {
        return Err(String::from(
            "\"Data\" is empty — map it to the variable or property that holds the gzip data",
        ));
    }
    if !input.starts_with(&GZIP_MAGIC) {
        return Err(String::from(
            "\"Data\" is not gzip data: it does not start with the gzip signature (1f 8b, \"H4sI\" in base64) — check that the value really is gzip-compressed and base64-encoded",
        ));
    }
    let mut output = Vec::with_capacity(input.len().saturating_mul(4).min(MAX_INPUT_BYTES));
    let mut remaining = input;
    while remaining.starts_with(&GZIP_MAGIC) {
        // The bufread decoder takes only the bytes of this member from `remaining`.
        let budget = (MAX_INPUT_BYTES + 1 - output.len()) as u64;
        GzDecoder::new(&mut remaining)
            .take(budget)
            .read_to_end(&mut output)
            .map_err(|error| {
                format!("\"Data\" is not a valid gzip stream: {error} — the data may be truncated or corrupted")
            })?;
        if output.len() > MAX_INPUT_BYTES {
            return Err(format!(
                "the decompressed data is more than the {MAX_INPUT_BYTES} bytes this step can produce — the input is too large to decompress here"
            ));
        }
    }
    Ok(output)
}

/// Data is always base64: gzip data starts with 1f 8b, which is never valid UTF-8, so it cannot
/// arrive as text.
fn run(data: &str, output_encoding: &str) -> Result<String, String> {
    let output_encoding = parse_encoding(output_encoding, "Output encoding", Encoding::Utf8)?;
    // decode_base64 also enforces MAX_INPUT_BYTES: its size estimate is exact for valid base64.
    let input = decode_base64(data)?;
    let decompressed = decompress(&input)?;
    encode_output(decompressed, output_encoding)
}

struct Gunzip;

impl Guest for Gunzip {
    fn gunzip(data: Option<String>, output_encoding: String) -> Result<String, String> {
        run(data.as_deref().unwrap_or_default(), &output_encoding)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Produced with `printf 'hello' | gzip -n | base64` so this checks against
    // real gzip output, not only against this crate's own counterpart.
    const HELLO_GZIP: &str = "H4sIAAAAAAAAA8tIzcnJBwCGphA2BQAAAA==";

    #[test]
    fn decompresses_real_gzip_output() {
        assert_eq!(run(HELLO_GZIP, "utf8").unwrap(), "hello");
    }

    #[test]
    fn empty_encoding_uses_the_default() {
        assert_eq!(run(HELLO_GZIP, "").unwrap(), "hello");
    }

    #[test]
    fn base64_output_returns_the_raw_bytes_encoded() {
        assert_eq!(run(HELLO_GZIP, "base64").unwrap(), "aGVsbG8=");
    }

    #[test]
    fn concatenated_members_are_all_decompressed() {
        let member = BASE64.decode(HELLO_GZIP).unwrap();
        let twice = BASE64.encode([member.clone(), member].concat());
        assert_eq!(run(&twice, "utf8").unwrap(), "hellohello");
    }

    #[test]
    fn trailing_nul_padding_is_ignored() {
        let mut bytes = BASE64.decode(HELLO_GZIP).unwrap();
        bytes.extend([0u8; 512]);
        assert_eq!(run(&BASE64.encode(bytes), "utf8").unwrap(), "hello");
    }

    #[test]
    fn trailing_garbage_is_ignored() {
        let mut bytes = BASE64.decode(HELLO_GZIP).unwrap();
        bytes.extend(b"junk");
        assert_eq!(run(&BASE64.encode(bytes), "utf8").unwrap(), "hello");
    }

    #[test]
    fn a_broken_second_member_is_still_an_error() {
        let mut bytes = BASE64.decode(HELLO_GZIP).unwrap();
        bytes.extend([0x1f, 0x8b, 0x08]);
        let error = run(&BASE64.encode(bytes), "utf8").unwrap_err();
        assert!(
            error.contains("not a valid gzip stream"),
            "error was: {error}"
        );
    }

    #[test]
    fn empty_data_is_a_clear_error() {
        for data in ["", "  \n"] {
            let error = run(data, "utf8").unwrap_err();
            assert!(error.contains("\"Data\" is empty"), "error was: {error}");
        }
    }

    #[test]
    fn non_gzip_input_is_a_clear_error() {
        let error = run(&BASE64.encode("<xml/>"), "utf8").unwrap_err();
        assert!(error.contains("not gzip data"), "error was: {error}");
    }

    #[test]
    fn truncated_gzip_is_a_clear_error() {
        let bytes = BASE64.decode(HELLO_GZIP).unwrap();
        let truncated = BASE64.encode(&bytes[..bytes.len() - 6]);
        let error = run(&truncated, "utf8").unwrap_err();
        assert!(
            error.contains("not a valid gzip stream"),
            "error was: {error}"
        );
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
    fn plain_text_data_is_a_clear_error() {
        let error = run("<Document/>", "utf8").unwrap_err();
        assert!(
            error.contains("not valid base64") && error.contains("plain text"),
            "error was: {error}"
        );
    }

    #[test]
    fn invalid_base64_is_a_clear_error() {
        let error = run("H4sI***", "utf8").unwrap_err();
        assert!(error.contains("not valid base64"), "error was: {error}");
    }

    #[test]
    fn binary_result_as_utf8_is_a_clear_error() {
        // The bytes ff fe are not valid UTF-8.
        let error = encode_output(vec![0xff, 0xfe], Encoding::Utf8).unwrap_err();
        assert!(
            error.contains("set \"Output encoding\" to base64"),
            "error was: {error}"
        );
    }

    #[test]
    fn decompression_bomb_is_stopped() {
        // MAX_INPUT_BYTES is 64 KiB in unit tests; the 32 MiB limit itself is the same code.
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
        std::io::Write::write_all(&mut encoder, &vec![0u8; MAX_INPUT_BYTES + 1]).unwrap();
        let bomb = encoder.finish().unwrap();
        let error = decompress(&bomb).unwrap_err();
        assert!(error.contains("more than"), "error was: {error}");
    }
}
