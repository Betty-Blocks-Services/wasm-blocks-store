//! Downloads a file over wasi:http, the way the native send-mail and store-file steps do.

use wstd::http::{BodyExt, Client, Request};

/// `url` without its query string, fragment and `user:password@`, for error messages. A file URL
/// from the file storage is signed in its query string, a temporary credential that must not end
/// up in logs. A secret inside the path cannot be recognised and is still shown.
pub fn display_url(url: &str) -> String {
    let url = match url.split_once("://") {
        // `user:password@` goes first: a password may itself hold a `?` or `#`.
        Some((scheme, rest)) => {
            let authority_end = rest.find('/').unwrap_or(rest.len());
            let host_start = rest[..authority_end].rfind('@').map_or(0, |at| at + 1);
            format!("{scheme}://{}", &rest[host_start..])
        }
        None => url.to_string(),
    };
    url.split(['?', '#']).next().unwrap_or_default().to_string()
}

fn too_large(shown: &str, size: Option<u64>) -> String {
    let size = size
        .map(|size| format!(" ({size} bytes)"))
        .unwrap_or_default();
    format!(
        "downloading \"{shown}\"{size} goes over the {} bytes this step downloads in one run (all its downloads together) — use a smaller file, or fewer files",
        crate::MAX_TOTAL_BYTES
    )
}

/// Downloads `url` and returns its bytes. Fails when the body would make `budget` negative, so a
/// set of downloads cannot use more memory than the caller allows. A `Content-Length` over the
/// budget is refused before the body is read; without one, the body is read in chunks and the
/// download stops as soon as it passes the budget.
pub async fn download(client: &Client, url: &str, budget: &mut usize) -> Result<Vec<u8>, String> {
    let shown = display_url(url);
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err(format!(
            "the file URL \"{shown}\" does not start with http:// or https://"
        ));
    }
    let request = Request::get(url)
        .body(())
        .map_err(|error| format!("could not build a request for \"{shown}\": {error}"))?;
    let response = client
        .send(request)
        .await
        .map_err(|error| format!("downloading \"{shown}\" failed: {error}"))?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!(
            "downloading \"{shown}\" failed with HTTP status {status} — a file link from the file storage is signed and expires (often 403), so check that the file still exists and run the action again"
        ));
    }
    let body = response.into_body();
    if let Some(length) = body
        .content_length()
        .filter(|length| *length > *budget as u64)
    {
        return Err(too_large(&shown, Some(length)));
    }
    let mut body = body.into_boxed_body();
    let mut bytes = Vec::new();
    while let Some(frame) = body.frame().await {
        let frame = frame
            .map_err(|error| format!("reading the download of \"{shown}\" failed: {error}"))?;
        if let Ok(chunk) = frame.into_data() {
            *budget = budget
                .checked_sub(chunk.len())
                .ok_or_else(|| too_large(&shown, None))?;
            bytes.extend_from_slice(&chunk);
        }
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secrets_in_the_url_are_left_out_of_messages() {
        assert_eq!(
            display_url("https://x/a.zip?X-Amz-Signature=secret#f"),
            "https://x/a.zip"
        );
        assert_eq!(display_url("https://x/a.zip"), "https://x/a.zip");
        assert_eq!(
            display_url("https://user:secret@x.example/a.zip?s=1"),
            "https://x.example/a.zip"
        );
        assert_eq!(display_url("https://a@b@x.example"), "https://x.example");
        assert_eq!(display_url("not a url"), "not a url");
        assert_eq!(
            display_url("https://bob:p#ss?w@host.example/a.zip"),
            "https://host.example/a.zip"
        );
    }
}
