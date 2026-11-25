# GZIP

Compresses data with gzip and returns it as text, so it can go into an API payload. A typical use
is a bank or government API that wants a file "compressed with GZIP and encoded as base64". The
counterpart is [GUNZIP](../../gunzip/1.0/README.md).

The steps only exchange text values, so the binary gzip data never leaves the step: you choose
how the input is read (plain text or base64), and the result is always base64.

**Status:** published with `bb functions publish` and run on a canvas of a test app. Also tested locally (unit tests and tests through a real component instance,
including decompressing the output with the system's `gzip -d`). The Output encoding option was
removed after the canvas test (it could never work, see below), so the step needs re-publishing
and another canvas run.

## Options

| Option | Notes |
|---|---|
| Data | The data to compress. Plain text such as XML, or base64 such as a file that another system returned as base64 |
| Input encoding | `utf8` (default): Data is compressed as it is, as UTF-8 bytes. `base64`: Data is base64-decoded first |
| Result | The compressed data as standard base64 |

Example: a file arrives as base64 and the receiving API wants it gzipped and base64-encoded. Use
one GZIP step with Input encoding `base64`. Decoding and re-encoding happen inside the step.

## Things to know

- **The result is always base64.** There is no text option for it, because gzip data starts with
  the bytes `1f 8b`, which can never be valid UTF-8.
- **Base64 is standard base64** (RFC 4648 section 4, with `+` and `/`), padded with `=`, on one
  line. Base64 input may contain line breaks and may leave out the padding. URL-safe base64
  (`-` and `_`) is refused with an error that says so.
- **The output is deterministic.** The gzip header has modification time 0 and no file name, so
  the same input always gives the same result. Compression level is the default (6). Base64 output
  starts with `H4sI`.
- **Size limit:** 32 MiB after decoding the input. This is the step's own guard, not a measured
  platform limit: how large a text value the platform passes between steps has not been tested.
- **Errors** name the option to check: invalid base64, an unknown encoding, or input over the size
  limit.
