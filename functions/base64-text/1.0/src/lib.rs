mod bindings {
    wit_bindgen::generate!({ generate_all });

    use crate::Base64Text;
    export!(Base64Text);
}

use std::borrow::Cow;

use base64::Engine;
use base64::alphabet::{STANDARD, URL_SAFE};
use base64::engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig};

use crate::bindings::exports::betty_blocks::base64_text::base64_text::Guest;

struct Base64Text;

/// Standard base64 (RFC 4648 section 4: `+` and `/`). Output is padded, like the classic
/// base64_encode expression; input is accepted with or without padding, because some systems drop
/// the trailing `=`. Leftover bits in the last character are ignored (`Zh==` decodes to `f`), as
/// lenient decoders do, instead of refusing the value.
const STANDARD_ENGINE: GeneralPurpose = GeneralPurpose::new(
    &STANDARD,
    GeneralPurposeConfig::new()
        .with_encode_padding(true)
        .with_decode_padding_mode(DecodePaddingMode::Indifferent)
        .with_decode_allow_trailing_bits(true),
);

/// URL-safe base64 (RFC 4648 section 5: `-` and `_`). Output has no padding, as in JWTs and URL
/// parameters; input is accepted with or without padding, and with leftover bits.
const URL_SAFE_ENGINE: GeneralPurpose = GeneralPurpose::new(
    &URL_SAFE,
    GeneralPurposeConfig::new()
        .with_encode_padding(false)
        .with_decode_padding_mode(DecodePaddingMode::Indifferent)
        .with_decode_allow_trailing_bits(true),
);

#[derive(Clone, Copy, Debug, PartialEq)]
enum Mode {
    Encode,
    Decode,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Alphabet {
    Standard,
    UrlSafe,
}

fn parse_mode(value: &str) -> Result<Mode, String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "encode" => Ok(Mode::Encode),
        "decode" => Ok(Mode::Decode),
        other => Err(format!(
            "\"Mode\" must be \"encode\" or \"decode\", found \"{other}\" — pick one of the two values in the step's \"Mode\" option"
        )),
    }
}

fn parse_alphabet(value: &str) -> Result<Alphabet, String> {
    match value.trim().to_ascii_lowercase().replace('_', "-").as_str() {
        "" | "standard" => Ok(Alphabet::Standard),
        "url-safe" | "urlsafe" => Ok(Alphabet::UrlSafe),
        other => Err(format!(
            "\"Alphabet\" must be \"standard\" or \"url-safe\", found \"{other}\" — pick one of the two values in the step's \"Alphabet\" option"
        )),
    }
}

/// Whether `text` only has base64 characters: letters, digits, `=` and the two of `extra`.
fn only_base64(text: &str, extra: [char; 2]) -> bool {
    text.chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '=' || extra.contains(&c))
}

/// What to check when `text` does not decode. The alphabet hint is only given when the text is
/// base64 of the other alphabet, so plain text with a `-` in it is not sent the wrong way.
fn hint(text: &str, alphabet: Alphabet) -> &'static str {
    let parts = text.split('.').collect::<Vec<_>>();
    if parts.len() == 3
        && parts[..2]
            .iter()
            .all(|part| !part.is_empty() && only_base64(part, ['-', '_']))
    {
        return " It looks like a whole JWT (three parts separated by \".\"): decode only the middle part, the payload, with \"Alphabet\" url-safe.";
    }
    match alphabet {
        Alphabet::Standard if text.contains(['-', '_']) && only_base64(text, ['-', '_']) => {
            " The text contains \"-\" or \"_\", which means URL-safe base64: set \"Alphabet\" to url-safe."
        }
        Alphabet::UrlSafe if text.contains(['+', '/']) && only_base64(text, ['+', '/']) => {
            " The text contains \"+\" or \"/\", which means standard base64: set \"Alphabet\" to standard."
        }
        _ => " Check that \"Text\" holds base64 and not plain text, or set \"Mode\" to encode.",
    }
}

fn decode(text: &str, alphabet: Alphabet) -> Result<String, String> {
    // Line breaks and other whitespace are ignored: MIME-style base64 wraps at 76 characters, and
    // copied values often end in a newline.
    let compact: Cow<str> = if text.contains(|c: char| c.is_ascii_whitespace()) {
        Cow::Owned(text.split_ascii_whitespace().collect())
    } else {
        Cow::Borrowed(text)
    };
    let engine = match alphabet {
        Alphabet::Standard => &STANDARD_ENGINE,
        Alphabet::UrlSafe => &URL_SAFE_ENGINE,
    };
    let bytes = engine.decode(compact.as_bytes()).map_err(|error| {
        format!(
            "\"Text\" is not valid base64: {error}.{}",
            hint(&compact, alphabet)
        )
    })?;
    String::from_utf8(bytes).map_err(|_| {
        String::from(
            "the decoded data is not UTF-8 text, so it cannot be returned as text — it is probably a binary file such as a PDF or an image. Keep it as base64, e.g. to store it with a file step",
        )
    })
}

