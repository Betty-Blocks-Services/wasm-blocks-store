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

- **Records without a file** (no value, or no URL) are skipped. A collection without files gives an
  empty ZIP, as in the ActionsJS step.
- **Duplicate file names** get `_1`, `_2`, ... before the extension (`a.pdf`, `a_1.pdf`), as in the
  ActionsJS step. Folder parts in a name are removed.
- **Files are downloaded one at a time** from their URLs over `wasi:http`, the way the native
  send-mail step reads collection attachments. Only the current file and the archive so far are in
  memory. A download that fails (for example an expired link) fails the step with the HTTP status.
  Error messages show a file's URL without its query string, which holds the URL's signature.
- **Compression:** Deflate for every file. Entries have the fixed date 1980-01-01 and permissions
  `0644`.
- **Size limits:** the ZIP can be at most the maximum file size of "Property to save file into"
  (a setting on the property in the data model). Downloads are capped at 64 MiB per run and at 1000
  files, the step's own guards. A download whose `Content-Length` is over the cap is refused before
  it is read; otherwise reading stops as soon as the cap is passed.

### Differences from the ActionsJS step

- Option names are kebab-case (`property-source`, `model-target`, ...), the convention for Wasm
  steps in this repo.
- Files are downloaded one after another, not all at once, to keep memory use low.

## Why the collection is a Value option

A `Collection` option passes its records to a Wasm step as objects, and no generic WIT type can take
an object whose fields depend on the model (three canvas attempts, see the learnings log,
2026-11-24). A `Value` option arrives as JSON text, as UNZIP's file input showed on a canvas, so this
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
