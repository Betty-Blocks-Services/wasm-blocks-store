# Data API Request

A WASM version of the ActionsJS "Data API Request" step
([Data-API-request-step](https://github.com/Betty-Blocks-Services/Data-API-request-step)).
It builds a Data API query from a model, a record/collection type, a filter, pagination and
sorting, sends it through the host `betty-blocks-types:data-api` interface (the WASM counterpart of
the ActionsJS `gql` helper), and returns the record, the list of records or the total count.

**Status:** works against the Data API in a test app, including a collection result used in a loop
that updates each record; the take limit is still being verified.

## Options

| Option | Notes |
|---|---|
| Model, Type | Start from one record (`one<Model>`) or a collection (`all<Model>`) |
| Filter | Data API filter without outer braces, e.g. `id: { eq: {{id}} }`. `{{name}}` inserts a value from Variables. Quote text values yourself: `name: { eq: "{{name}}" }` |
| Variables | Map of values for `{{name}}` in the filter |
| Skip, Take | Take defaults to 200 when empty or 0 and is capped at 5000 (the ActionsJS limit, not verified against the server) |
| Sort order, Sort field | Sorting is only added when a sort field is given |
| Query data | The properties and relations to retrieve, e.g. `name company { id name }`. The record's `id` is always added (see below) |
| Output type | `results` or `totalCount` for a collection; ignored for a record |
| Result | Record or Object (one record), Collection or Array (the records) or Number (total count). Bind it as a Collection of the selected model to use the records in steps that need a collection, e.g. a loop that updates each one. The properties in Query data have to exist on that model. |

## Differences from the ActionsJS step

The step covers the same use cases. Where it differs on purpose:

- **Errors fail the step** with the Data API's own message (for example
  `in object 'Webuser' the fields 'ida' are not found.`). The ActionsJS step logged the error and
  then crashed with a TypeError.
- **Readable errors for bracket mistakes.** Unbalanced `{ }`, `[ ]` or `( )` in the filter or the
  query data are reported before anything is sent. A filter wrapped in one pair of outer braces is
  accepted and unwrapped.
- **`sortField` is optional**, and **Query data** may not be empty unless only the total count is
  requested.
- **Templating** is a Rust port of the `templayed.js` subset the ActionsJS step used (`{{x}}`
  HTML-escaped, `{{{x}}}`/`{{&x}}` raw, `{{! comment}}`, `{{#list}}`, `{{^empty}}`, `{{.}}`,
  `{{../x}}`). Differences from templayed.js:
  - a tag ends at its first `}}`, so `{{id}}}}` works and the "always put spaces in filter
    objects" advice no longer applies;
  - a `|` in the template stays a `|` (templayed turned it into a line break by accident) and
    `\r\n` is not doubled;
  - a missing intermediate value (`{{a.b}}` without `a`) renders empty instead of throwing;
  - a section over a non-empty string, object or number renders nothing (templayed iterated over
    the characters of a string);
  - an unclosed or unmatched section is reported as an error.
- **The `id` is always selected.** Later steps (an update, a loop, ...) find a record by its `id`: without it the
  result cannot be used as a Record or Collection (an Update step fails with `can't convert undefined to
  BigInt`). The step adds `id` to the top level of Query data when it is missing, so an Object or Array
  result now always contains `id`. It does not add `id` inside relations: write `company { id name }`
  yourself. Only the properties you list are filled in; reading another property of the record later
  gives an empty value.
- Option names are kebab-case (`filter-variables`, `sort-order`, ...) to match the WIT parameters.

Values are put into the query as text, as in the ActionsJS step: a text value containing a quote
can change the query. Do not insert untrusted input into a filter.

## Tests

`just test` runs the unit tests. Two of them compare against the original ActionsJS code:
`tests/generate_templayed_cases.mjs` and `tests/generate_query_cases.mjs` record what the original
`templayed.js` and step 1.7 produce, and the tests in `src/template.rs` and `src/query.rs` assert
that the Rust code gives the same output (see the header of each script for how to regenerate).

## Building

Requires Rust with the `wasm32-wasip2` target and `wkg`; `just build` from the repository root.
The host WIT interfaces are vendored in `external/` (`betty-blocks-types@3.0.0`).
