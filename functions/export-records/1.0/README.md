# Export to CSV or Excel

Exports the records of a model to a CSV or Excel (XLSX) file, stores the file through the host
`betty-blocks-types:upload-file@4.0.0` interface and returns the file reference. A Rust port of the
ActionsJS step `export-to-csv` 3.4 (Betty-Blocks-Services/rob-block-store-functions).

**Status:** built for `wasm32-wasip2` and unit-tested (`cargo test -p export-records`). **Not yet run
on a canvas.** Until it is, this step does not meet the bar in [CONTRIBUTING.md](../../../CONTRIBUTING.md).
Open questions for the first canvas test are listed under "Not tested yet".

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
| Filename | Without extension: `.csv` or `.xlsx` is added (a name that already ends in it does not get it twice). Empty means `export` |
| Property mapping | KEY = column header, VALUE = database name (`created_at`) or `relation.property` for a belongs-to relation. Columns follow the order of the map |
| Format mapping for export columns | KEY = column header, VALUE = `Date\|<format>` (also `Datetime\|` and `Time\|`), or `Decimal\|<separator>` / `Price\|<separator>` |
| Use BOM | CSV only; default on |
| As | The file reference (Text) |

## Behaviour

- **All matching records are exported**, fetched 200 per request (the Data API's page size) until
  `totalCount` is reached. Every page is one request towards the Data API rate limit.
- **The records are always sorted**, by `id` when no sort property is chosen, so paging cannot export a
  record twice or skip one. With a sort property that is not unique this can still happen, as in the
  ActionsJS step.
- **CSV** is written like the `export-to-csv` 0.2 package the ActionsJS step used: `\r\n` after every
  line (the last one too), text in double quotes with `"` doubled, `TRUE`/`FALSE` for yes/no values,
  numbers without quotes, and an empty value as `""`.
- **XLSX** has one sheet, named after the model to save the file into (as in the ActionsJS step;
  invalid characters are removed and the name is cut to 31 characters). Numbers are number cells,
  yes/no values boolean cells, text is text, and empty values leave the cell empty.
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
- **`Time|<format>`** works on time values (`13:45:00`). The ActionsJS step listed it but ignored it.
- **A has-many relation or object** in a column is written as JSON instead of `[object Object]`.
- **A header that holds the delimiter, a quote or a line break** is quoted, so it cannot break the file.
- **A property path with other characters than letters, digits and `_`** fails the step instead of
  being turned into a different field name.
- Option names are kebab-case (`model-source`, not `modelSource`): an action that used the ActionsJS step
  has to be configured again.

## Not tested yet

To check on the first canvas run:

- whether the platform turns a Select value into the matching WIT enum case (`file-type`, `sort-order`, `delimiter`) for a custom component
  (the native HTTP step does it for its `method` and `protocol` enums, inside a record), and whether My BB's
  function.json editor accepts a Select on it after an IDE upload (it refused one on a WIT `string`);
- how the platform passes an empty optional **Sort property** (expected `null` or an empty list; both are
  handled);
- whether a **Map** reaches the step as a JSON object string (as the Expression step's does) with the keys
  in the order they were entered. The step reads the keys in the order of the JSON text and also accepts
  a list of `{ "key", "value" }`;
- the format of **datetime values** from the Data API (offsets are converted to UTC);
- the largest export that fits: the whole file is built in memory and sent to the upload as base64.

## Tests

`cargo test -p export-records` runs the unit tests. `src/template.rs` and `src/graphql.rs` are
copies of the files with the same name in `data-api-request` (and `data-api-query`), and
`tests/templayed_cases.json` is the same fixture. The value check in `src/filter.rs` is a copy of the
one in `data-api-query/1.0/src/query.rs`. When one copy changes, change the others too.

## Building

From the repository root, `just build` builds every step for `wasm32-wasip2` and copies
`export_records.wasm` into this folder. `just step-lint` checks `function.json` against the WIT.
