# UNZIP

Extracts the files from a ZIP file and returns a list of file references. A Wasm port of the
ActionsJS UNZIP step 1.0 (`Betty-Blocks-Services/zip-unzip-action-function`). The counterpart is
[ZIP](../../zip/1.0/README.md).

> **After configuring:** loop over the result (`As`) with a Loop step and save each reference with a
> Create Record or Update Record step on "Model to save the extracted files into", in "Property to
> save the extracted files into". This step does not store the references in records.

**Status:** published with `bb functions publish` and run on a canvas of a test app. Also tested
locally: unit tests, and tests through a real component instance
with a mocked download and a mocked upload, including an archive made by the system's `zip`.

## Options

| Option | Notes |
|---|---|
| ZIP file | The ZIP to extract: a file property's value, or a variable that holds a file or a URL |
| Model to save the extracted files into / Property to save the extracted files into | Where the references will be saved. All files go into this one property |
| As | The file references of the extracted files, in ZIP order (Array of text) |

## Behaviour

- **Folders and metadata are skipped:** folder entries, a top-level `__MACOSX/` folder, `._` files,
  `.DS_Store` and `Thumbs.db`, the same kinds as the ActionsJS step. They are matched on the whole
  file name, so `Thumbs.db.xml` is kept (the ActionsJS step skipped it).
- **The whole ZIP is extracted before anything is stored,** so a broken ZIP, a ZIP bomb or too many
  files stores nothing. If an upload fails partway, the error names the files already uploaded.
  The upload interface cannot remove them, and as the step returns no references they end up in no
  record.
- **Folders inside the ZIP are flattened:** `invoices/a.xml` is stored as `a.xml`.
- **Errors** for: no file in "ZIP file", a failed download, a file that is not a ZIP, a
  password-protected ZIP, or a failed upload. An empty ZIP gives an empty list. Error messages show
  the URL without its query string, which holds the URL's signature.
- **Limits:** each extracted file can be at most the maximum file size of the target file property
  (a setting on the property in the data model); a larger file fails with both sizes in the error.
  Also 64 MiB for the ZIP and for all extracted files together (this stops ZIP bombs), and at most
  1000 files (folders and metadata not counted): the step's own guards. A download whose
  `Content-Length` is over 64 MiB is refused before it is read.
- **Any `http(s)` URL is fetched.** "ZIP file" also takes a plain URL from a text variable, which is
  then downloaded as is, like the native HTTP step does. Don't fill it with a URL an end user typed
  unless you check that URL first.
- **Compression methods:** Stored and Deflate, which is what practically every ZIP tool writes.

### Differences from the ActionsJS step

- The stored file name is the name without its folders. The ActionsJS step passed the full path
  inside the ZIP (`invoices/a.xml`) as the file name.
- Option names are kebab-case (`zip-file`, `model-target`, ...), the convention for Wasm steps in
  this repo.

## Confirmed on the canvas

The first canvas run worked, which confirms that a `Value` option with `allowedKinds: ["FILE"]`
reaches the step as JSON text in a `json-string` parameter, that the platform accepts the
`wasi:http@0.2.12` import `wstd` produces, and that downloading a signed asset URL and storing files
through `upload-file@4.0.0` work.
