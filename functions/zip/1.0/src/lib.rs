mod bindings {
    wit_bindgen::generate!({ generate_all });

    use crate::Zip;
    export!(Zip);
}

mod download;
mod files;
mod names;

use std::io::{Cursor, Write};

use base64::{Engine, engine::general_purpose::STANDARD};
use zip::CompressionMethod;
use zip::write::{SimpleFileOptions, ZipWriter};

use crate::bindings::betty_blocks_types::data_api::data_api::HelperContext;
use crate::bindings::betty_blocks_types::types::types::{BettyModel, BettyProperty};
use crate::bindings::betty_blocks_types::upload_file::upload_file;
use crate::bindings::exports::betty_blocks::zip::zip::Guest;

/// The most bytes this step downloads in one run, summed over all files. A guard against running
/// out of memory; not a measured platform limit. Files are downloaded one at a time, so memory
/// holds the current download and the archive so far, and at the end the archive and its base64
/// copy for the upload (about 2.3 times the archive).
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

fn first_property(properties: Vec<BettyProperty>, label: &str) -> Result<BettyProperty, String> {
    properties
        .into_iter()
        .next()
        .ok_or_else(|| format!("\"{label}\" is empty — select a file property"))
}

/// The name the archive is stored under: the configured name (default `zip`), cleaned like the
/// names inside the archive (no folders, no control characters), with `.zip` added unless it
/// already ends in it.
fn archive_file_name(file_name: Option<&str>) -> String {
    let name = file_name.and_then(names::clean_file_name);
    let name = name.as_deref().unwrap_or("zip");
    if name.to_ascii_lowercase().ends_with(".zip") {
        name.to_string()
    } else {
        format!(
            "{}.zip",
            names::shorten(name, names::MAX_NAME_BYTES - ".zip".len())
        )
    }
}

fn run(
    helper_context: HelperContext,
    collection: String,
    property_source: Vec<BettyProperty>,
    model_target: BettyModel,
    property_target: Vec<BettyProperty>,
    file_name: Option<String>,
) -> Result<String, String> {
    let source = first_property(property_source, "Property to read files from")?;
    let target = first_property(property_target, "Property to save file into")?;
    let records = files::records_in_collection(&collection)?;
    let files = files::files_in_records(&records, &source.name)?;

    let options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .unix_permissions(0o644);
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    let mut budget = MAX_TOTAL_BYTES;
    let client = wstd::http::Client::new();
    // One file at a time: only the current download and the archive so far are in memory.
    for file in &files {
        let bytes = wstd::runtime::block_on(download::download(&client, &file.url, &mut budget))?;
        writer
            .start_file(file.entry_name.as_str(), options)
            .and_then(|()| writer.write_all(&bytes).map_err(Into::into))
            .map_err(|error| {
                format!("adding \"{}\" to the ZIP failed: {error}", file.entry_name)
            })?;
    }
    let archive = writer
        .finish()
        .map_err(|error| format!("finishing the ZIP failed: {error}"))?
        .into_inner();

    let result = upload_file::upload(
        &helper_context,
        &upload_file::Input {
            model: model_target,
            property: target,
            file_base64: STANDARD.encode(&archive),
            full_filename: archive_file_name(file_name.as_deref()),
        },
    )
    .map_err(|error| {
        upload_error(
            "the ZIP",
            &error,
            "Property to save file into",
            "check that \"Property to save file into\" is a file property of \"Model to save file into\"",
        )
    })?;
    Ok(result.reference)
}

struct Zip;

impl Guest for Zip {
    fn zip(
        helper_context: HelperContext,
        collection: String,
        property_source: Vec<BettyProperty>,
        model_target: BettyModel,
        property_target: Vec<BettyProperty>,
        file_name: Option<String>,
    ) -> Result<String, String> {
        run(
            helper_context,
            collection,
            property_source,
            model_target,
            property_target,
            file_name,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_size_refusal_is_explained_with_the_sizes() {
        let storage = "upload failed with status 400: <?xml version=\"1.0\"?><Error><Code>EntityTooLarge</Code><Message>Your proposed upload exceeds the maximum allowed size</Message><ProposedSize>14618555</ProposedSize><MaxSizeAllowed>10485760</MaxSizeAllowed></Error>";
        assert_eq!(
            upload_error("the ZIP", storage, "Property to save file into", "hint"),
            "storing the ZIP failed: it is larger than the maximum file size of \"Property to save file into\" (13.9 MB while it accepts 10.0 MB) — raise that property's maximum file size in the data model, or use smaller files"
        );
        let without_sizes = upload_error("the ZIP", "<Code>EntityTooLarge</Code>", "P", "hint");
        assert!(
            without_sizes.contains("maximum file size of \"P\" — raise"),
            "{without_sizes}"
        );
        assert_eq!(
            upload_error("the ZIP", "boom", "P", "hint"),
            "storing the ZIP failed: boom — hint"
        );
    }

    #[test]
    fn archive_name_gets_the_zip_extension_once() {
        assert_eq!(archive_file_name(None), "zip.zip");
        assert_eq!(archive_file_name(Some("  ")), "zip.zip");
        assert_eq!(archive_file_name(Some("invoices")), "invoices.zip");
        assert_eq!(archive_file_name(Some("invoices.ZIP")), "invoices.ZIP");
        assert_eq!(archive_file_name(Some("../../x")), "x.zip");
        assert_eq!(archive_file_name(Some("a\\b\u{0}c")), "bc.zip");
        assert_eq!(archive_file_name(Some("..")), "zip.zip");
        let long = archive_file_name(Some(&"a".repeat(500)));
        assert_eq!(long.len(), names::MAX_NAME_BYTES);
        assert!(long.ends_with("a.zip"), "{long}");
    }
}
