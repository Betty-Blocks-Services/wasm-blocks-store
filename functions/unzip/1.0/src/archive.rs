//! Reads the files out of a ZIP archive.

use std::io::{Cursor, Read};

use zip::ZipArchive;
use zip::result::ZipError;

/// The most files one ZIP may hold. Every file becomes its own upload, so this keeps a single run
/// bounded; not a measured platform limit. Folders and metadata files are not counted.
pub const MAX_FILES: usize = 1000;

/// macOS and Windows metadata that archivers add, which is not part of the content: anything in a
/// top-level `__MACOSX` folder, AppleDouble `._` files, `.DS_Store` and `Thumbs.db`. The same kinds
/// as the ActionsJS UNZIP step, but matched on whole names, so `Thumbs.db.xml` is kept.
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

fn describe(error: ZipError) -> String {
    match error {
        ZipError::UnsupportedArchive(message) if message.contains("assword") => {
            String::from("the ZIP is password-protected, which this step does not support")
        }
        ZipError::UnsupportedArchive(message) => {
            format!("the ZIP uses a feature this step does not support: {message}")
        }
        other => format!("the file is not a valid ZIP archive: {other}"),
    }
}

/// The name and content of every file in the archive, in archive order. Folders and metadata
/// files are skipped. Everything is extracted before anything is stored, so a broken archive stores
/// nothing. Stops with an error once the extracted content would pass `max_total_bytes`, so a ZIP
/// bomb cannot exhaust memory.
pub fn extract_files(
    bytes: Vec<u8>,
    max_total_bytes: usize,
) -> Result<Vec<(String, Vec<u8>)>, String> {
    let mut archive = ZipArchive::new(Cursor::new(bytes)).map_err(describe)?;
    let mut remaining = max_total_bytes;
    let mut files = Vec::new();
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).map_err(describe)?;
        let path = entry.name().map_err(describe)?.to_string();
        if entry.is_dir() || is_metadata(&path) {
            continue;
        }
        let name = base_name(&path);
        if name.is_empty() {
            continue;
        }
        if files.len() == MAX_FILES {
            return Err(format!(
                "the ZIP holds more than the {MAX_FILES} files this step extracts in one run"
            ));
        }
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
        files.push((name.to_string(), content));
    }
    Ok(files)
}

#[cfg(test)]
mod tests {
    use std::io::Write;

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
    fn metadata_is_matched_on_whole_names() {
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
}
