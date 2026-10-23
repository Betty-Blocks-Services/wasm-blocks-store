# Data API Request

A WASM version of the ActionsJS "Data API Request" step
([Data-API-request-step](https://github.com/Betty-Blocks-Services/Data-API-request-step)).
It builds a Data API query from a model, a record/collection type, a filter, pagination and
sorting, sends it through the host `betty-blocks-types:data-api` interface (the WASM counterpart of
the ActionsJS `gql` helper), and returns the record, the list of records or the total count.

**Status:** tested on a canvas against real apps: collections and records with nested relations,
a record that does not exist (result `null`), Output type total count, filters with variables and
nested conditions, sorting, paging beyond 200 records (a Take of 500, on the build after the review
fixes), empty results, errors, and Record and Collection results used in loops and updates.

## Options

| Option | Notes |
|---|---|
| Model, Type | Start from one record (`one<Model>`) or a collection (`all<Model>`) |
| Filter | Data API filter without outer braces, e.g. `id: { eq: {{id}} }`. `{{name}}` inserts a value from Variables. Quote text values yourself: `name: { eq: "{{name}}" }` |
| Variables | Map of values for `{{name}}` in the filter |
| Skip, Take | Take defaults to 200 when empty or 0 and is at most 5000. The Data API returns at most 200 records per request (a higher `take` is rejected), so a Take above 200 is fetched in several requests of 200 using Skip, and returned as one list. **Use a unique Sort field for that** (see below) |
| Sort order, Sort field | Sorting is only added when a sort field is given: a property name, or a relation path like `company.name` |
| Query data | The properties and relations to retrieve, e.g. `name company { id name }`. The record's `id` is always added (see below) |
| Output type | `results` or `totalCount` for a collection; ignored for a record |
| Result | Record or Object (one record), Collection or Array (the records) or Number (total count). Bind it as a Collection of the selected model to use the records in steps that need a collection, e.g. a loop that updates each one. The properties in Query data have to exist on that model. |

## Things to know

- **Paging needs a unique sort.** Every page of 200 is a separate request, so with a non-unique
  Sort field (say a name that two records share) a record can come back twice or be skipped between
  two pages. When you give no Sort field, the step sorts by `id` while paging. If you sort on
  something else, make it unique or add a tie-breaker you control. A Take of 200 or less is a
  single request and is not affected.
- **`{{name}}` HTML-escapes the value**, as the ActionsJS step did: `&` becomes `&amp;`, `"`
  becomes `&quot;`, `<` and `>` too. A filter like `name: { eq: "{{name}}" }` with the value
  `A&B` therefore looks for `A&amp;B`. Use `{{{name}}}` or `{{&name}}` to insert the value as it
  is. With `{{name}}` a backslash in a value is doubled, so a value ending in `\` cannot escape
  the closing quote of the string it sits in; the raw tags insert the value untouched.
- **Values become part of the query text**, as in the ActionsJS step. `{{name}}` is safe inside a
  quoted string, but a value inserted outside a string, or with `{{{name}}}`, is code and can
  change the query (that is what makes a variable that holds a whole filter fragment possible).
  The brackets and strings of the finished query are checked, the step only returns the result of
  the query it built for the configured model, and the request runs with the permissions of the
  action's user, but do not insert untrusted input into a filter.
- **Bracket and comment checking.** Strings, block strings (`"""`) and `#` comments (which end at
  any line break) are read the way GraphQL reads them. Unbalanced `{ }`, `[ ]` or `( )`, a string
  that is never closed, and a filter longer than 64 KB after the variables are filled in are
  reported before anything is sent. A bracket error names the line. For Query data it also quotes
  that line; for the filter it does not, because the text can contain values filled in from
  variables.
- Sort field and model name are checked: a sort field may only contain letters, digits, `_` and
  `.`, a model name letters, digits and `_`.

## Differences from the ActionsJS step

The step covers the same use cases. Where it differs on purpose:

- **Errors fail the step** with the Data API's own message (for example
  `in object 'Webuser' the fields 'ida' are not found.`). The ActionsJS step logged the error and
  then crashed with a TypeError.
- **Take above 200 works by paging.** The Data API rejects a `take` above 200. The step fetches
  collection results in batches of 200 (up to 25 requests for 5000 records), sorted by `id` unless
  you set a Sort field, and returns them as one list; it stops early when the data runs out. Every
  request counts towards the Data API read-rate limit. For a record or the total count Take has no
  effect: the request is sent with `take: 200` (the ActionsJS step sent your number).
- **Readable errors for bracket mistakes**, see above. A filter wrapped in one pair of outer
  braces is accepted and unwrapped. Query data is only checked when it is used (not for a total
  count), like the ActionsJS step.
- **Comments in a filter cannot swallow the closing braces.** The step puts a line break after
  the filter and the Query data, so a `# comment` on the last line is harmless; the ActionsJS step
  put the closing brace on the same line.
- **The `id` is always selected.** Later steps (an update, a loop, ...) find a record by its `id`:
  without it the result cannot be used as a Record or Collection (an Update step fails with
  `can't convert undefined to BigInt`). The step adds `id` to the top level of Query data when it
  is missing, so an Object or Array result now always contains `id`. It does not add `id` inside
  relations: write `company { id name }` yourself. An alias such as `id: name` counts as selecting
  `id`, so no real `id` is added then. Only the properties you list are filled in; reading another
  property of the record later gives an empty value.
- **`sortField` is optional**, and **Query data** may not be empty unless only the total count is
  requested. An empty Sort order with a Sort field means ascending (the ActionsJS step sent
  `order: undefined`). The model name is trimmed. An invalid Type, Sort order or Output type is an
  error.
- **A `null` list of results** is an empty list, whether one page or several were fetched.
- **Templating** is flat: `{{x}}` (HTML-escaped), `{{{x}}}` and `{{&x}}` (raw), `{{! comment}}`,
  dotted paths such as `{{webuser.id}}`, and `{{list.length}}` / `{{list.0}}`. It is a Rust port of
  that part of the `templayed.js` library the ActionsJS step used. **Sections are not supported**
  (`{{#list}}...{{/list}}`, `{{^x}}...{{/x}}`, `{{.}}`, `{{../x}}`): a filter that needs a loop or a
  condition, such as one `OR` condition per record of a collection, is built in an Expression step
  and passed in as the filter (or inserted with `{{{filters}}}`). Each of these tags is an
  error, not silently ignored. Differences from templayed.js for what is supported:
  - a tag ends at its first `}}`, so `{{id}}}}` works and the "always put spaces in filter
    objects" advice no longer applies. A raw tag closed with two braces (`{{{x}}`) is left as text
    where templayed rendered it;
  - a `|` in the template stays a `|` (templayed turned it into a line break by accident) and
    `\r\n` is not doubled;
  - a backslash in the template text stays a backslash (templayed turned the template into a
    JavaScript string, so `\n` became a line break and `\"` threw), and a backslash in a value is
    doubled;
  - a line break in a value inserted with `{{name}}` is written as `\n` (or `\r`), so a value from a
    multi-line text field stays inside its quoted string; inside a block string (`"""..."""`),
    where `\n` is not an escape, it shows as the two characters;
  - a missing value (`{{a.b}}` without `a`) renders empty instead of throwing;
  - the rendered text may not grow past 64 KB;
  - numbers print the shortest digits that round-trip but never in exponent form, so `1e21` is
    `1000000000000000000000` where JavaScript prints `1e+21`, and the last digits of integers
    beyond 2^53 can differ.
- Option names are kebab-case (`filter-variables`, `sort-order`, ...) to match the WIT parameters.

## Tests

`src/template.rs` and `src/graphql.rs` are also copied into the
[`data-api-query`](../../data-api-query/1.0/README.md) step: when you change one, change the other.

`just test` runs the unit tests. Two of them compare against the original ActionsJS code:
`tests/generate_templayed_cases.mjs` and `tests/generate_query_cases.mjs` record what the original
`templayed.js` and step 1.7 produce (the queries, the field names and what the step returns for a
mocked answer), and the tests in `src/template.rs` and `src/query.rs` assert that the Rust code
gives the same output. Run `tests/fetch_original.sh` first: it downloads the original at a pinned
commit. Cases where the original throws are listed under `skipped` in `templayed_cases.json`. A
case count in each test makes a regenerated fixture that lost cases fail; update it when you add
cases.

The tests do not run the step through the WIT boundary with a stubbed host (the other steps'
`tests/mod.rs` do; their imports are `wasi:http` or none): the Data API call is behind a
closure in `src/query.rs`, which the unit tests replace.

## Building

Requires Rust with the `wasm32-wasip2` target and `wkg`; `just build` from the repository root.
The host WIT interfaces are vendored in `external/` (`betty-blocks-types@3.0.0`).
