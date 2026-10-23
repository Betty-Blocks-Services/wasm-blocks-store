# Data API Query

Runs a Data API query you write yourself. Unlike [`data-api-request`](../../data-api-request/1.0/README.md),
which builds the query for one selected model, this step has no Model option: the query is the
whole input, so one step can read several models at once. It sends the query through the host
`betty-blocks-types:data-api` interface and returns the `data` of the response.

**Status:** tested on a canvas in a production-zone app: a query over two models at once, a query without
`query { }` around it, an alias, a record with nested relations (one of them empty), the Data API's own
error for an unknown field, an unbalanced bracket, `{{variables}}` in a quoted string, unquoted and next to
closing braces, a raw `{{{variable}}}` that ends in a backslash, an unsafe value outside quotes (refused)
next to a plain one (works), and a document with two operations and a direct mutation (both refused or
failed on the Data API side, see "Behaviour"). Unit-tested only: sections, a missing record, a value with
a quote, `{{{...}}}` with a quote in the middle, templates that read differently from the rendered query
(a quote in a `{{! comment}}`, a backslash in front of a tag), and the error texts added after the
review. See "Known limits" below for two things that did not behave as hoped.

## Options

| Option | Notes |
|---|---|
| Query | The Data API query, e.g. `query { allWebuser(take: 5) { results { id name } } allCompany { totalCount } }`. If it does not start with `{` or `query`/`mutation`/`subscription`/`fragment`, the step wraps it in `query { ... }`, so `allWebuser { results { id } }` works too |
| Variables | Map of values for `{{name}}` in the query |
| Schema model | Describes the shape of the response data, to type the fields of the Result |
| Result | Object: the `data` of the response, one property per top-level field in the query |

For `query { allWebuser { results { id } } allCompany { totalCount } }` the Result is
`{ "allWebuser": { "results": [...] }, "allCompany": { "totalCount": 4 } }`. If you give a field an
alias (`users: allWebuser { ... }`) the alias is the property name.

## Variables

Values are put into the query as text with `{{name}}` before it is sent. **The values go into the
query text, so what a value may be depends on where its tag sits:**

- **Inside quotes** (`name: { eq: "{{name}}" }`): any text is safe. `{{name}}` escapes `&`, `"`, `<`
  and `>` and doubles every backslash, so a value cannot close or escape the string it sits in.
  (`&` becomes `&amp;`, so a filter on a value containing `&` will not match it: use `{{{name}}}` for
  that, with a value you trust.) A value with a line break cannot go inside quotes: the step fails
  with a message that says so.
- **Outside quotes** (`id: { eq: {{id}} }`): the value is query code, so the step only accepts a
  number, `true` or `false`, a name (letters, digits, `_`, `-` and `.`), or a comma-separated list of
  those (a list value prints as `1,2,3`, so `in: [{{ids}}]` works). Anything else fails the step with an
  error that names the variable. Without this, a value like `7 } }) { ... } other: allUser { ... }` typed by an end
  user could add filters or fields on other models.
- **Raw tags** (`{{{name}}}` and `{{&name}}`) insert the value untouched and are not checked. Use them only
  for query code you wrote or trust, never for text an end user can type.
- **A name is still a name.** A tag in the place of a field, an argument or an operation keyword
  (`{ {{f}} }`) lets its value pick any field, model or operation that the Data API knows. An empty or
  missing value outside quotes is accepted and leaves a query the Data API rejects (`eq: }`). Whether a
  value is inside quotes is judged on the finished query, read the way the Data API reads it.

More about the templating (the same as `data-api-request`, a Rust port of the `templayed.js` subset the
ActionsJS Data API Request step used):

- `{{a.b}}` reads a nested value, `{{list.0}}` an item of a list and `{{list.length}}` its length.
  A list prints as `1,2,3` and an object as `[object Object]` (as JavaScript would print them).
  `{{! comment}}` is dropped.
- Sections (`{{#list}}...{{/list}}`, `{{^list}}`) are **not supported** and give an error: build
  that part of the query in an Expression step and insert it with `{{{name}}}`.
- A tag ends at its first `}}` (a raw `{{{name}}}` at its first `}}}`), so `{ eq: {{id}}}}` works. A
  missing value is inserted as nothing. A `|` in the query stays a `|` (templayed turned it into a line
  break).
