# Data API Query

Runs a Data API query you write yourself. Unlike [`data-api-request`](../../data-api-request/1.0/README.md),
which builds the query for one selected model, this step has no Model option: the query is the
whole input, so one step can read several models at once. It sends the query through the host
`betty-blocks-types:data-api` interface and returns the `data` of the response.

**Status:** tested on a canvas in a production-zone app: a query over two models at once, a query without
`query { }` around it, an alias, a record with nested relations (one of them empty), the Data API's own
error for an unknown field, an unbalanced bracket, `{{variables}}` in a quoted string, unquoted and next to
closing braces, and a raw `{{{variable}}}` that ends in a backslash. Not tried live: sections, a
missing record, a value with a quote, and `{{{...}}}` with a quote in the middle (unit-tested only). See
"Known limits" below for two things that did not behave as hoped.

## Options

| Option | Notes |
|---|---|
| Query | The Data API query, e.g. `query { allWebuser(take: 5) { results { id name } } allCompany { totalCount } }`. If it does not start with `{` or `query`/`mutation`/`fragment`, the step wraps it in `query { ... }`, so `allWebuser { results { id } }` works too |
| Variables | Map of values for `{{name}}` in the query |
| Schema model | Describes the shape of the response data, to type the fields of the Result |
| Result | Object: the `data` of the response, one property per top-level field in the query |

For `query { allWebuser { results { id } } allCompany { totalCount } }` the Result is
`{ "allWebuser": { "results": [...] }, "allCompany": { "totalCount": 4 } }`. If you give a field an
alias (`users: allWebuser { ... }`) the alias is the property name.

## Variables

Values are put into the query as text with `{{name}}` before it is sent. This is the same
templating as `data-api-request` (a Rust port of the `templayed.js` subset the ActionsJS Data API
Request step used), so the rules are the same:

- `{{name}}` inserts the value HTML-escaped (`&` becomes `&amp;`, `"` becomes `&quot;`, `<` and `>`
  too) and with every backslash doubled, so a value cannot close or escape the string it sits in.
  `{{{name}}}` and `{{&name}}` insert the value untouched.
- `{{a.b}}` reads a nested value, `{{list.0}}` an item of a list and `{{list.length}}` its length.
  `{{! comment}}` is dropped.
- Sections (`{{#list}}...{{/list}}`, `{{^list}}`) are **not supported** and give an error: build
  that part of the query in an Expression step and insert it with `{{{name}}}`.
- A tag ends at its first `}}`, so `{ eq: {{id}}}}` works. A missing value is inserted as nothing.
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

**Values become part of the query text.** `{{name}}` is safe inside a quoted string, but a value
inserted outside a string, or with `{{{name}}}`, is code and can change the query. Only insert
values you trust there, and never insert untrusted input into a query.

## Behaviour

- **Nothing is changed in the query** apart from the variables and the optional `query { }`
  wrapper. In particular no `id` is added to your fields and there is no paging: the Data API
  returns at most 200 records per request, so page yourself with `skip` and `take` (200 or less)
  in the query text. `data-api-request` pages for you.
- **Errors fail the step** with the Data API's own message (for example
  `in object 'Webuser' the fields 'ida' are not found.`). A response with errors is never returned
  as a Result, even when it also has partial data.
- **Readable errors for bracket and string mistakes.** Unbalanced `{ }`, `[ ]` or `( )` and strings
  that are never closed are reported with the line number before anything is sent (the line is not
  quoted, because it can hold values from Variables). The check reads strings, block strings and
  `#` comments the way the Data API does, so brackets inside them are ignored. When the query uses
  `{{...}}` tags, line numbers count after the variables were filled in.
- **Not a Record or Collection.** The Result is an object, not a Betty Blocks Record or Collection
  of a model: the step does not know which model(s) the query reads. Use `data-api-request` to feed
  the records of one model into steps that need a Collection or Record.
- A query that is a `mutation` is sent as it is. The step is meant for reading.
- **Data API limits apply.** A request may contain at most 10 top-level fields, aliases included;
  more fails the whole request with an error, so split a bigger query over several steps. Every
  step run is one request towards the Data API rate limit.

## Known limits

- **A relation that is `null` can fail the step.** With a record whose relation is empty,
  `{ oneContact(where: { id: { eq: 42 } }) { id parent { id } } }` failed with
  `Item 'parent' is not an Object'. (property name changed here)`, and the same query without that relation worked.
  The message is not from this step: it comes from the platform, most likely when the Result is
  bound as an Object. Not isolated further (whether a Schema Model changes it was not checked). Leave
  out relations that can be empty, or query them from the other side.
- **A backslash in a value was not confirmed to match.** `{{name}}` doubles every backslash so a value
  cannot escape its string. A record whose text ended in a backslash did not come back from a
  filter on `{{name}}` in the one live try, and why was not investigated. The raw tag
  `{{{name}}}` with such a value is rejected by the step, as intended. The same escaping is in
  `data-api-request`.

## Tests

`cargo test -p data-api-query` runs the unit tests. `src/template.rs` and `src/graphql.rs` are
copies of the files with the same name in `data-api-request` (only two error messages in
`template.rs` say "query" instead of "filter"), including the comparison with the original
`templayed.js` in `tests/templayed_cases.json`. When one of the two steps changes its copy, change
the other one too.

## Building

Requires Rust with the `wasm32-wasip2` target and `wkg`; `just build` from the repository root.
The host WIT interfaces are vendored in `external/` (`betty-blocks-types@3.0.0`).
