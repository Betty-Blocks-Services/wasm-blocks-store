//! Reads the files out of a ZIP archive.

use std::collections::HashSet;
use std::io::{Cursor, Read};

use zip::result::ZipError;
use zip::{CompressionMethod, ZipArchive};

use crate::names::{clean_file_name, unique_name};

/// The most files one ZIP may hold. Every file becomes its own upload, so this keeps a single run
/// bounded; not a measured platform limit. Folders and metadata files are not counted.
pub const MAX_FILES: usize = 1000;

/// The most entries (files, folders and metadata together) one ZIP may hold. Reading the list of
/// entries costs memory for every entry before any of them is looked at (about 270 bytes each,
/// measured natively by the PR review), so this is checked first. High enough that a ZIP of
/// uncompressed Office files, whose own entries are counted too, rarely reaches it.
pub const MAX_ENTRIES: usize = 50_000;

/// The signature that starts every entry in a ZIP's central directory (`PK\x01\x02`).
const ENTRY_SIGNATURE: [u8; 4] = [0x50, 0x4b, 0x01, 0x02];

/// macOS and Windows metadata that archivers add, which is not part of the content: anything in a
/// top-level `__MACOSX` folder, and files named `.DS_Store`, `Thumbs.db` (any case) or starting
/// with `._` (AppleDouble). Only the file name is checked, not the folders it is in. The ActionsJS
/// UNZIP step checked the whole path and matched `.DS_Store` and `Thumbs.db` anywhere in it.
fn is_metadata(path: &str) -> bool {
    let name = base_name(path);
    path.split(['/', '\\']).next() == Some("__MACOSX")
        || name.starts_with("._")
        || name == ".DS_Store"
        || name.eq_ignore_ascii_case("Thumbs.db")
}

/// The file name without the folders it sits in inside the archive.
fn base_name(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}

/// A folder entry: its name ends in a slash (the test the `zip` crate uses too).
fn is_folder(path: &str) -> bool {
    path.ends_with('/') || path.ends_with('\\')
}

/// At least the number of entries the archive's central directory can hold: every entry starts
/// with [`ENTRY_SIGNATURE`], and the `zip` crate checks that signature for each entry it reads.
/// Counts too many, never too few: the signature can also occur inside file content, for example
/// in a ZIP stored uncompressed inside this one.
fn entry_count_upper_bound(bytes: &[u8]) -> usize {
    bytes
        .windows(ENTRY_SIGNATURE.len())
        .filter(|window| *window == ENTRY_SIGNATURE)
        .count()
}

fn describe(error: ZipError) -> String {
    match error {
        ZipError::UnsupportedArchive(ZipError::PASSWORD_REQUIRED) => {
            String::from("the ZIP is password-protected, which this step does not support")
        }
        ZipError::UnsupportedArchive(message) => {
            format!("the ZIP uses a feature this step does not support: {message}")
        }
        other => format!("the file is not a valid ZIP archive: {other}"),
    }
}

