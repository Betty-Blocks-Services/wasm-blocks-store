# GUNZIP

Decompresses gzip data, the inverse of [GZIP](../../gzip/1.0/README.md). Use it to read a gzipped
response from an API, or to check what a GZIP step produced.

**Status:** published with `bb functions publish` and run on a canvas of a test app. Also tested locally (unit tests and tests through a real component instance,
including data compressed by the system's `gzip`). The handling of trailing bytes and of empty Data
was added after the canvas test and is tested locally only.

## Options

| Option | Notes |
|---|---|
| Data | The gzip data, normally as base64 (it starts with `H4sI`) |
| Input encoding | `base64` (default): Data is base64-decoded first. `utf8`: Data's UTF-8 bytes are taken as the gzip data, which only works in the rare case that they are valid UTF-8 |
| Output encoding | `utf8` (default): the decompressed data as text, e.g. XML. `base64`: the decompressed bytes as base64, for binary content such as a PDF |
| Result | The decompressed data, encoded as Output encoding says |

## Things to know

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
