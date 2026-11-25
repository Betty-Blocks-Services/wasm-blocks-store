# ZIP

Puts the files of a collection into one ZIP file and returns a file reference. A Wasm port of the
ActionsJS ZIP step 1.1 (`Betty-Blocks-Services/zip-unzip-action-function`). The counterpart is
[UNZIP](../../unzip/1.0/README.md).

> **After configuring:** add a Create Record or Update Record step on "Model to save file into" and
> put the result (`As`) in "Property to save file into". This step does not store the reference in
> a record.

**Status:** published with `bb functions publish` and run on a canvas of a test app. Also tested
locally: unit tests, and tests through a real component instance
with mocked downloads and a mocked upload (the archive also opens with the system's `unzip`).

## Options

| Option | Notes |
|---|---|
| Collection to read files from | A collection variable with the records whose files go into the ZIP. A `Value` option limited to collections, not a `Collection` option (see below) |
| Property to read files from | A file or image property of the collection's model |
| Model to save file into / Property to save file into | Where the reference will be saved. The upload needs them, so they must match the Create or Update step |
| Name for the ZIP file | Default `zip`. `.zip` is added unless the name already ends in it |
| As | The file reference of the ZIP (Text) |

## Behaviour

- **Records without a file** are skipped: a value that is `null`, `{}`, or has neither a `url` nor
  a `name`. The step fails, and uploads nothing, when:
  - the collection has records but **none of them has a file** in the property (`0 of N records
    had a file in "<property>"`), which usually means the wrong property is selected;
  - a value is **not a file**: text, a number, or an object without a `url`;
  - the records have no such property at all.
- **An empty collection** (no records) gives an empty ZIP and its reference: there is nothing wrong
  with the configuration, so it is not an error.
- **File names** inside the ZIP: the record's file name with its folder parts, NUL and other control
  characters removed, at most 200 bytes (the extension is kept). Without a usable name, the last
  part of the URL is used (percent-decoded), or `file`. **Duplicate names** get `_1`, `_2`, ...
  before the extension (`a.pdf`, `a_1.pdf`; a name like `.env` has no extension, so `.env_1`).
  Names that differ only in case count as duplicates, because Windows and macOS would extract
  `Report.pdf` and `report.pdf` to the same file.
- **Name for the ZIP file** is cleaned the same way, so `../../x` is stored as `x.zip`.
- **Files are downloaded one at a time** from their URLs over `wasi:http`, the way the native
  send-mail step reads collection attachments. A download that fails (for example an expired link)
  fails the step with the HTTP status. Error messages show a file's URL without its query string
  (which holds the URL's signature) and without a `user:password@` part.
- **Any `http(s)` URL in a record is fetched** and its body goes into the ZIP. File properties hold
  links to the app's own file storage, but if records can hold a URL an end user chose, that user
  can make the step fetch any address the step can reach and get the response back in the ZIP.
- **Compression:** Deflate for every file, permissions `0644`, and the date 1980-01-01 (the `zip`
  crate's default; both are checked by a test).
- **Size limits:** the ZIP can be at most the maximum file size of "Property to save file into"
  (a setting on the property in the data model). Downloads are capped at 64 MiB per run, summed
  over all files, and at 1000 files: the step's own guards. A download whose `Content-Length` is
  over the cap is refused before it is read; otherwise reading stops as soon as the cap is passed.
- **Memory:** the current download and the archive so far, then the archive and its base64 copy
  for the upload. Measured natively by the PR review: about 149 MiB at the peak for 64 MiB of
  input. The platform's memory limit per run is not known.

### Differences from the ActionsJS step

Compared with ZIP 1.1 in `Betty-Blocks-Services/zip-unzip-action-function`:

- **Compression:** the ActionsJS step stored every file uncompressed (JSZip's default); this step
  uses Deflate.
- **Content type:** the ActionsJS step stored the ZIP with content type `application/zip`.
  `upload-file@4.0.0` takes no content type, and what the platform sets instead is not known.
- **Records without a file:** the ActionsJS step skipped only `{}`; `null` made it fail with a
  TypeError and an object without a `url` made it fetch `undefined`. This step skips `null` too, and
  fails with a clear message on a value without a `url` and when no record has a file.
- **Failed downloads:** the ActionsJS step did not check the HTTP status, so an error page could end
  up in the ZIP. This step fails on any status other than 2xx.
- **File names:** the ActionsJS step used the name as it was, folders included (`x/a.pdf` became a
  folder in the ZIP), and wrote `undefined` for a missing name. This step removes folders and
  control characters, trims and shortens names, and falls back to the URL or `file`.
- **Duplicate names of dotfiles:** the ActionsJS step made `.env` into `_1.env`; this step makes it
  `.env_1`. Other names get `_1`, `_2` the same way in both.
- **The collection option** is a `Value` option limited to collections, with
  `allowLazyCollection: false`, not a `Collection` option (see below). A lazy collection may not be
  selectable.
- **Limits:** 64 MiB of downloads and 1000 files per run; the ActionsJS step had none of its own.
- Files are downloaded one after another, not all at once, to keep memory use low.
- Option names are kebab-case (`property-source`, `model-target`, ...), the convention for Wasm
  steps in this repo.

## Why the collection is a Value option

A `Collection` option did not work in three canvas attempts: it passes its records to a Wasm step
as objects, and no generic WIT type can take an object whose fields depend on the model (see the
learnings log, 2026-11-24; the cause is inferred from the error messages). A `Value` option arrives as JSON text, as UNZIP's file input showed on a canvas, so this
step takes the collection as `Value` with `allowedKinds: ["COLLECTION"]` and parses the records
itself. It accepts `{ "data": [...] }` or a bare list; any other shape fails with an error that
shows the start of what arrived.

## Confirmed on the canvas

ZIP works on a canvas with the collection as a `Value` option: the file property picker offers the
collection's properties, and the step reads the records, downloads and zips the files, and stores
the archive.

The first full run failed to store a 13.9 MB archive because the target file property allowed at
most 10 MB. **The maximum file size is a setting of each file property in the data model**; after
raising it to 25 MB the same run worked. The step now explains such a refusal with both sizes.
