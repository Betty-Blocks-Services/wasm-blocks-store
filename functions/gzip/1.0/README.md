# GZIP

Compresses data with gzip and returns it as text, so it can go into an API payload. A typical use
is a bank or government API that wants a file "compressed with GZIP and encoded as base64". The
counterpart is [GUNZIP](../../gunzip/1.0/README.md).

The steps only exchange text values, so the binary gzip data never leaves the step: you choose
how the input is read (plain text or base64) and how the result is written (base64 or text).

**Status:** published with `bb functions publish` and run on a canvas of a test app. Also tested locally (unit tests and tests through a real component instance,
including decompressing the output with the system's `gzip -d`).

## Options

| Option | Notes |
|---|---|
| Data | The data to compress. Plain text such as XML, or base64 such as a file that another system returned as base64 |
| Input encoding | `utf8` (default): Data is compressed as it is, as UTF-8 bytes. `base64`: Data is base64-decoded first |
| Output encoding | `base64` (default): the gzip bytes as standard base64. `utf8` only works if the gzip bytes happen to be valid UTF-8, which is practically never; it exists for symmetry with GUNZIP |
| Result | The compressed data, encoded as Output encoding says |

Example: a file arrives as base64 and the receiving API wants it gzipped and base64-encoded. Use
one GZIP step with Input encoding `base64` and Output encoding `base64`. Decoding and re-encoding
happen inside the step.

## Things to know

- **Base64 is standard base64** (RFC 4648 section 4, with `+` and `/`), padded with `=`, on one
  line. Base64 input may contain line breaks and may leave out the padding. URL-safe base64
  (`-` and `_`) is refused with an error that says so.
- **The output is deterministic.** The gzip header has modification time 0 and no file name, so
  the same input always gives the same result. Compression level is the default (6). Base64 output
  starts with `H4sI`.
- **Size limit:** 32 MiB after decoding the input. This is the step's own guard, not a measured
  platform limit: how large a text value the platform passes between steps has not been tested.
- **Errors** name the option to check: invalid base64, an unknown encoding, a result that is not
  valid UTF-8, or input over the size limit.
