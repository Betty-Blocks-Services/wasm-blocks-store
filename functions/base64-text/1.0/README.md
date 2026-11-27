# Base64

Encodes text as base64, or decodes base64 back to text. It replaces the classic expression
functions `base64_encode` and `base64_decode` in one step: the Mode option picks the direction.

Typical uses: build a `Basic` authorization header (`Basic ` followed by `user:password` encoded),
fill an API field that wants a value as base64, or read the payload of a JWT: decode only the
middle part of the token (between the two `.`), with Alphabet `url-safe`. A whole token gives an
error that says so.

**Status:** published with `bb functions publish` and run on a canvas. Also tested locally (unit
tests with the RFC 4648 test vectors, and tests through a real component instance).

## Options

| Option | Notes |
|---|---|
| Mode | `encode` (default): text to base64. `decode`: base64 to text |
| Text | The text to encode (as UTF-8), or the base64 to decode. An empty variable gives an empty result |
| Alphabet | `standard` (default, `+` and `/`) or `url-safe` (`-` and `_`) |
| Result | The base64, or the decoded text |

## Things to know

- **Encoding:** standard base64 is padded with `=`, on one line, like the classic
  `base64_encode`. URL-safe base64 is written without padding, as JWTs and URL parameters use it.
- **Decoding** accepts input with or without `=` padding, ignores line breaks and other
  whitespace (MIME-style base64 wraps at 76 characters), and ignores leftover bits in the last
  character (`Zh==` decodes to `f`). If the input is base64 of the other alphabet, the error says
  to switch Alphabet; for other text it says to check Text or Mode.
- **Decoding only works for text.** Steps pass text values to each other, so the decoded bytes
  must be valid UTF-8. Base64 of a binary file (a PDF, an image, a gzip file) fails with an error
  that says so: keep such data as base64, or pass it to a step that takes base64 directly (for
  example a step that stores it as a file).
- **Differences from the classic expressions**
  ([reference](https://docs.bettyblocks.com/classic-docs/classic-expression-reference)): for
  text, Encode gives the same output as `base64_encode` (`"test"` gives `dGVzdA==`) and Decode the
  same as `base64_decode(value, "string")`. Files are not supported: classic `base64_encode` also
  takes a file, and `base64_decode(value, "file", name)` returns a temporary file. This step
  only takes and returns text.
- **Errors** name the option to check: invalid base64, a decoded result that is not text, or an
  unknown Mode or Alphabet value.
