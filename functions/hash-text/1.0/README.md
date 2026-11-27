# Hash Text

Hashes text and returns the hash as text. It replaces the hash functions of the
[classic expressions](https://docs.bettyblocks.com/classic-docs/classic-expression-reference) in
one step: the Algorithm option picks the function.

| Classic expression | Algorithm |
|---|---|
| `sha256(text)` | `sha256` |
| `sha384(text)` | `sha384` |
| `sha512(text)` | `sha512` |
| `sha1(text)` | `sha1` |
| `md5(text)` | `md5` |
| `hmac_sha256(text, secret)` | `hmac-sha256`, with the secret in Secret key |
| `hmac_sha384(text, secret)` | `hmac-sha384`, with the secret in Secret key |
| `hmac_sha512(text, secret)` | `hmac-sha512`, with the secret in Secret key |

With Output encoding `hex` (the default) the step gives the same output as the classic
expression: the unit tests check every example of the classic expression reference
(`"Betty Blocks"`, secret `"secret"`).

Typical uses: sign a request for an API that asks for an HMAC signature, check the signature of an
incoming webhook (hash the body with the shared secret and compare it to the signature header), or
make a fingerprint of a value to compare later.

**Status:** published with `bb functions publish` and run on a canvas. Also tested locally (unit
tests with the classic expression reference examples, the FIPS 180 and RFC 4231 test vectors, and
tests through a real component instance).

## Options

| Option | Notes |
|---|---|
| Algorithm | `sha256` (default), `sha384`, `sha512`, `sha1`, `md5`, `hmac-sha256`, `hmac-sha384` or `hmac-sha512` |
| Text | The text to hash, as UTF-8 bytes. An empty variable hashes the empty string |
| Secret key | Required for the HMAC algorithms, and must be empty for the others. Used as UTF-8 text |
| Output encoding | `hex` (default, lowercase, as the classic expressions) or `base64` (standard, padded) |
| Hash | The result |

## Things to know

- **The text is hashed exactly as it is.** A trailing space or line break, or `\r\n` instead of
  `\n`, gives a different hash. When checking a webhook signature, hash the raw body the sender
  signed, not a re-formatted copy.
- **Hex is lowercase.** Compare hashes from systems that send uppercase hex case-insensitively.
- **The secret key is used as UTF-8 text**, as in the classic `hmac_*` expressions. Some systems
  hand out a key that is itself hex or base64 and expect the decoded bytes to be the key; this
  step does not decode it. Such a key gives a different HMAC here.
- **SHA-1 and MD5 are not secure** against deliberate collisions. Only use them where another
  system requires them, or for a fingerprint that does not need to be secure.
- **Errors** name the option to check: an HMAC without a secret key, a secret key with a plain
  hash (likely the wrong algorithm was picked), or an unknown Algorithm or Output encoding value.
  The Algorithm value also accepts the classic names, such as `hmac_sha256`.
- No size limit of its own: the text is already in memory when the step runs.