- The query may not grow beyond 64 KB after the variables are filled in.

```graphql
query {
  allWebuser(where: { name: { eq: "{{name}}" }, id: { eq: {{id}} } }) {
    results { id name }
  }
}
```

GraphQL's own `$variable` declarations are not supported: the step sends the query with no
variables. Use `{{name}}` instead.

## Behaviour

- **Nothing is changed in the query** apart from the variables and the optional `query { }`
  wrapper. In particular no `id` is added to your fields and there is no paging: the Data API
  returns at most 200 records per request, so page yourself with `skip` and `take` (200 or less)
  in the query text. `data-api-request` pages for you.
- **Errors fail the step** with the Data API's own message (for example
  `in object 'Webuser' the fields 'ida' are not found.`), or a fixed text when an error has no
  message. A response whose `errors` is not empty always fails the step and its `data` is dropped,
  also when part of it is there. For a mutation that means part of it may have run even though the
  step reports an error.
- **Readable errors for bracket and string mistakes.** Unbalanced `{ }`, `[ ]` or `( )` and strings
  that are never closed are reported with the line number before anything is sent. This step's own
  messages do not quote the line, because it can hold values from Variables. (An error that comes
  back from the Data API itself is shown as the Data API wrote it, and may quote part of the query.)
  The check reads strings, block strings and `#` comments the way the GraphQL specification does, so
  brackets inside them are ignored. Line numbers refer to the query after the variables were filled in
  and leading and trailing blank lines were removed.
- **Not a Record or Collection.** The Result is an object, not a Betty Blocks Record or Collection
  of a model: the step does not know which model(s) the query reads. Use `data-api-request` to feed
  the records of one model into steps that need a Collection or Record.
- **Mutations and several operations** are sent as written. In the live tests the Data API did not run
  them: a document with two operations failed on the canvas with `gRPC call failed: status: Unknown,
  message: "Internal Server Error"` (no detail), and a direct mutation such as `updatePerson` was refused
  (per the Data API's own rules, mutations only go through actions: `mutation { action(id: "...",
  input: ...) }`). That is one try of each; running an action that way was not tried. The step sends no GraphQL variables, so an `$input`
  variable would not be filled in. The step is meant for reading.
- **Variables must be a map** of names to values. An empty Variables option means none.
- **Data API limits apply.** The documentation allows at most 10 top-level fields per request, aliases
  included. A query with 13 failed as a whole and one with 7 ran; the boundary was not tried. Split a
  bigger query over several steps. Every
  step run is one request towards the Data API rate limit.

## Known limits

- **A relation that is `null` can fail the step.** With a record whose relation is empty,
  `{ oneContact(where: { id: { eq: 42 } }) { id parent { id } } }` failed with
  `Item 'parent' is not an Object'. (property name changed here)`, and the same query without that relation worked.
  The message is not from this step: it comes from the platform, most likely when the Result is
  bound as an Object. Not isolated further (whether a Schema Model changes it was not checked). A missing
  record (`oneContact: null`) probably fails the same way; that was not tried. Leave out relations that can
  be empty, or query them from the other side.
- **A backslash in a value was not confirmed to match.** `{{name}}` doubles every backslash so a value
  cannot escape its string. A record whose text ended in a backslash did not come back from a
  filter on `{{name}}` in the one live try, and why was not investigated. The raw tag
  `{{{name}}}` with such a value is rejected by the step, as intended. The same escaping is in
  `data-api-request`.

## Tests

`cargo test -p data-api-query` runs the unit tests. `src/template.rs` and `src/graphql.rs` are
copies of the files with the same name in `data-api-request`. In `template.rs` only error messages and
doc comments say "query" where the original says "filter". `tests/templayed_cases.json` is the same
fixture as in `data-api-request/1.0/tests/`, where it is regenerated against the original
`templayed.js` with `generate_templayed_cases.mjs`. When one of the two steps changes its copy, change
the other one too.

## Building

Requires Rust with the `wasm32-wasip2` target and `wkg`; `just build` from the repository root.
The host WIT interfaces are vendored in `external/` (`betty-blocks-types@3.0.0`).
