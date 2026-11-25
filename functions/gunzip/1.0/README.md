# GUNZIP

Decompresses gzip data, the inverse of [GZIP](../../gzip/1.0/README.md). Use it to read a gzipped
response from an API, or to check what a GZIP step produced.

**Status:** published with `bb functions publish` and run on a canvas of a test app. Also tested locally (unit tests and tests through a real component instance,
including data compressed by the system's `gzip`). The handling of trailing bytes and of empty Data,
and the removal of the Input encoding option (it could never work, see below), came after the
canvas test, so the step needs re-publishing and another canvas run.

## Options

| Option | Notes |
|---|---|
| Data | The gzip data as base64 (it starts with `H4sI`) |
| Output encoding | `utf8` (default): the decompressed data as text, e.g. XML. `base64`: the decompressed bytes as base64, for binary content such as a PDF |
| Result | The decompressed data, encoded as Output encoding says |

## Things to know

- **Data is always base64.** There is no text option for it, because gzip data starts with the
  bytes `1f 8b`, which can never be valid UTF-8.
- **Base64** is read as standard base64 (`+` and `/`); line breaks and missing `=` padding are
  accepted. Base64 output is padded, on one line.
- **Several gzip streams one after another** are all decompressed and joined, as `gzip -d` does.
- **Trailing bytes** after the last gzip stream that do not start a new one (such as the NUL
  padding some systems add to fill a block) are ignored, as `gzip -d` does. Bytes that do start a
  new stream but break off are an error.
- **Size limit:** 32 MiB for the decoded input and for the decompressed result. A small gzip
  stream can expand to gigabytes, so the step stops with an error at that point instead of running
  out of memory. This is the step's own guard, not a measured platform limit.
- **Errors** name what to check: empty Data, data that does not start with the gzip signature, a truncated or
  corrupted stream, invalid base64, or a result that is not valid UTF-8 text (choose `base64` as
  Output encoding for binary content).
