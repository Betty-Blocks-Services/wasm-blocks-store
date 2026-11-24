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
    if compact.len() / 4 * 3 > MAX_INPUT_BYTES {
        return Err(too_large(compact.len() / 4 * 3));
    }
    BASE64.decode(compact.as_bytes()).map_err(|error| {
        let hint = if compact.contains(['-', '_']) {
            " The text contains \"-\" or \"_\", which suggests URL-safe base64: only standard base64 (with \"+\" and \"/\") is supported."
        } else {
            " If the data is raw gzip bytes passed as text, that cannot work: binary data has to arrive base64-encoded."
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
            "\"Data\" is not gzip data: it does not start with the gzip signature (1f 8b, \"H4sI\" in base64) — check that the value really is gzip-compressed and that \"Input encoding\" matches how it is encoded",
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

fn run(data: &str, input_encoding: &str, output_encoding: &str) -> Result<String, String> {
    let input_encoding = parse_encoding(input_encoding, "Input encoding", Encoding::Base64)?;
    let output_encoding = parse_encoding(output_encoding, "Output encoding", Encoding::Utf8)?;
    let input = decode_input(data, input_encoding)?;
    let decompressed = decompress(&input)?;
    encode_output(decompressed, output_encoding)
}

struct Gunzip;

impl Guest for Gunzip {
    fn gunzip(
        data: Option<String>,
        input_encoding: String,
        output_encoding: String,
    ) -> Result<String, String> {
        run(
            data.as_deref().unwrap_or_default(),
            &input_encoding,
            &output_encoding,
        )
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
        assert_eq!(run(HELLO_GZIP, "base64", "utf8").unwrap(), "hello");
    }

    #[test]
    fn empty_encodings_use_the_defaults() {
        assert_eq!(run(HELLO_GZIP, "", "").unwrap(), "hello");
    }

    #[test]
    fn base64_output_returns_the_raw_bytes_encoded() {
        assert_eq!(run(HELLO_GZIP, "base64", "base64").unwrap(), "aGVsbG8=");
    }

    #[test]
    fn concatenated_members_are_all_decompressed() {
        let member = BASE64.decode(HELLO_GZIP).unwrap();
        let twice = BASE64.encode([member.clone(), member].concat());
        assert_eq!(run(&twice, "base64", "utf8").unwrap(), "hellohello");
    }

    #[test]
    fn trailing_nul_padding_is_ignored() {
        let mut bytes = BASE64.decode(HELLO_GZIP).unwrap();
        bytes.extend([0u8; 512]);
        assert_eq!(
            run(&BASE64.encode(bytes), "base64", "utf8").unwrap(),
            "hello"
        );
    }

    #[test]
    fn trailing_garbage_is_ignored() {
        let mut bytes = BASE64.decode(HELLO_GZIP).unwrap();
        bytes.extend(b"junk");
        assert_eq!(
            run(&BASE64.encode(bytes), "base64", "utf8").unwrap(),
            "hello"
        );
    }

    #[test]
    fn a_broken_second_member_is_still_an_error() {
        let mut bytes = BASE64.decode(HELLO_GZIP).unwrap();
        bytes.extend([0x1f, 0x8b, 0x08]);
        let error = run(&BASE64.encode(bytes), "base64", "utf8").unwrap_err();
        assert!(
            error.contains("not a valid gzip stream"),
            "error was: {error}"
        );
    }

    #[test]
    fn empty_data_is_a_clear_error() {
        for (data, encoding) in [("", "base64"), ("  \n", "base64"), ("", "utf8")] {
            let error = run(data, encoding, "utf8").unwrap_err();
            assert!(error.contains("\"Data\" is empty"), "error was: {error}");
        }
    }

    #[test]
    fn non_gzip_input_is_a_clear_error() {
        let error = run(&BASE64.encode("<xml/>"), "base64", "utf8").unwrap_err();
        assert!(error.contains("not gzip data"), "error was: {error}");
    }

    #[test]
    fn plain_text_input_is_a_clear_error() {
        let error = run("<xml/>", "utf8", "utf8").unwrap_err();
        assert!(error.contains("not gzip data"), "error was: {error}");
    }

    #[test]
    fn truncated_gzip_is_a_clear_error() {
        let bytes = BASE64.decode(HELLO_GZIP).unwrap();
        let truncated = BASE64.encode(&bytes[..bytes.len() - 6]);
        let error = run(&truncated, "base64", "utf8").unwrap_err();
        assert!(
            error.contains("not a valid gzip stream"),
            "error was: {error}"
        );
    }

    #[test]
    fn invalid_base64_is_a_clear_error() {
        let error = run("H4sI***", "base64", "utf8").unwrap_err();
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
