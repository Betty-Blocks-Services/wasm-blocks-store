mod bindings {
    wit_bindgen::generate!({ generate_all });

    use crate::HashText;
    export!(HashText);
}

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use hmac::{Hmac, Mac};
use md5::Md5;
use sha1::Sha1;
use sha2::{Digest, Sha256, Sha384, Sha512};

use crate::bindings::exports::betty_blocks::hash_text::hash_text::Guest;

struct HashText;

/// The hash functions of the classic expressions (sha1, sha256, sha384, sha512, md5,
/// hmac_sha256, hmac_sha384, hmac_sha512), with the same output for the same input.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Algorithm {
    Sha256,
    Sha384,
    Sha512,
    Sha1,
    Md5,
    HmacSha256,
    HmacSha384,
    HmacSha512,
}

impl Algorithm {
    fn label(self) -> &'static str {
        match self {
            Algorithm::Sha256 => "SHA-256",
            Algorithm::Sha384 => "SHA-384",
            Algorithm::Sha512 => "SHA-512",
            Algorithm::Sha1 => "SHA-1",
            Algorithm::Md5 => "MD5",
            Algorithm::HmacSha256 => "HMAC-SHA256",
            Algorithm::HmacSha384 => "HMAC-SHA384",
            Algorithm::HmacSha512 => "HMAC-SHA512",
        }
    }

    fn uses_key(self) -> bool {
        matches!(
            self,
            Algorithm::HmacSha256 | Algorithm::HmacSha384 | Algorithm::HmacSha512
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum OutputEncoding {
    Hex,
    Base64,
}

const ALGORITHM_VALUES: &str = "\"sha256\", \"sha384\", \"sha512\", \"sha1\", \"md5\", \"hmac-sha256\", \"hmac-sha384\" or \"hmac-sha512\"";

fn parse_algorithm(value: &str) -> Result<Algorithm, String> {
    // Also accepts the classic expression names (hmac_sha256) and dashed spellings (sha-256).
    let normalized = value.trim().to_ascii_lowercase().replace(['_', '-'], "");
    match normalized.as_str() {
        "sha256" => Ok(Algorithm::Sha256),
        "sha384" => Ok(Algorithm::Sha384),
        "sha512" => Ok(Algorithm::Sha512),
        "sha1" => Ok(Algorithm::Sha1),
        "md5" => Ok(Algorithm::Md5),
        "hmacsha256" => Ok(Algorithm::HmacSha256),
        "hmacsha384" => Ok(Algorithm::HmacSha384),
        "hmacsha512" => Ok(Algorithm::HmacSha512),
        _ => Err(format!(
            "\"Algorithm\" must be {ALGORITHM_VALUES}, found \"{}\" — pick one of the values in the step's \"Algorithm\" option",
            value.trim()
        )),
    }
}

fn parse_output_encoding(value: &str) -> Result<OutputEncoding, String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "" | "hex" => Ok(OutputEncoding::Hex),
        "base64" => Ok(OutputEncoding::Base64),
        other => Err(format!(
            "\"Output encoding\" must be \"hex\" or \"base64\", found \"{other}\" — pick one of the two values in the step's \"Output encoding\" option"
        )),
    }
}

fn to_hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut hex = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        hex.push(DIGITS[usize::from(byte >> 4)] as char);
        hex.push(DIGITS[usize::from(byte & 0x0f)] as char);
    }
    hex
}

fn hmac<M: Mac + hmac::digest::KeyInit>(key: &str, text: &str) -> Vec<u8> {
    // HMAC takes a key of any length (a long key is hashed first), so this cannot fail.
    let mut mac = <M as Mac>::new_from_slice(key.as_bytes()).expect("HMAC accepts any key length");
    mac.update(text.as_bytes());
    mac.finalize().into_bytes().to_vec()
}

fn hash_text(
    algorithm: &str,
    text: &str,
    secret_key: &str,
    output_encoding: &str,
) -> Result<String, String> {
    // Validate every option before doing any work, so a misconfiguration is reported first.
    let algorithm = parse_algorithm(algorithm)?;
    let output_encoding = parse_output_encoding(output_encoding)?;
    let name = algorithm.label();
    if algorithm.uses_key() && secret_key.is_empty() {
        return Err(format!(
            "\"Secret key\" is empty, but {name} needs a key — fill in \"Secret key\" (check that the variable you use has a value), or choose a hash without HMAC"
        ));
    }
    if !algorithm.uses_key() && !secret_key.is_empty() {
        return Err(format!(
            "\"Secret key\" is filled in, but {name} does not use a key — choose an HMAC algorithm, or leave \"Secret key\" empty"
        ));
    }

    let text_bytes = text.as_bytes();
    let digest: Vec<u8> = match algorithm {
        Algorithm::Sha256 => Sha256::digest(text_bytes).to_vec(),
        Algorithm::Sha384 => Sha384::digest(text_bytes).to_vec(),
        Algorithm::Sha512 => Sha512::digest(text_bytes).to_vec(),
        Algorithm::Sha1 => Sha1::digest(text_bytes).to_vec(),
        Algorithm::Md5 => Md5::digest(text_bytes).to_vec(),
        Algorithm::HmacSha256 => hmac::<Hmac<Sha256>>(secret_key, text),
        Algorithm::HmacSha384 => hmac::<Hmac<Sha384>>(secret_key, text),
        Algorithm::HmacSha512 => hmac::<Hmac<Sha512>>(secret_key, text),
    };

    Ok(match output_encoding {
        OutputEncoding::Hex => to_hex(&digest),
        OutputEncoding::Base64 => STANDARD.encode(digest),
    })
}

