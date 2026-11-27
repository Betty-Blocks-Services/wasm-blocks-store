# Export to CSV or Excel

Exports the records of a model to a CSV or Excel (XLSX) file, stores the file through the host
`betty-blocks-types:upload-file@4.0.0` interface and returns the file reference. A Rust port of the
ActionsJS step `export-to-csv` 3.4.

**Status:** built for `wasm32-wasip2`, unit- and component-tested (`cargo test -p export-records`)
and run successfully on a canvas (2026-11-24). Later runs exported 50,000 records in 4.4 seconds and 200,000 in
24 seconds (see "Performance"). The **Escape formulas** option, the paging changes and the review fixes of PR #21 have
not all been run on a canvas yet. The points
under "Not tested yet" are still open.

> **After configuring:** add a Create Record or Update Record step on the model chosen as
> "Model to save file into" and put the result (`As`) in the file property chosen as "Property to
> save file into". This step only returns a file reference; it does not store it in a record.

## Options

| Option | Notes |
|---|---|
| Type | CSV or Excel (Select values `csv` and `excel`, the cases of the WIT `enum file-type`) |
| Model to export records from | The model to read |
| Filter | Data API filter without outer braces, e.g. `type: { eq: "{{type}}" }`. Empty exports every record |
| Variables in filter | Values for `{{name}}` in the filter |
| Sort property | Leave empty to sort by `id` |
| Sort order | Ascending or descending (values `ASC`/`DESC`, WIT `enum sort-order`) |
| Delimiter | `;` `,` `\|` tab or space (values `semicolon`, `comma`, `pipe`, `tab`, `space`, WIT `enum delimiter`); default `,`. CSV only |
| Model to save file into / Property to save file into | Where the file reference is meant to go (the upload needs them) |
| Filename | Without extension: `.csv` or `.xlsx` is added (a name that already ends in it does not get it twice). Empty means `export`. `/`, `\` and control characters become `_` |
| Property mapping | KEY = column header, VALUE = database name (`created_at`) or `relation.property` for a relation. For a has-many or has-and-belongs-to-many relation (`departments.description`) the values of all related records are written in one cell, one per line. Columns follow the order of the map |
| Format mapping for export columns | KEY = column header, VALUE = `Date\|<format>` (also `Datetime\|` and `Time\|`), or `Decimal\|<separator>` / `Price\|<separator>` |
| Use BOM | CSV only; default on |
| Escape formulas | CSV only; default on. Puts a `'` in front of text that starts with `=`, `+`, `-`, `@`, a tab or a carriage return, so a spreadsheet does not run it as a formula (CSV injection). The `'` stays visible in the cell and is part of the value, also for a program that reads the CSV: turn the option off for a file that is not opened in a spreadsheet. Column headers are escaped the same way. A sign followed by only digits, spaces, `.` and `,` (and maybe a closing `%`) is left alone: `-12,5`, `+31 6 1234 5678`, `-5%`. Other text that starts with a sign does get the `'`, e.g. `- none -` |
| As | The file reference (Text) |

## Behaviour

- **All matching records are exported**, asked for 5000 per request until `totalCount` is reached.
  A page with fewer records does not end the export (the Data API may cap pages lower); only reaching
  `totalCount` does.
  A `totalCount` that is not a whole number fails the step. If a page comes back empty before that, the step fails rather than storing an
  incomplete file. Every page is one request towards the Data API rate limit.
- **The records are always sorted**, by `id` when no sort property is chosen, so paging cannot export a
  record twice or skip one. With a sort property that is not unique the order of equal values can differ
  between requests. The step always selects `id` and fails with a clear message when a record comes back
  on two pages, instead of writing a file with duplicates and gaps (the ActionsJS step wrote it). The
  step sends one sort field; whether the Data API accepts a second one as a tie-breaker has not been
  checked. The option's info asks for a property with unique values.
- **CSV** is written like the `export-to-csv` 0.2 package the ActionsJS step used: `\r\n` after every
  line (the last one too), text in double quotes with `"` doubled, `TRUE`/`FALSE` for yes/no values,
  numbers without quotes, and an empty value as `""`.
