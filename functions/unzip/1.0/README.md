# UNZIP

Extracts the files from a ZIP file and returns a list of file references. A Wasm port of the
ActionsJS UNZIP step 1.0 (`Betty-Blocks-Services/zip-unzip-action-function`). The counterpart is
[ZIP](../../zip/1.0/README.md).

> **After configuring:** loop over the result (`As`) with a Loop step and save each reference with a
> Create Record or Update Record step on "Model to save the extracted files into", in "Property to
> save the extracted files into". This step does not store the references in records.

**Status:** published with `bb functions publish` and run on a canvas of a test app. Re-published
and run on the canvas again on 2026-11-25, after the review changes (extract-then-upload order, entry cap, name cleaning, `Content-Length` check). Also tested
locally: unit tests, and tests through a real component instance
with a mocked download and a mocked upload, including an archive made by the system's `zip` (kept as
a fixture in `tests/fixtures/`).

## Options

| Option | Notes |
|---|---|
| ZIP file | The ZIP to extract: a file value, such as a record's file property (confirmed on a canvas). The step also accepts a URL as JSON or plain text, but whether the option lets you pick a text variable is not tested |
| Model to save the extracted files into / Property to save the extracted files into | Where the references will be saved. All files go into this one property |
| As | The file references of the extracted files, in ZIP order (Array of text) |

## Behaviour

- **Folders and metadata are skipped**, without being opened: folder entries, anything in a
  top-level `__MACOSX/` folder, and files named `.DS_Store`, `Thumbs.db` (any case) or starting with
  `._`. Only the file name is checked, not the folders it is in, and `.DS_Store` and `Thumbs.db` must
  be the whole name: `Thumbs.db.xml` is kept.
- **The whole ZIP is extracted before anything is stored,** so a broken ZIP, a ZIP bomb or too many
  files stores nothing. If an upload fails partway, the error names the files already uploaded.
  The upload interface cannot remove them, and as the step returns no references they end up in no
  record.
- **Folders inside the ZIP are flattened:** `invoices/a.xml` is stored as `a.xml`. Names also lose
  NUL and other control characters and are at most 200 bytes (the extension is kept); a name with
  nothing usable left (such as `d/..`) becomes `file`. **Duplicate names** get `_1`, `_2`, ...
  before the extension, so `2023/report.pdf` and `2024/report.pdf` become `report.pdf` and
  `report_1.pdf`, in ZIP order. Names that differ only in case (`Report.pdf`, `report.pdf`) count
  as duplicates.
- **Names without the ZIP's UTF-8 flag** are read as CP437, as the ZIP format specifies. Names that
  a Windows tool wrote in another code page come out with the wrong characters (`é` in Latin-1
  becomes `Θ`).
- **Errors** for: no file in "ZIP file", a failed download, a file that is not a ZIP, a
  password-protected file in the ZIP, a compression method other than Stored or Deflate, a ZIP with
  entries but no files (only folders or metadata), or a failed upload. An empty ZIP gives an empty
  list. Error messages show the URL without its query string (which holds the URL's signature) and
  without a `user:password@` part.
- **Limits:** each extracted file can be at most the maximum file size of the target file property
  (a setting on the property in the data model); a larger file fails with both sizes in the error.
  The step's own guards: 64 MiB for the ZIP and for all extracted files together (this stops ZIP
  bombs), at most 50,000 entries (files, folders and metadata together, checked before the ZIP's
  list of entries is read; a ZIP or Office file stored uncompressed inside the ZIP counts its own
  entries too), and at most 1000 files. A download whose `Content-Length` is over 64 MiB is refused before it is read.
- **Memory:** the ZIP and all extracted files are in memory together, plus the base64 copy of the
  file being uploaded. Measured natively by the PR review: about 176 MiB at the peak for 64 MiB of
  input. The platform's memory limit per run is not known.
- **Any `http(s)` URL is fetched.** "ZIP file" also takes a plain URL, which is then downloaded as
  is, like the native HTTP step does. Don't fill it with a URL an end user typed unless you check
  that URL first.
- **Compression methods:** Stored and Deflate, which is what practically every ZIP tool writes.

### Differences from the ActionsJS step

Compared with UNZIP 1.0 in `Betty-Blocks-Services/zip-unzip-action-function`:

- **File names:** the ActionsJS step stored each file under its full path inside the ZIP
  (`invoices/a.xml`), with no de-duplication or cleaning. This step stores the name without its
  folders, cleaned and made unique (see above).
- **Metadata:** the ActionsJS step checked the whole path: `._` at the start of any folder or file
  name, and `.DS_Store` or `Thumbs.db` anywhere in it (case-sensitive). So it also skipped
  `._dir/b.txt` and `Thumbs.db.xml` and kept `thumbs.db`; this step keeps the first two and skips
  the last.
- **Names without the UTF-8 flag:** the ActionsJS step showed `?` for characters it could not
  decode; this step decodes them as CP437 (see above).
- **Content type:** the ActionsJS step passed a content type per file, from its extension.
  `upload-file@4.0.0` takes no content type, and what the platform sets instead is not known.
- **Output:** the ActionsJS step's `As` was an `Array` with `dataType: SCHEMA`; this step's is an
  `Array` of `STRING`.
- **Input:** besides a file value, this step accepts a URL as JSON or plain text.
- **Errors and limits:** a ZIP of only folders or metadata, too many entries or files, an
  unsupported compression method and a ZIP over 64 MiB fail with their own message. The ActionsJS
  step had no limits of its own and returned an empty list for a ZIP without files.
- Option names are kebab-case (`zip-file`, `model-target`, ...), the convention for Wasm steps in
  this repo.

## Confirmed on the canvas

The first canvas run worked, which confirms that a `Value` option with `allowedKinds: ["FILE"]`
reaches the step as JSON text in a `json-string` parameter, that the platform accepts the
`wasi:http@0.2.12` import `wstd` produces, and that downloading a signed asset URL and storing files
through `upload-file@4.0.0` work.
