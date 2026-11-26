//! Downloads the import file over `wasi:http`.

use waki::Client;

const MAX_REDIRECTS: usize = 5;

/// Checks the URL before anything else runs: it must be an http(s) URL.
pub fn check_url(url: &str) -> Result<&str, String> {
    let url = url.trim();
    if url.is_empty() {
        return Err(String::from(
            "The URL to the import file is empty: use the URL of a file, e.g. the url of a file property",
        ));
    }
    let lower = url.to_ascii_lowercase();
    if !lower.starts_with("https://") && !lower.starts_with("http://") {
        return Err(String::from(
            "The URL to the import file must start with https:// (or http://)",
        ));
    }
    Ok(url)
}

/// `url` with the characters a URI may not contain percent-encoded: spaces, non-ASCII, control
/// characters, double quotes, angle and curly brackets, backslash, caret, backtick and pipe. A file
/// URL can hold the file name as it is (`files/My import.csv`); a browser and the ActionsJS step's
/// `fetch` encode it, `waki` refuses it ("invalid uri character"). `%` and the reserved characters
/// (`/?#&=` and so on) are kept, so an already encoded URL does not change. A `%` that does not
/// start an escape (`50% off.csv`) is encoded. A `#` or `?` in a file name cannot be told apart from
/// a fragment or query, so it is kept.
fn encode(url: &str) -> String {
    let bytes = url.as_bytes();
    let mut encoded = String::with_capacity(url.len());
    for (index, &byte) in bytes.iter().enumerate() {
        let escape = byte == b'%'
            && bytes.get(index + 1).is_some_and(u8::is_ascii_hexdigit)
            && bytes.get(index + 2).is_some_and(u8::is_ascii_hexdigit);
        if byte.is_ascii_graphic() && !b"\"<>\\^`{|}".contains(&byte) && (byte != b'%' || escape) {
            encoded.push(byte as char);
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

/// The URL a redirect points to: `location` as it is when it is absolute, else resolved against
/// the scheme and host of `from`.
fn resolve(from: &str, location: &str) -> String {
    let lower = location.to_ascii_lowercase();
    if lower.starts_with("https://") || lower.starts_with("http://") {
        return location.to_string();
    }
    let (scheme, rest) = from.split_once("://").unwrap_or(("https", from));
    let rest = rest.split(['?', '#']).next().unwrap_or(rest);
    let host = rest.split('/').next().unwrap_or(rest);
    if let Some(path) = location.strip_prefix("//") {
        return format!("{scheme}://{path}");
    }
    if location.starts_with('?') {
        return format!("{scheme}://{rest}{location}");
    }
    if location.starts_with('/') {
        return format!("{scheme}://{host}{location}");
    }
    let base = match rest.rfind('/') {
        Some(index) if index > host.len() => &rest[..index],
        _ => host,
    };
    format!("{scheme}://{base}/{location}")
}

/// The body of the file at `url`, following up to five redirects.
pub fn download(url: &str) -> Result<Vec<u8>, String> {
    let mut current = url.to_string();
    for _ in 0..=MAX_REDIRECTS {
        let response = Client::new().get(&encode(&current)).send().map_err(|error| {
            format!(
                "Downloading the import file failed: {error}. Check that the URL is correct and reachable"
            )
        })?;
        let status = response.status_code();
        if (300..400).contains(&status) {
            let location = response
                .header("location")
                .and_then(|value| value.to_str().ok())
                .ok_or_else(|| {
                    format!("Downloading the import file failed: HTTP {status} without a Location")
                })?;
            current = resolve(&current, location);
            continue;
        }
        if !(200..300).contains(&status) {
            return Err(format!(
                "Downloading the import file failed with HTTP {status}. Check that the URL is correct and still valid (a signed file URL expires)"
            ));
        }
        return response
            .body()
            .map_err(|error| format!("Reading the downloaded import file failed: {error}"));
    }
    Err(format!(
        "Downloading the import file failed: more than {MAX_REDIRECTS} redirects"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_must_be_http() {
        assert_eq!(
            check_url(" https://x.nl/a.csv ").unwrap(),
            "https://x.nl/a.csv"
        );
        assert!(check_url("").is_err());
        assert!(check_url("ftp://x").is_err());
        assert!(check_url("/public/a.csv").is_err());
    }

    #[test]
    fn characters_a_uri_may_not_contain_are_encoded() {
        assert_eq!(
            encode("https://x.nl/files/CSV import special characters"),
            "https://x.nl/files/CSV%20import%20special%20characters"
        );
        assert_eq!(encode("https://x.nl/é.csv"), "https://x.nl/%C3%A9.csv");
        assert_eq!(
            encode("https://x.nl/a%20b.csv?x=1&y=[2]#z"),
            "https://x.nl/a%20b.csv?x=1&y=[2]#z"
        );
        assert_eq!(
            encode("https://x.nl/50% off.csv"),
            "https://x.nl/50%25%20off.csv"
        );
        assert_eq!(encode("https://x.nl/a%2"), "https://x.nl/a%252");
    }

    #[test]
    fn redirects_are_resolved() {
        let from = "https://a.nl/dir/file.csv?x=1";
        assert_eq!(resolve(from, "https://b.nl/f"), "https://b.nl/f");
        assert_eq!(resolve(from, "/other"), "https://a.nl/other");
        assert_eq!(resolve(from, "//c.nl/f"), "https://c.nl/f");
        assert_eq!(resolve(from, "next.csv"), "https://a.nl/dir/next.csv");
        assert_eq!(resolve("https://a.nl", "f"), "https://a.nl/f");
        assert_eq!(
            resolve("https://a.nl/dl?next=/x/y", "f.csv"),
            "https://a.nl/f.csv"
        );
        assert_eq!(resolve(from, "?x=2"), "https://a.nl/dir/file.csv?x=2");
    }
}
