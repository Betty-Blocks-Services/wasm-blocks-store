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

/// The URL a redirect points to: `location` as it is when it is absolute, else resolved against
/// the scheme and host of `from`.
fn resolve(from: &str, location: &str) -> String {
    let lower = location.to_ascii_lowercase();
    if lower.starts_with("https://") || lower.starts_with("http://") {
        return location.to_string();
    }
    let (scheme, rest) = from.split_once("://").unwrap_or(("https", from));
    let host = rest.split(['/', '?', '#']).next().unwrap_or(rest);
    if let Some(path) = location.strip_prefix("//") {
        return format!("{scheme}://{path}");
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
        let response = Client::new().get(&current).send().map_err(|error| {
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
    fn redirects_are_resolved() {
        let from = "https://a.nl/dir/file.csv?x=1";
        assert_eq!(resolve(from, "https://b.nl/f"), "https://b.nl/f");
        assert_eq!(resolve(from, "/other"), "https://a.nl/other");
        assert_eq!(resolve(from, "//c.nl/f"), "https://c.nl/f");
        assert_eq!(resolve(from, "next.csv"), "https://a.nl/dir/next.csv");
        assert_eq!(resolve("https://a.nl", "f"), "https://a.nl/f");
    }
}
