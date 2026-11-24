# GUNZIP

Decompresses gzip data, the inverse of [GZIP](../../gzip/1.0/README.md). Use it to read a gzipped
response from an API, or to check what a GZIP step produced.

**Status:** built and tested locally only (unit tests and tests through a real component instance,
including data compressed by the system's `gzip`). Not yet tested on a canvas.

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
- **Size limit:** 32 MiB for the decoded input and for the decompressed result. A small gzip
  stream can expand to gigabytes, so the step stops with an error at that point instead of running
  out of memory. This is the step's own guard, not a measured platform limit.
- **Errors** name what to check: data that does not start with the gzip signature, a truncated or
  corrupted stream, invalid base64, or a result that is not valid UTF-8 text (choose `base64` as
  Output encoding for binary content).
