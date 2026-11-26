# Import from CSV or Excel

Downloads a CSV or Excel file, converts its rows into records of a model and writes them through
the Data API: new records with `createMany<Model>`, existing records (with deduplication on) with
`upsertMany<Model>`. A Rust port of the ActionsJS step `importCSV` 3.3
(Betty-Blocks-Services/import-csv).

**Status:** built for `wasm32-wasip2`, unit-tested and component-tested (`cargo test -p
import-records`). **Not run on a canvas yet.** Until it is, it does not meet the bar in
[CONTRIBUTING.md](../../../CONTRIBUTING.md). See "Not tested yet" for what the canvas run has to
show.

## Options

| Option | Notes |
|---|---|
| URL to the import file | An `https://` (or `http://`) URL. Spaces and other characters a URL may not contain (as in a file property URL with the file name in it) are percent-encoded. Up to 5 redirects are followed |
| Import file type | CSV or Excel (values `csv`, `excel`, WIT `enum file-type`). Excel reads the first sheet of an xlsx, xlsm, xls or ods file |
| Delimiter | Detect, `;`, `,`, `\|` or tab (values `auto`, `semicolon`, `comma`, `pipe`, `tab`, WIT `enum delimiter`). Detect counts each one in the header row and picks the most common one (`,` on a tie or when there is none). CSV only |
| Import model | The model the records go into |
| Import mapping | KEY = column name in the file, VALUE = database name (`first_name`) or `model.property` for a belongs-to relation (`company.name`). A KEY ending in `*` marks a required column |
| Format mapping for import columns | KEY = column name, VALUE = `checkbox`, `decimal` (or `price`), `number`, `text`, `Date, <format>`, `Time, <format>` or `Datetime, <format>`. `Date` alone means `Date, dd-MM-yyyy`. `Date\|dd-MM-yyyy` (the export step's notation) works too |
| Default mapping | KEY = database name of a property or relation, VALUE = the value set on every record (both new and updated) |
| Deduplicate records | Off by default. When on, rows whose unique column matches an existing record update that record |
| Unique record identifier | The column (as in the Import mapping) that identifies a record. It must be mapped to a property, not a relation |
| Unique record type | Text, Number, Decimal or Id (WIT `enum unique-type`): how the values are compared and sent in the lookup filter |
| Update mapping for existing records | Like the Import mapping, used instead of it for updated records. Empty: updated records get the Import mapping |
| Validate required columns | Off by default. When on, a missing `*` column fails the step before anything is written |
| Result | `Records created: 10, records updated: 2`, followed by the number of skipped rows and of unmatched relation values when there are any |

## Behaviour

- **Every value is converted before anything is written.** A value that does not fit its format
  fails the step with the row number and column name, e.g. `Row 14, column "Age": "twelve" is not a
  whole number. Nothing was imported`. Relation and duplicate lookups also run before the first
  write.
- **The writes are not one transaction.** Records are sent in chunks of 500 per request, creates
  first, then updates. When a chunk fails, the error names its rows and how many records were already
  written; those stay.
- **Values:**
  - Without a format, a CSV value is text and an Excel cell keeps its type (number, yes/no, text).
    An Excel date without a format becomes `yyyy-MM-dd`, or `yyyy-MM-dd HH:mm:ss` when it has a time.
  - `text`: the value as text (an Excel number `12` becomes `"12"`).
  - `number`: a whole number. `12.0` is accepted; `12.5` is an error.
  - `decimal`/`price`: sent as text with a `.`, without rounding. `12,5` becomes `12.5`, and `12`
    becomes `12.00` (as in the ActionsJS step). With both `.` and `,` in a value, the last one is the
    decimal separator (`1.234,56` and `1,234.56` are both `1234.56`).
  - `checkbox`: `true`/`false`, `1`/`0`, `yes`/`no`, `ja`/`nee` (any case).
  - `Date`/`Time`/`Datetime`: read with the format, sent as `yyyy-MM-dd`, `HH:mm:ss` or
    `yyyy-MM-dd HH:mm:ss` (the strings the ActionsJS step sent). Supported letters: `y` (`yy` is
    1969–2068), `M` (`MMM` `Jan`, `MMMM` `January`, English), `d`, `H`, `h` with `a` (AM/PM), `m`, `s`
    and `S`; text in single quotes is literal. Numbers may have fewer digits than the pattern
    (`dd-MM-yyyy` reads `1-2-2024`). An ISO date (`2024-01-31`, with or without a time) is accepted
    in any date column. An Excel date cell is used as it is, whatever the pattern.
  - An empty cell is `null`.
- **Missing columns:** a mapped column that is not in the file is left out of the records (not
  set to `null`). If none of the Import mapping's columns is in the file, the step fails and lists
  the file's columns, which usually means the wrong delimiter or a typo.
- **Relations** (`company.name`): the related model is assumed to have the name of the relation
  (`Company`), as in the ActionsJS step. The step looks up the records whose `name` is one of the
  file's values (200 values per request) and sets the relation to the id of the first match. A value
  without a match sets the relation to `{}`, as the ActionsJS step did, and is counted in the Result.
  Give the column a `number` or `decimal` format when the related property is not text.
- **Deduplication:** the existing records are looked up by the unique column's values (200 per
  request, as GraphQL variables, `in` for text, numbers and ids, `_or` of `eq` for decimals). Rows
  with an empty unique value are skipped and counted in the Result. Two rows with the same new
  value both create a record; two rows matching the same existing record both update it.