impl Guest for HashText {
    fn hash_text(
        algorithm: String,
        text: Option<String>,
        secret_key: Option<String>,
        output_encoding: String,
    ) -> Result<String, String> {
        hash_text(
            &algorithm,
            text.as_deref().unwrap_or_default(),
            secret_key.as_deref().unwrap_or_default(),
            &output_encoding,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_matches_the_fips_180_test_vectors() {
        assert_eq!(
            hash_text("sha256", "abc", "", "hex").unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            hash_text("sha256", "", "", "hex").unwrap(),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn sha1_matches_the_fips_180_test_vector() {
        assert_eq!(
            hash_text("sha1", "abc", "", "hex").unwrap(),
            "a9993e364706816aba3e25717850c26c9cd0d89d"
        );
    }

    #[test]
    fn hmac_sha256_matches_rfc_4231_test_case_2() {
        assert_eq!(
            hash_text("hmac-sha256", "what do ya want for nothing?", "Jefe", "hex").unwrap(),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
    }

    /// The examples of the classic expression reference
    /// (docs.bettyblocks.com/classic-docs/classic-expression-reference), so the step gives the
    /// same output as the classic expressions.
    #[test]
    fn matches_the_classic_expression_reference_examples() {
        let text = "Betty Blocks";
        for (algorithm, key, expected) in [
            ("sha1", "", "b6a6208882a9ab2e097259a2534f7c126bbada4d"),
            (
                "sha256",
                "",
                "51588c6d5db3a3bcf1a323627f8f6839ed04bf781e5796a855719f7249e6a66c",
            ),
            (
                "sha384",
                "",
                "065e413175c0d0989d94982cea6193f7a2bed7acc6a693e0e4bcebc3d1dd7db084309e015f65d231f7539b812b56bb18",
            ),
            (
                "sha512",
                "",
                "619a52f61ccc91a59dbb162c4a203ca544ae02b06f81ba54cc3c055828ee747f48c911df9d9d35cef07264fcd35a610bf8fdfe141a80bb8d848e001469a6c4c0",
            ),
            ("md5", "", "f6e8a2baa494d57bd9c8b053f1fba164"),
            (
                "hmac-sha256",
                "secret",
                "ec1d3a29ae219e55fa3c3a3055a25b6ba514e0acadcb1d198798dc48dc978a66",
            ),
            (
                "hmac-sha384",
                "secret",
                "4b642d2c4b3ca8349714cfa4d8bed17570c94284e06e07889d9fe6b0c0c2cd9a37e91d684632803943535d149b582382",
            ),
            (
                "hmac-sha512",
                "secret",
                "14dd565a61c4ac3aab353edae6f81dc9225c99a90f0e138bea494c359e3ee40cbf9f581ecc57205722e8d6ed22c0748d65685e98dcd90e3264b4392d4bdbed48",
            ),
        ] {
            assert_eq!(
                hash_text(algorithm, text, key, "hex").unwrap(),
                expected,
                "{algorithm}"
            );
        }
    }

    #[test]
    fn accepts_the_classic_expression_names() {
        assert_eq!(
            hash_text("hmac_sha256", "what do ya want for nothing?", "Jefe", "").unwrap(),
            hash_text("hmac-sha256", "what do ya want for nothing?", "Jefe", "").unwrap()
        );
        assert!(hash_text("SHA-512", "x", "", "").is_ok());
    }

    #[test]
    fn hmac_takes_a_key_longer_than_the_block_size() {
        // A 131-character key, longer than SHA-256's 64-byte block, so HMAC hashes it first.
        // Expected value: `printf x | openssl dgst -sha256 -hmac <131 times "a">`.
        let key = "a".repeat(131);
        assert_eq!(
            hash_text("hmac-sha256", "x", &key, "hex").unwrap(),
            "76f0801bdbc0ff29214ffdd6edce36467d4142a842d91a011c59c66d12693db0"
        );
    }

    #[test]
    fn hashes_the_utf8_bytes_of_the_text() {
        // `printf 'café' | shasum -a 256`
        assert_eq!(
            hash_text("sha256", "café", "", "hex").unwrap(),
            "850f7dc43910ff890f8879c0ed26fe697c93a067ad93a7d50f466a7028a9bf4e"
        );
    }

    #[test]
    fn base64_output_is_standard_padded_base64() {
        assert_eq!(
            hash_text("sha256", "abc", "", "base64").unwrap(),
            "ungWv48Bz+pBQUDeXa4iI7ADYaOWF3qctBD/YfIAFa0="
        );
    }

    #[test]
    fn empty_output_encoding_defaults_to_hex() {
        assert_eq!(
            hash_text("sha1", "abc", "", "").unwrap(),
            "a9993e364706816aba3e25717850c26c9cd0d89d"
        );
    }

    #[test]
    fn hmac_without_a_key_is_refused() {
        let error = hash_text("hmac-sha256", "message", "", "hex").unwrap_err();
        assert!(error.contains("\"Secret key\" is empty"), "{error}");
    }

    #[test]
    fn a_key_with_a_plain_hash_is_refused() {
        let error = hash_text("sha256", "message", "key", "hex").unwrap_err();
        assert!(error.contains("SHA-256 does not use a key"), "{error}");
    }

    #[test]
    fn unknown_values_name_the_option() {
        let error = hash_text("crc32", "message", "", "hex").unwrap_err();
        assert!(error.contains("\"Algorithm\""), "{error}");
        assert!(error.contains("\"crc32\""), "{error}");

        let error = hash_text("sha256", "message", "", "base32").unwrap_err();
        assert!(error.contains("\"Output encoding\""), "{error}");
    }

    #[test]
    fn an_invalid_output_encoding_is_reported_before_a_missing_key() {
        let error = hash_text("hmac-sha256", "message", "", "base32").unwrap_err();
        assert!(error.contains("\"Output encoding\""), "{error}");
    }
}
