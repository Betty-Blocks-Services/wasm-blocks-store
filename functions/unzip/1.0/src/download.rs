//! Downloads a file over wasi:http, the way the native send-mail and store-file steps do.

use wstd::http::{Client, Request};

/// Downloads `url` and returns its bytes. Fails when the body would make `budget` negative, so a
/// set of downloads cannot use more memory than the caller allows.
pub async fn download(client: &Client, url: &str, budget: &mut usize) -> Result<Vec<u8>, String> {
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err(format!(
            "the file URL \"{url}\" does not start with http:// or https://"
        ));
    }
    let request = Request::get(url)
        .body(())
        .map_err(|error| format!("could not build a request for \"{url}\": {error}"))?;
    let mut response = client
        .send(request)
        .await
        .map_err(|error| format!("downloading \"{url}\" failed: {error}"))?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!(
            "downloading \"{url}\" failed with HTTP status {status}"
        ));
    }
    let body = response
        .body_mut()
        .contents()
        .await
        .map_err(|error| format!("reading the download of \"{url}\" failed: {error}"))?;
    *budget = budget.checked_sub(body.len()).ok_or_else(|| {
        format!(
            "the files are larger than the {} bytes this step can handle in one run",
            crate::MAX_TOTAL_BYTES
        )
    })?;
    Ok(body.to_vec())
}