/// The name and content of every file in the archive, in archive order. Folders and metadata
/// files are skipped without being opened. Names lose their folders and are made safe and unique
/// (`a.pdf`, `a_1.pdf`). Everything is extracted before anything is stored, so a broken archive
/// stores nothing. Stops with an error once the extracted content would pass `max_total_bytes`,
/// so a ZIP bomb cannot exhaust memory. A ZIP with entries but no files (only folders or metadata)
/// is an error; an empty ZIP gives no files.
pub fn extract_files(
    bytes: Vec<u8>,
    max_total_bytes: usize,
) -> Result<Vec<(String, Vec<u8>)>, String> {
    if entry_count_upper_bound(&bytes) > MAX_ENTRIES {
        return Err(format!(
            "the ZIP holds more than {MAX_ENTRIES} entries (files, folders and metadata together), more than this step reads in one run. ZIP, Office or other ZIP-based files stored uncompressed inside it count their own entries too; compressing the ZIP normally avoids that"
        ));
    }
    let mut archive = ZipArchive::new(Cursor::new(bytes)).map_err(describe)?;
    let mut remaining = max_total_bytes;
    let mut files = Vec::new();
    let mut taken = HashSet::new();
    for index in 0..archive.len() {
        let path = match archive.name_for_index(index) {
            Some(name) => name.map_err(describe)?.into_owned(),
            None => continue,
        };
        if is_folder(&path) || is_metadata(&path) {
            continue;
        }
        if files.len() == MAX_FILES {
            return Err(format!(
                "the ZIP holds more than the {MAX_FILES} files this step extracts in one run"
            ));
        }
        {
            // Read without decompressing, so an unsupported method gets a clear message. An
            // encrypted entry is refused by `by_index` below (see `describe`).
            let method = archive.by_index_raw(index).map_err(describe)?.compression();
            if !matches!(
                method,
                CompressionMethod::Stored | CompressionMethod::Deflated
            ) {
                return Err(format!(
                    "\"{path}\" in the ZIP is compressed with {method}, which this step does not support (only Stored and Deflate) — make the ZIP again with a standard ZIP tool"
                ));
            }
        }
        let mut entry = archive.by_index(index).map_err(describe)?;
        let mut content = Vec::new();
        (&mut entry)
            .take(remaining as u64 + 1)
            .read_to_end(&mut content)
            .map_err(|error| format!("extracting \"{path}\" failed: {error}"))?;
        remaining = remaining.checked_sub(content.len()).ok_or_else(|| {
            format!(
                "the extracted files are larger than the {max_total_bytes} bytes this step can handle in one run"
            )
        })?;
        let name = clean_file_name(&path).unwrap_or_else(|| String::from("file"));
        files.push((unique_name(&name, &mut taken), content));
    }
    if files.is_empty() && !archive.is_empty() {
        return Err(format!(
            "the ZIP holds {} entries but no files: all of them are folders or macOS/Windows metadata files, which this step skips",
            archive.len()
        ));
    }
    Ok(files)
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use zip::unstable::write::FileOptionsExt;
    use zip::write::{SimpleFileOptions, ZipWriter};

    use super::*;

    fn make_zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
        let options =
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        for (name, content) in entries {
            if name.ends_with('/') {
                writer.add_directory(*name, options).unwrap();
            } else {
                writer.start_file(*name, options).unwrap();
                writer.write_all(content).unwrap();
            }
        }
        writer.finish().unwrap().into_inner()
    }

    fn extract(bytes: Vec<u8>, limit: usize) -> Result<Vec<(String, Vec<u8>)>, String> {
        extract_files(bytes, limit)
    }

    fn names(files: &[(String, Vec<u8>)]) -> Vec<&str> {
        files.iter().map(|(name, _)| name.as_str()).collect()
    }

    /// A ZIP whose `encrypted` entries are encrypted with ZipCrypto, next to plain `entries`.
    fn make_encrypted_zip(entries: &[(&str, &[u8])], encrypted: &[(&str, &[u8])]) -> Vec<u8> {
        let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
        let options = SimpleFileOptions::default();
        for (name, content) in entries {
            writer.start_file(*name, options).unwrap();
            writer.write_all(content).unwrap();
        }
        let locked = options.with_deprecated_encryption(b"secret").unwrap();
        for (name, content) in encrypted {
            writer.start_file(*name, locked).unwrap();
            writer.write_all(content).unwrap();
        }
        writer.finish().unwrap().into_inner()
    }

    #[test]
    fn extracts_files_and_skips_folders_and_metadata() {
        let zip = make_zip(&[
            ("docs/", b""),
            ("docs/a.txt", b"hello"),
            ("b.xml", "<é/>".as_bytes()),
            ("__MACOSX/docs/._a.txt", b"meta"),
            ("docs/.DS_Store", b"meta"),
            ("._c.txt", b"meta"),
        ]);
        let files = extract(zip, 1024).unwrap();
        assert_eq!(
            files,
            [
                (String::from("a.txt"), b"hello".to_vec()),
                (String::from("b.xml"), "<é/>".as_bytes().to_vec()),
            ]
        );
    }

    #[test]
    fn an_empty_zip_has_no_files() {
        assert!(extract(make_zip(&[]), 1024).unwrap().is_empty());
    }

    #[test]
    fn not_a_zip_is_a_clear_error() {
        let error = extract(b"<xml/>".to_vec(), 1024).unwrap_err();
        assert!(error.contains("not a valid ZIP"), "error was: {error}");
    }

    #[test]
    fn a_zip_bomb_is_stopped() {
        let zip = make_zip(&[("big.bin", &[0u8; 10_000])]);
        let error = extract(zip, 9_999).unwrap_err();
        assert!(error.contains("larger than"), "error was: {error}");
    }

    #[test]
    fn the_limit_counts_all_files_together() {
        let zip = make_zip(&[("a", &[1u8; 600]), ("b", &[2u8; 600])]);
        assert!(extract(zip.clone(), 1200).is_ok());
        assert!(extract(zip, 1199).is_err());
    }

    #[test]
    fn metadata_is_matched_on_the_file_name() {
        for path in [
            "__MACOSX/docs/._a.txt",
            "._a.txt",
            "docs/._a.txt",
            "docs/.DS_Store",
            "Thumbs.db",
            "docs/thumbs.db",
        ] {
            assert!(is_metadata(path), "{path} should be metadata");
        }
        for path in [
            "Thumbs.db.xml",
            "x.DS_Store.pdf",
            "docs/__MACOSX/a.txt",
            "a._b.txt",
        ] {
            assert!(!is_metadata(path), "{path} should be kept");
        }
    }

    #[test]
    fn the_file_limit_counts_only_files() {
        let mut entries: Vec<(String, &[u8])> = vec![(String::from("docs/"), b"")];
        entries.extend((0..MAX_FILES).map(|n| (format!("docs/{n}.txt"), &b"x"[..])));
        entries.push((String::from("docs/.DS_Store"), b"meta"));
        let as_refs: Vec<(&str, &[u8])> = entries
            .iter()
            .map(|(name, content)| (name.as_str(), *content))
            .collect();
        assert_eq!(
            extract(make_zip(&as_refs), 1 << 20).unwrap().len(),
            MAX_FILES
        );

        entries.push((String::from("one-too-many.txt"), b"x"));
        let as_refs: Vec<(&str, &[u8])> = entries
            .iter()
            .map(|(name, content)| (name.as_str(), *content))
            .collect();
        let error = extract(make_zip(&as_refs), 1 << 20).unwrap_err();
        assert!(
            error.contains("more than the 1000 files"),
            "error was: {error}"
        );
    }

    #[test]
    fn too_many_entries_are_refused_before_the_archive_is_read() {
        let folders: Vec<String> = (0..MAX_ENTRIES).map(|n| format!("d{n}/")).collect();
        let mut entries: Vec<(&str, &[u8])> = folders
            .iter()
            .map(|name| (name.as_str(), &b""[..]))
            .collect();
        entries.pop();
        entries.push(("a.txt", b"x"));
        assert_eq!(
            names(&extract(make_zip(&entries), 1024).unwrap()),
            ["a.txt"]
        );

        entries.push(("d-extra/", b""));
        let error = extract(make_zip(&entries), 1024).unwrap_err();
        assert!(
            error.contains("more than 50000 entries"),
            "error was: {error}"
        );

        let metadata: Vec<String> = (0..=MAX_ENTRIES).map(|n| format!("._{n}")).collect();
        let entries: Vec<(&str, &[u8])> = metadata
            .iter()
            .map(|name| (name.as_str(), &b"m"[..]))
            .collect();
        let error = extract(make_zip(&entries), 1 << 20).unwrap_err();
        assert!(
            error.contains("more than 50000 entries"),
            "error was: {error}"
        );
    }

    #[test]
    fn office_files_stored_uncompressed_stay_under_the_entry_cap() {
        // An .xlsx is itself a ZIP of about 25 entries; 1000 of them stored uncompressed.
        let inner = {
            let names: Vec<String> = (0..25).map(|n| format!("xl/part{n}.xml")).collect();
            let entries: Vec<(&str, &[u8])> = names
                .iter()
                .map(|name| (name.as_str(), &b"<x/>"[..]))
                .collect();
            make_zip(&entries)
        };
        let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
        let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        for n in 0..MAX_FILES {
            writer.start_file(format!("book{n}.xlsx"), stored).unwrap();
            writer.write_all(&inner).unwrap();
        }
        let zip = writer.finish().unwrap().into_inner();
        assert!(entry_count_upper_bound(&zip) > 25_000);
        assert_eq!(extract(zip, 1 << 26).unwrap().len(), MAX_FILES);
    }

    #[test]
    fn the_entry_count_bound_counts_every_entry() {
        let zip = make_zip(&[("d/", b""), ("d/a", b"1"), ("._b", b"2")]);
        assert_eq!(entry_count_upper_bound(&zip), 3);
        assert_eq!(entry_count_upper_bound(&make_zip(&[])), 0);
    }

    #[test]
    fn flattened_names_are_made_unique_and_safe() {
        let zip = make_zip(&[
            ("2023/report.pdf", b"1"),
            ("2024/report.pdf", b"2"),
            ("d/..", b"3"),
            ("a\\b.txt", b"4"),
            ("x\0y.txt", b"5"),
            ("e/\t.", b"6"),
        ]);
        let files = extract(zip, 1024).unwrap();
        assert_eq!(
            names(&files),
            [
                "report.pdf",
                "report_1.pdf",
                "file",
                "b.txt",
                "xy.txt",
                "file_1"
            ]
        );
        assert_eq!(files[1].1, b"2");
    }

    #[test]
    fn a_zip_of_only_folders_and_metadata_is_an_error() {
        let zip = make_zip(&[("docs/", b""), ("__MACOSX/._a", b"m"), (".DS_Store", b"m")]);
        let error = extract(zip, 1024).unwrap_err();
        assert!(
            error.contains("holds 3 entries but no files"),
            "error was: {error}"
        );
    }

    #[test]
    fn a_password_protected_file_is_a_clear_error() {
        let zip = make_encrypted_zip(&[("a.txt", b"open")], &[("b.txt", b"locked")]);
        let error = extract(zip, 1024).unwrap_err();
        assert_eq!(
            error,
            "the ZIP is password-protected, which this step does not support"
        );
    }

    #[test]
    fn an_encrypted_metadata_file_is_skipped_without_being_opened() {
        let zip = make_encrypted_zip(&[("a.txt", b"open")], &[("__MACOSX/._a.txt", b"meta")]);
        assert_eq!(names(&extract(zip, 1024).unwrap()), ["a.txt"]);
    }

    #[test]
    fn an_unsupported_compression_method_is_a_clear_error() {
        let mut zip = {
            let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
            let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
            writer.start_file("a.txt", stored).unwrap();
            writer.write_all(b"hello").unwrap();
            writer.finish().unwrap().into_inner()
        };
        // Set the method to 12 (bzip2) in the local header (offset 8) and the central directory
        // entry (offset 10).
        zip[8] = 12;
        let central = zip
            .windows(4)
            .position(|window| window == ENTRY_SIGNATURE)
            .unwrap();
        zip[central + 10] = 12;
        let error = extract(zip, 1024).unwrap_err();
        assert!(
            error.contains("\"a.txt\" in the ZIP is compressed with")
                && error.contains("only Stored and Deflate"),
            "error was: {error}"
        );
    }
}