fn base64_text(mode: &str, text: &str, alphabet: &str) -> Result<String, String> {
    // Validate every option before doing any work, so a misconfiguration is reported first.
    let mode = parse_mode(mode)?;
    let alphabet = parse_alphabet(alphabet)?;
    match mode {
        Mode::Encode => Ok(match alphabet {
            Alphabet::Standard => STANDARD_ENGINE.encode(text),
            Alphabet::UrlSafe => URL_SAFE_ENGINE.encode(text),
        }),
        Mode::Decode => decode(text, alphabet),
    }
}

impl Guest for Base64Text {
    fn base64_text(mode: String, text: Option<String>, alphabet: String) -> Result<String, String> {
        base64_text(&mode, text.as_deref().unwrap_or_default(), &alphabet)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_the_rfc_4648_test_vectors() {
        for (text, expected) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(base64_text("encode", text, "standard").unwrap(), expected);
            assert_eq!(base64_text("decode", expected, "standard").unwrap(), text);
        }
    }

    #[test]
    fn matches_the_classic_expression_reference_example() {
        // base64_encode("test") >>> "dGVzdA==" and base64_decode(var:base64, "string") >>> "test"
        // (docs.bettyblocks.com/classic-docs/classic-expression-reference).
        assert_eq!(
            base64_text("encode", "test", "standard").unwrap(),
            "dGVzdA=="
        );
        assert_eq!(
            base64_text("decode", "dGVzdA==", "standard").unwrap(),
            "test"
        );
    }

    #[test]
    fn encodes_the_utf8_bytes_of_the_text() {
        // `printf 'café ✓' | base64`
        assert_eq!(
            base64_text("encode", "café ✓", "standard").unwrap(),
            "Y2Fmw6kg4pyT"
        );
        assert_eq!(
            base64_text("decode", "Y2Fmw6kg4pyT", "standard").unwrap(),
            "café ✓"
        );
    }

    #[test]
    fn url_safe_uses_dash_and_underscore_without_padding() {
        // UTF-8 bytes ef af bf 3f: `printf '\xef\xaf\xbf?' | base64` gives "76+/Pw==".
        let text = "\u{fbff}?";
        assert_eq!(base64_text("encode", text, "standard").unwrap(), "76+/Pw==");
        assert_eq!(base64_text("encode", text, "url-safe").unwrap(), "76-_Pw");
        assert_eq!(base64_text("decode", "76-_Pw", "url-safe").unwrap(), text);
        assert_eq!(base64_text("decode", "76-_Pw==", "url-safe").unwrap(), text);
    }

    #[test]
    fn decoding_ignores_line_breaks_and_missing_padding() {
        assert_eq!(
            base64_text("decode", "Zm9v\r\nYmE\n", "standard").unwrap(),
            "fooba"
        );
    }

    #[test]
    fn empty_alphabet_defaults_to_standard() {
        assert_eq!(base64_text("encode", "f", "").unwrap(), "Zg==");
    }

    #[test]
    fn decoding_the_wrong_alphabet_says_which_one_to_pick() {
        let error = base64_text("decode", "76-_Pw", "standard").unwrap_err();
        assert!(error.contains("set \"Alphabet\" to url-safe"), "{error}");

        let error = base64_text("decode", "76+/Pw==", "url-safe").unwrap_err();
        assert!(error.contains("set \"Alphabet\" to standard"), "{error}");
    }

    #[test]
    fn plain_text_with_a_dash_is_not_sent_to_the_other_alphabet() {
        let error = base64_text("decode", "order-123 shipped!", "standard").unwrap_err();
        assert!(error.contains("set \"Mode\" to encode"), "{error}");
        assert!(!error.contains("url-safe"), "{error}");
    }

    #[test]
    fn a_whole_jwt_says_to_decode_the_payload() {
        let error = base64_text(
            "decode",
            "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0In0.c2ln",
            "url-safe",
        )
        .unwrap_err();
        assert!(error.contains("decode only the middle part"), "{error}");
        assert_eq!(
            base64_text("decode", "eyJzdWIiOiIxMjM0In0", "url-safe").unwrap(),
            "{\"sub\":\"1234\"}"
        );
    }

    #[test]
    fn leftover_bits_in_the_last_character_are_ignored() {
        assert_eq!(base64_text("decode", "Zh==", "standard").unwrap(), "f");
        assert_eq!(base64_text("decode", "Zh", "url-safe").unwrap(), "f");
    }

    #[test]
    fn decoding_plain_text_is_refused() {
        let error = base64_text("decode", "hello world!", "standard").unwrap_err();
        assert!(error.contains("\"Text\" is not valid base64"), "{error}");
        assert!(error.contains("set \"Mode\" to encode"), "{error}");
    }

    #[test]
    fn decoding_binary_data_is_refused() {
        // A PNG signature: 89 50 4e 47.
        let error = base64_text("decode", "iVBORw==", "standard").unwrap_err();
        assert!(error.contains("not UTF-8 text"), "{error}");
    }

    #[test]
    fn unknown_values_name_the_option() {
        let error = base64_text("reverse", "x", "standard").unwrap_err();
        assert!(error.contains("\"Mode\""), "{error}");

        let error = base64_text("encode", "x", "base32").unwrap_err();
        assert!(error.contains("\"Alphabet\""), "{error}");
    }
}
