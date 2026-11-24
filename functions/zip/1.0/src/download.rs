//! Downloads a file over wasi:http, the way the native send-mail and store-file steps do.

use wstd::http::{BodyExt, Client, Request};

/// `url` without its query string and fragment, for error messages. A file URL from the file
/// storage is signed in its query string, a temporary credential that must not end up in logs.
pub fn display_url(url: &str) -> &str {
    url.split(['?', '#']).next().unwrap_or(url)
}

fn too_large() -> String {
    format!(
        "the files are larger than the {} bytes this step can handle in one run",
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
            "downloading \"{shown}\" failed with HTTP status {status}"
        ));
    }
    let body = response.into_body();
    if body
        .content_length()
        .is_some_and(|length| length > *budget as u64)
    {
        return Err(too_large());
    }
    let mut body = body.into_boxed_body();
    let mut bytes = Vec::new();
    while let Some(frame) = body.frame().await {
        let frame = frame
            .map_err(|error| format!("reading the download of \"{shown}\" failed: {error}"))?;
        if let Ok(chunk) = frame.into_data() {
            *budget = budget.checked_sub(chunk.len()).ok_or_else(too_large)?;
            bytes.extend_from_slice(&chunk);
        }
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_signature_is_left_out_of_messages() {
        assert_eq!(
            display_url("https://x/a.zip?X-Amz-Signature=secret#f"),
            "https://x/a.zip"
        );
        assert_eq!(display_url("https://x/a.zip"), "https://x/a.zip");
    }
}
