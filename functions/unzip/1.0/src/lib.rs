mod bindings {
    wit_bindgen::generate!({ generate_all });

    use crate::Unzip;
    export!(Unzip);
}

mod archive;
mod download;

use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::Value;

use crate::bindings::betty_blocks_types::data_api::data_api::HelperContext;
use crate::bindings::betty_blocks_types::types::types::{BettyModel, BettyProperty};
use crate::bindings::betty_blocks_types::upload_file::upload_file;
use crate::bindings::exports::betty_blocks::unzip::unzip::Guest;

/// The most bytes this step handles in one run: the ZIP's size, and separately the extracted
/// files' total size. A guard against running out of memory and against ZIP bombs; not a
/// measured platform limit.
pub const MAX_TOTAL_BYTES: usize = 64 * 1024 * 1024;

/// A size in MB (of 1024 x 1024 bytes).
fn megabytes(bytes: u64) -> String {
    format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
}

/// The number between `<tag>` and `</tag>` in the storage's XML error, if there is one.
fn xml_number(error: &str, tag: &str) -> Option<u64> {
    let start = error.find(&format!("<{tag}>"))? + tag.len() + 2;
    let end = start + error[start..].find('<')?;
    error[start..end].trim().parse().ok()
}

/// The message for a failed upload. The file storage refuses a file larger than the maximum file
/// size set on the target file property in the data model, with S3's `EntityTooLarge` error and the
/// sizes in it (seen on a canvas, 2026-11-24); that case is put in plain words. Any other error is
/// passed on with `hint`.
fn upload_error(what: &str, error: &str, property_label: &str, hint: &str) -> String {
    if !error.contains("EntityTooLarge") {
        return format!("storing {what} failed: {error} — {hint}");
    }
    let sizes = match (
        xml_number(error, "ProposedSize"),
        xml_number(error, "MaxSizeAllowed"),
    ) {
        (Some(size), Some(max)) => {
            format!(" ({} while it accepts {})", megabytes(size), megabytes(max))
        }
        _ => String::new(),
    };
    format!(
        "storing {what} failed: it is larger than the maximum file size of \"{property_label}\"{sizes} — raise that property's maximum file size in the data model, or use smaller files"
    )
}

/// The URL of the ZIP file. A file value arrives as JSON: an object with a `url` (a file
/// property's value), or a JSON string with the URL (a text variable). Plain text is accepted too.
fn zip_url(zip_file: &str) -> Result<String, String> {
    let url = match serde_json::from_str::<Value>(zip_file) {
        Ok(Value::Object(fields)) => fields
            .get("url")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        Ok(Value::String(url)) => url,
        Ok(Value::Null) => String::new(),
        Ok(other) => {
            return Err(format!(
                "\"ZIP file\" must be a file or a URL, found {other} — select a file property or a variable that holds one"
            ));
        }
        Err(_) => zip_file.trim().to_string(),
    };
    if url.is_empty() {
        return Err(String::from(
            "No ZIP file was found: \"ZIP file\" is empty or has no URL — check that the record has a file in that property",
        ));
    }
    Ok(url)
}

fn run(
    helper_context: HelperContext,
    zip_file: String,
    model_target: BettyModel,
    property_target: Vec<BettyProperty>,
) -> Result<String, String> {
    let target = property_target.into_iter().next().ok_or_else(|| {
        String::from(
            "\"Property to save the extracted files into\" is empty — select a file property",
        )
    })?;
    let url = zip_url(&zip_file)?;
    let mut budget = MAX_TOTAL_BYTES;
    let bytes = wstd::runtime::block_on(download::download(
        &wstd::http::Client::new(),
        &url,
        &mut budget,
    ))?;

    let mut references = Vec::new();
    archive::for_each_file(bytes, MAX_TOTAL_BYTES, |name, content| {
        let result = upload_file::upload(
            &helper_context,
            &upload_file::Input {
                model: model_target.clone(),
                property: target.clone(),
                file_base64: STANDARD.encode(&content),
                full_filename: name.to_string(),
            },
        )
        .map_err(|error| {
            upload_error(
                &format!("\"{name}\""),
                &error,
                "Property to save the extracted files into",
                "check that \"Property to save the extracted files into\" is a file property of \"Model to save the extracted files into\"",
            )
        })?;
        references.push(Value::String(result.reference));
        Ok(())
    })?;
    Ok(Value::Array(references).to_string())
}

struct Unzip;

impl Guest for Unzip {
    fn unzip(
        helper_context: HelperContext,
        zip_file: String,
        model_target: BettyModel,
        property_target: Vec<BettyProperty>,
    ) -> Result<String, String> {
        run(helper_context, zip_file, model_target, property_target)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_url_from_a_file_value() {
        let value = r#"{"name":"a.zip","url":"https://x/a.zip?sig=1"}"#;
        assert_eq!(zip_url(value).unwrap(), "https://x/a.zip?sig=1");
        assert_eq!(zip_url(r#""https://x/a.zip""#).unwrap(), "https://x/a.zip");
        assert_eq!(zip_url(" https://x/a.zip ").unwrap(), "https://x/a.zip");
    }

    #[test]
    fn an_empty_value_is_a_clear_error() {
        for value in ["", "null", "{}", r#"{"url":""}"#, r#""""#] {
            let error = zip_url(value).unwrap_err();
            assert!(
                error.contains("No ZIP file was found"),
                "error was: {error}"
            );
        }
        let error = zip_url("[1]").unwrap_err();
        assert!(
            error.contains("must be a file or a URL"),
            "error was: {error}"
        );
    }
}