- **XLSX** has one sheet, named after the model to save the file into (as in the ActionsJS step;
  invalid characters are removed and the name is cut to 31 characters). Numbers are number cells,
  yes/no values boolean cells, text is text, and empty values leave the cell empty. Integers beyond
  2^53, which an Excel number would round, are written as text with all their digits. A sheet holds at
  most 1,048,575 records: when `totalCount` is higher the step fails after the first page, before
  fetching the rest. Text over 32,767 characters (Excel's cell limit) fails the step and names the
  column and record.
- **Filter variables** are filled in like the `data-api-query` step (same template code and the same
  value check): inside quotes any text is safe; outside quotes only a number, `true`/`false`, a name or
  a comma-separated list of those; `{{{name}}}` is raw and unchecked. The filter must have balanced
  brackets, so it cannot close `where: { }` and add fields or models to the export.
- **Dates** use date-fns format letters: `y`, `M` (up to `MMMM`), `d`, `E`, `Q`, `H`, `h`, `m`, `s`,
  `S` and `a`; text in single quotes is printed as it is (`'at'`), `''` is a quote. Other letters fail the
  step with a message (e.g. `YYYY` and `DD`, as date-fns also refuses them by default). A value that is
  not a date, time or datetime fails the step and names the column.

### Differences from the ActionsJS step

- **No records** gives a file with only the header row. The ActionsJS step failed with "The generated
  query did not give any results."
- **Datetimes are formatted in UTC.** The ActionsJS step used the server's time zone.
- **`Datetime|<format>` and `Time|<format>`** are formatted like `Date|`. The ActionsJS step only
  handled `Date|` and exported the value unformatted for the other two.
- **Only the date-fns letters `y M d E Q H h m s S a` are supported.** The ActionsJS step passed the
  pattern to date-fns 2, which also knows `do`, `w`, `L`, `e`, `k`, `K`, `G` and more: a pattern such as
  `MMMM do, yyyy` worked there and fails the step here (with a message listing the supported letters).
  `aaa` gives `am`/`pm` and `a`/`aa` give `AM`/`PM`, as in date-fns.
- **`Date|dd|MM`** uses `dd|MM` as the pattern (a `|` after the first one is part of it). The ActionsJS
  step used only the part between the first and the second `|`.
- **A mistake in the Format mapping fails the step** before any record is fetched: a key that is not a
  column of the Property mapping, a value that is not text, a value without `|`, an unknown kind
  (`Dates|`) or an empty format. The ActionsJS step ignored all of these and exported the value
  unformatted. None of these cases is kept lenient. The format is trimmed, so `Date| dd-MM-yyyy` adds no
  space.
- **Filename:** spaces around the name are removed and an empty name becomes `export`. The ActionsJS
  step used the name as given (an empty option gave the file name `undefined`).
- **Empty values in XLSX are blank cells.** The ActionsJS step wrote them as empty text (`''` through
  SheetJS), which `ISBLANK` and `COUNTA` treat differently.
- **CSV text that could run as a formula** gets a `'` in front by default (option Escape formulas).
  The ActionsJS step wrote it as it was.
- **`Decimal|` and `Price|` only change numbers** and text that is a number. The ActionsJS step
  replaced the first `.` in any value, so `v1.2-beta` became `v1,2-beta`.
- **A blank row in the Format mapping** (no key and no value) is skipped.
- **Incomplete exports fail.** A Data API response without results, or a page that comes back empty
  before `totalCount` is reached, fails the step. The ActionsJS step could write a short file.
- **A has-many or has-and-belongs-to-many relation** (`departments.description`) is written as one value per line in a single cell (the ActionsJS step wrote `[object Object]`). Empty values are left out and a record without related records gets an empty cell. Values are not quoted or escaped inside the cell, so a value that holds a line break itself cannot be told from two values.
- **An object** that is not a relation property is written as JSON.
- **Excel column widths** fit the longest line of each column (header included), between 8 and 60 characters, up to 90 for a column with multi-line text. Cells with a line break wrap, so every line shows. The ActionsJS step left every column at the default width.
- **Excel dates are real dates.** A column with a `Date|`, `Datetime|` or `Time|` format is written as an Excel date with a matching number format, so it sorts and filters as a date and shows as the pattern says. A value stays text when the pattern has no exact Excel equivalent (the quarter `Q`, `h` without `a`, `H` with `a`, a minute or month Excel would read the other way, more than 3 `S`), when the value is no date, or when the date is before 1900-03-01. A to-many relation column is always text. CSV is always text. The ActionsJS step wrote text.
- **The Excel header row** is bold, frozen (it stays visible while scrolling) and has filter buttons. The ActionsJS step wrote it as plain text in the first row.
- **A header that holds the delimiter, a quote or a line break** is quoted, so it cannot break the file.
- **A property path with other characters than letters, digits and `_`** fails the step instead of
  being turned into a different field name.
- Option names are kebab-case (`model-source`, not `modelSource`): an action that used the ActionsJS step
  has to be configured again.

## Not tested yet

Run on a canvas, but not checked separately:

- every case of the Select options: the platform turned the chosen Select values into WIT enum cases
  in the canvas run, but not every value of `file-type`, `sort-order` and `delimiter` has been tried;
- how the platform passes an empty optional **Sort property** (expected `null` or an empty list; both are
  handled);
- whether a **Map** reaches the step as a JSON object string (as the Expression step's does) with the keys
  in the order they were entered. The step reads the keys in the order of the JSON text and also accepts
  a list of `{ "key", "value" }`;
- the format of **datetime values** from the Data API (offsets are converted to UTC);
- the largest export that fits: the records, the file and its base64 are all held in memory, so a very
  large export can run out of memory. 50,000 records worked (see "Performance"); there is no cap
  because no limit has been found yet.

## Performance

Canvas runs with 5000 records per request: 50,000 records in 4.4 seconds and 200,000 records in 24
seconds. With 200 per request (the earlier maximum) 50,000 records took 32 seconds. A step started from
the front end is answered after about a minute and keeps running for at most 10 minutes (as reported by
the platform team, not tested here). The largest export tried is 200,000 records; the memory limit has
not been found yet.

## Tests

`cargo test -p export-records` runs the unit tests and `tests/mod.rs`, which runs the built
component (`just build` first; needs Rust 1.94+ for wasmtime) with a mocked Data API that caps pages
at 100 records and a mocked upload-file. `src/template.rs` and `src/graphql.rs` are
copies of the files with the same name in `data-api-request` (and `data-api-query`), and
`tests/templayed_cases.json` is the same fixture. The value check in `src/filter.rs` is a copy of the
one in `data-api-query/1.0/src/query.rs`. When one copy changes, change the others too.

## Building

From the repository root, `just build` builds every step for `wasm32-wasip2` and copies
`export_records.wasm` into this folder. `just step-lint` checks `function.json` against the WIT.