- **CSV:** UTF-8 (a byte order mark is removed), UTF-16 with a byte order mark, and Windows-1252
  for files that are not valid UTF-8 (what Excel's "CSV (comma delimited)" writes on Windows).
  Quoted values may hold delimiters, quotes (`""`) and line breaks. Header names are trimmed. Rows
  with only empty values are skipped. Row numbers in errors are the line numbers in the file.
- **Requests:** every lookup page and every write chunk is one Data API request, towards the Data
  API's rate limit.

### Differences from the ActionsJS step

- **No batched processing** (the six batch options). The ActionsJS step needed it for the 1-minute
  action limit; a Wasm action may run for 10 minutes. It can come back in a later version if large
  files need it.
- **No Logging option.** It would need the `wasi:logging` import, which has not been tried in a
  custom step.
- **Wrong values fail the step** instead of being imported as `null` or empty text (dates, numbers,
  decimals) or as `true` (checkbox: the ActionsJS step read any unknown text, `no` included, as
  `true`).
- **Mistakes in the options fail the step** before the download: a Format mapping key that is not a
  mapped column, an unknown format kind (the ActionsJS step read it as a date), `Time`/`Datetime`
  without a format, an invalid property name, Deduplicate on without a unique column.
- **The duplicate lookup is filtered.** The ActionsJS step's `getAll` dropped the `where` filter, so
  it read every record of the model (and failed above 20,000).
- **A missing column is left out** instead of setting the property to `null`, so an update with
  a column missing from the file does not clear that property.
- **The Update mapping supports relations and formats.** The ActionsJS step formatted only columns of
  the Import mapping and wrote a relation in the Update mapping as a plain property.
- **Decimals keep their digits.** The ActionsJS step rounded to 2 decimals and replaced only the first
  `,`.
- **The Result** says how many records were created and updated (the ActionsJS step returned
  `Finished processing N import lines.`).
- **More date letters** are read leniently (`d-M-yyyy` and `dd-MM-yyyy` read the same text), but only
  the letters listed above; date-fns knew more (`do`, `L`, `EEEE`, ...).
- **Empty CSV values are `null`**, not empty text.
- Option names are kebab-case (`file-url`, not `fileUrl`): an action that used the ActionsJS step has
  to be configured again.

## Not tested yet

To check in the first canvas run:

- **Writing through `request`.** The step writes with the host `data-api` `request` function and
  the `helper-context`, which this repo's steps have used for reads only. The mutations
  (`createMany<Model>(input: $input)`, `upsertMany<Model>(input: $input)`) have the shape that
  `wasm-base-components`' data-api sends and that the ActionsJS step used. A direct `update<Model>`
  mutation was refused in an earlier test (developer learnings log, 2026-10-23), with an inline
  input and no variables. If the Data API refuses these too, the fallback is the native steps' route:
  a `borrow<data-api>` parameter instead of `helper-context`.
- **The lookup query** declares `$where: <Model>FilterInput`, as the platform's crud component does.
- **The relation input**: a plain id, and `{}` for no match, as in the ActionsJS step.
- **Downloading** over `wasi:http` from a step that also imports `betty-blocks-types` (tested in
  `exchange-code-for-tokens` on its own), and from a file property URL.
- How a **Map** arrives (expected: a JSON object string with the keys in entered order; a list of
  `{ key, value }` is accepted too), and the Select values `auto` and `id`.
- **Size and time:** the file, its rows and the records are held in memory. No limit has been
  measured; try a large file and note the result here.

## Tests

`cargo test -p import-records` runs the unit tests and `tests/mod.rs`, which runs the built
component (`just build` first; needs Rust 1.94+ for wasmtime) with a stubbed `wasi:http` download
and a mocked Data API. `parse_map`, `snake_to_camel` and `is_name` in `src/mapping.rs` also exist in the export-records
step (PR #21): once both are merged, keep the copies the same.

## Building

From the repository root, `just build` builds every step for `wasm32-wasip2` and copies
`import_records.wasm` into this folder. `just step-lint` checks `function.json` against the WIT.
