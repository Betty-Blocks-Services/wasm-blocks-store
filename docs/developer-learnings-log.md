# WASM Actions — developer learnings log (for Betty Blocks developers using WASM features)

Shared, living log of non-obvious, hands-on-discovered facts worth folding into the crash-course docs in `crash-course/` for Betty Blocks developers (people building applications on BB who use its WASM Actions feature) — as opposed to feedback aimed at BB's own product developers, which is tracked separately, outside this repo.

This log started in one person's local learning project and now lives here so it compounds across everyone using this repo — see `../.claude/skills/wasm-actions/SKILL.md` for how findings get logged here going forward, and `../CONTRIBUTING.md` for the discipline around verifying a claim before writing it down as fact (see the entries below marked RETRACTED/corrected for why that discipline exists).

Each entry: the fact, how it was found, and where it should eventually land in the docs.

---

## The Data API does not run a document with two operations (opaque "Internal Server Error") and refuses direct mutations (2026-10-23)

**Fact:**
- A document with two named operations sent through the Data API Query step, `query A { onePerson(where: { id: { eq: <id> } }) { id } }` followed by `query B { allCountry { totalCount } }`, failed with `gRPC call failed: status: Unknown, message: "Internal Server Error", details: []`. No data came back and the message does not say what was wrong, so a builder cannot tell a second operation from a server fault.
- A direct mutation, `mutation { updatePerson(id: <id>, input: { webBio: "x" }) { id webBio } }`, was refused by the Data API. Per Marcel, the Data API does not accept direct mutations at all: mutations only go through actions, as `mutation { action(id: "<action id>", input: $input) }`. The exact refusal text was not recorded.

**How found:** run on the canvas through the `data-api-query` step on a production-zone app, to answer a review question: can text injected into a query run a second operation or a mutation? On this evidence it cannot: the two-operation document failed and the mutation was refused. The step sends no GraphQL variables (`request(..., "{}")`), so an `$input` variable for the action mutation would not be filled in.

**Not tested:** running an action through `mutation { action(id: "...", input: ...) }` from the step, and whether a document with one operation and unused extra fragments or a named operation among several is accepted.

**Where this lands:** the `data-api-query` README, "Behaviour".

**Status:** both outcomes seen firsthand on the canvas on 2026-10-23.

---

## A `null` relation in a Data API response made a step's Object result fail with "Item '...' is not an Object'" (2026-10-23)

**Fact:** a hand-written query on one record, `{ oneContact(where: { id: { eq: <id> } }) { id country { id name } state { abbreviation } parent { id } children { id } } }`, failed on the canvas with `Item 'parent' is not an Object'.` (property name changed here) when the `parent` relation was `null`. The same query without `parent { id }` worked and returned the other relations (an empty to-many relation, `children`, came back as `[]` without trouble).

**How found:** two canvas runs of the `data-api-query` step on a production-zone app; the step returns the Data API's `data` as a JSON string typed with a named alias.

**Not known:** which layer raises the message (it is not one of the step's own errors; the platform's Result binding is the likely one, unverified) or whether a Schema Model on the Result changes it. A missing record (`oneContact: null`) was not tried and is probably the same case.

**Where this lands:** a platform question for the product team once isolated; until then the `data-api-query` README lists it under "Known limits".

**Status:** the failing and the working query were both run on the canvas; the layer that raises the error is not confirmed.

---

## Data API limits that matter when you write queries by hand: 10 top-level fields per request, 4 nested layers, no aggregates, nested lists take only where/sort (2026-10-23)

**Fact, from first-hand use of a production app (Marcel) and the app's introspection response:**
- **A request with more root fields than the Data API allows fails as a whole** (error, no data). Evidence: a query with 13 root fields (several aliases of one `all<Model>` field) was rejected; a 7-field one with deep nesting ran. The limit is taken to be 10 top-level fields, aliases included: that number comes from the docs as Marcel reads them, and the boundary (10 runs, 11 fails) was not tested. That app splits its analytics queries over 2 Data API query steps (2 requests). The documented rate limit (e.g. 200 requests/minute on live production apps, per Marcel's reading of the docs) is separate, and each step run costs one request.
- **Nested relations: max 4 layers** (Marcel's reading of the docs, not tested by us; the deepest used so far is 3). Nested relations are taken not to count towards the 10 (not tested, no evidence beyond the 7-field query with deep nesting that ran).
- **No aggregates in the schema:** every `<Model>List` type has only `results` and `totalCount` (confirmed in the default introspection response). That app works around it with a precomputed metrics model. Not tried: querying for a sum/avg field, since none is declared.
- **Nested to-many fields accept only `where` and `sort`, no `take`/`skip`** (schema). Whether such a nested list is capped is **not verified**, so counting `<relation> { id }` per parent may undercount for large relations; exact counts need a filtered root `totalCount`.
- **BB introspection quirks:** scalar fields are reported with `kind: OBJECT` (name `String`, `Int`, ...), some empty enums come back as `enumValues: [""]` (a string, not an object), and the default query's output is mostly input types (filter/sort/relation/operator) that can be derived from the models.

**How found:** Marcel's own queries on a production app, and the app's default introspection response.

**Where this lands:** the `data-api-query` step README notes the 10-field limit; `data-api-request` is not affected (it sends one field per request).

**Status:** the 10-field limit is from the author's own tests; the schema facts were read from an introspection response on 2026-10-23; the 4-layer limit and the nested-list cap are not verified by us.

---

## Expression step: how it evaluates, how the Variables map works, what fails and why, and proven patterns (2026-10-09)

**Where each part comes from, so you know how far to trust it:**

- **Canvas-tested** (Marcel, October 2026, building an action for an MCP-based search agent; the doc was
  updated three times, the last adding the backslash/`String.raw` findings): every rule in 4 and the patterns
  in 5. Error texts are quoted as they appeared.
- **Source-confirmed** (2026-10-09): the engine, read in
  [`bettyblocks/native-wasm-components`](https://github.com/bettyblocks/native-wasm-components)
  `functions/expression/1.0/` (`src/lib.rs`, `includes/templated.js/templayed.js`). Two rules first only
  inferred from error messages (the prefix rule, the `}}` rule) are confirmed by it.
- **Replayed locally** (2026-10-09): edge cases marked *(replayed)* were run in Node against the real
  `templayed.js` with the same wrapper `lib.rs` uses. Node is not QuickJS: same behaviour, but the exact
  error wording may differ from canvas. These were not run on a real canvas.

### 1. How the step works

The step evaluates one JavaScript **expression** and returns its value. It is a Rust WASM component that
embeds the QuickJS engine (`rquickjs`) plus [templayed.js](https://github.com/archan937/templayed.js) 0.2.3,
a small Mustache-style template library. Per call, `lib.rs`:

1. parses the **Variables** map (it arrives as a JSON string) into an object;
2. **template phase:** runs the Expression text through templayed with that object, replacing every
   `{{ ... }}` / `{{{ ... }}}` with *text*;
3. **JavaScript phase:** evaluates `new Function("return " + <resulting text>)()`;
4. returns the value JSON-stringified (`undefined` becomes `null`).

Everything below follows from that order: values only exist in the template phase, and in the JavaScript
phase only text is left.

### 2. Getting values in: the Variables map

- Add values to the step's **Variables** map (key → value), e.g. key `data`, value `masterDataResponse`.
- They can **only** be referenced inside `{{ }}` / `{{{ }}}`. Outside the brackets `data` alone gives
  `Error: data is not defined` and `vars.data` gives `Error: vars is not defined`.
- Types survive until the template phase: the map reaches the component as a JSON-object string with typed
  values (a number stays a number, a record is the full object). The WIT parameter is a plain `json-string`;
  the key/value table is `function.json`-only (`"type": "Map"`).

### 3. `{{x}}` vs `{{{x}}}`

| Syntax | Behaviour |
|---|---|
| `{{name}}` | Inserts the value **HTML-escaped**: `&` → `&amp;`, `"` → `&quot;`, `<` → `&lt;`, `>` → `&gt;` |
| `{{{name}}}` | Inserts the value **raw** |

Canvas-tested for `"`; the other three are from the source and *(replayed)*. Use triple brackets whenever
the value can contain any of them: JSON, GraphQL filter text, URLs with `&`.

### 4. What works and what fails

**4.1 The prefix rule (source-confirmed).** templayed does not parse what is inside the brackets. It treats
it as a property path and generates `vars[vars.length - 1]` + `.` + *your text*. So **the first identifier
is always read as a key of the Variables map**, and after it everything is ordinary JavaScript, run in the
template phase:

| You write | What actually runs | Result |
|---|---|---|
| `{{ data.list.map(x => x.id) }}` | `vars[…].data.list.map(x => x.id)` | works |
| `{{ JSON.stringify(data) }}` | `vars[…].JSON.stringify(data)` | `cannot read property 'stringify' of undefined` |
| `{{ data \| json }}` | `vars[…].data \| json` | `json is not defined` (no filters; `\|` is bitwise-or) |
| `{{{ (() => { … })() }}}` | `vars[…].(() => …` | `expecting field name` (syntax error) |

So **start every bracket section with a variable name**, then use normal JavaScript (canvas-confirmed:
property paths, arrow functions, `.map` / `.reduce` / `.find` / `.join` / `.trim` / `.push`, object and array
literals, `if`/`else` and `const` inside arrow-function bodies). Anything that doesn't start with a variable
name (`JSON`, `(`, `Object`, `Math`) fails; do that work *outside* the brackets or inside a lambda after the
first identifier.

**4.2 Never write `}}` inside the code (source-confirmed, replayed).** A tag ends at the **first `}}`** (the
pattern is a lazy `\{\{…}}+`). `list.map(x => ({k:{v:x}}))` fails; `list.map(x => ({k:{v:x} }))` works.
Keep closing braces apart: `} }`, `}); }`.

**4.2b Inside the brackets: single quotes only, one line per section, no backslashes (canvas rule, cause
source-confirmed and replayed).** Before it looks for tags, templayed escapes the *whole* expression text so
it can be embedded in a string: every `"` becomes `\"` and every real line break becomes a literal `\n`.
That also hits the code *inside* the brackets, where those characters are no longer valid JavaScript:

| Inside a bracket | What the code becomes | Result |
|---|---|---|
| a double quote, `"v"` | `\"v\"` | syntax error *(replayed)* |
| a real line break between tokens | a literal `\n` outside any string | syntax error *(replayed)*; the canvas shows it as `unexpected token in expression: '\'` |

So: use **single quotes**, keep each `{{{ ... }}}` **on one line** (a line break *between* two single-line
brackets is fine), and avoid backslashes. The canvas finding was that `'\n'` worked in one expression and
failed in another with `Error: unexpected token in expression: '\'`, unchanged after the backtick and `${`
were rewritten. A `'\n'` that sits inside quotes on a single line replays fine, so the likelier culprit in the
failing expression is a double quote or line break near it, but that was not proven for that expression.
Either way the safe habit is the same: use a separator without backslashes, e.g. `.join(' ### ')`.
`.filter(Boolean)` works inside the brackets (canvas, and *(replayed)*), as long as it isn't the first
identifier.

**4.3 Missing values.**
- A **missing top-level key or property** inserts an **empty string**, not the word `null` *(replayed)*. If
  the whole expression is just that, the result is `null` (`{{ data.map }}` on an object gave `null` on
  canvas); inside a bigger expression it leaves a hole: `1 + {{missing}}` becomes `1 + `, a syntax error.
- A **missing nested property** throws: `Error: cannot read property 'results' of undefined`. Check the
  nesting. A GraphQL response stored in a variable usually has its own `data` level, so the path becomes
  `data.data.<alias>.results`.

**4.4 Objects become `[object Object]`.** Every inserted value is converted to text, so `{{{data}}}` on an
object gives `[object Object]`. Do the object work **inside** the brackets (4.1) and return a string; you
can't `JSON.stringify` the variable in the template phase because of the prefix rule.

**4.5 JavaScript outside the brackets.** Plain JavaScript with the `JSON` global (canvas-confirmed:
`JSON.stringify(...)`, `.slice()`). The inserted text is **pasted into the source**, so wrap it in backticks
to make it a string literal: `` `{{{filters}}}` ``. Without backticks it is read as code and usually fails
(`testing is not defined` for a plain string value, which is in the component's own tests). Caveats
*(replayed)*:

- A value containing a **backtick** breaks the expression; one containing **`${…}`** is *evaluated*
  (`x${1+1}y` becomes `x2y`), so it is a code-injection point if the value is untrusted.
- A **backslash** in the text outside the brackets is interpreted by the template phase first: `'a\nb'` has a
  real newline in it by the time the JavaScript phase sees it, and fails. Backslashes *inside* the brackets:
  see 4.2b.
- **Backslashes in the inserted data** also matter, once the text lands in backticks: a `\u` or `\x` in free
  text is read as an escape and causes a syntax error (`Invalid Unicode escape sequence`, *(replayed)*).
  Canvas finding: for free text, put **`String.raw` in front of the backticks**
  (`` String.raw`{{{ ... }}}` ``), and JavaScript leaves backslashes in the data alone. Replayed: with
  `String.raw`, `C:\users\x \u12` comes through intact.
- **`String.raw` does not help with a backtick or `${` in the data** *(replayed)*: a backtick still ends the
  string early and `${1+1}` is still *evaluated*.
- **Backticks and `${` inside the brackets (canvas advice: avoid them; cause inferred).** The replay found no
  template-phase problem with a template literal *used as code* inside a bracket (`` x => `a${x}` `` runs
  fine). What breaks is a bracket whose **result text** contains a backtick or `${`, because that text is
  pasted into the surrounding backtick string. Avoiding them stays sensible, but the reason is the output,
  not the code.
- **A leading newline makes the result `null`:** the engine evaluates `return ` + your text, and `return`
  followed by a line break returns `undefined`. Start the expression on its first line.

**4.6 Extras the canvas tests didn't cover (source-confirmed, replayed).** templayed is Mustache-compatible,
so sections `` `{{#items}}{{.}},{{/items}}` `` (loop over an array / show a block if `true`), inverted
sections `{{^items}}…{{/items}}` and comments `{{! note }}` also work in the template phase.

### 5. Proven patterns

**5.1 Escape text for use inside a JSON string**, e.g. a filter string that goes into an HTTP request body:

```javascript
JSON.stringify(`{{{filters}}}`).slice(1, -1)
```

Input `city:{matches:"Atlanta"}` gives `city:{matches:\"Atlanta\"}`. Use **one** `JSON.stringify`: applying it
twice adds another layer of escaping and extra `\"` around the value.

**5.2 Turn an object into compact text.** One bracket section per list, each starting with the variable name
and returning a string, the whole thing inside backticks:

```javascript
`# SPECIALTIES (id|title)
{{{ data.data.specialties.results.map(r => r.id + '|' + r.title.trim()).join('\n') }}}
# GD TAGS (ids|title)
{{{ data.data.gdTags.results.reduce((acc, r) => { const t = (r.title || '').trim(); const g = acc.find(x => x.k === t.toLowerCase()); if (g) { g.ids.push(r.id); } else { acc.push({ k: t.toLowerCase(), t: t, ids: [r.id] }); } return acc; }, []).map(g => g.ids.join(',') + '|' + g.t).join('\n') }}}`
```

On canvas this reduced a ~13,000-token JSON response to a few thousand tokens, which matters when the next
step is an LLM call. It uses `.join('\n')`, which works as a single-line bracket; if you ever see
`unexpected token in expression: '\'`, switch to a separator like `' ### '` (see 5.3 and 4.2b).

**5.3 Turn a list of records into compact text (search results).** One record per person, fields separated by
` | `, records by ` ### `. No backslashes anywhere, single quotes only, free text inside `String.raw` so
backslashes in the data stay intact:

```javascript
String.raw`{{{ data.map(p => ['id:' + p.id, p.fullname, (p.organization && p.organization.name) || '', 'tags:' + (p.gdTags || []).map(t => t.title.trim()).join(', '), 'notes:' + (p.notes || []).map(n => n.notes || '').join(' / ')].join(' | ')).join(' ### ') }}}`
```

**Remaining risk:** a backtick or `${` in the free text breaks (or evaluates inside) the outer string. Rare in
bios and notes, but they can't be filtered out inside the brackets without the characters that break the
template engine. (Replayed: this pattern, with a shortened record, produces the expected one-line output.)

### 6. Debugging

- **Log the step output after every change.** The template phase changes values silently (`&quot;`,
  `[object Object]`, empty strings, `null`).
- **Build up in small steps:** first `` `{{{ data.data.specialties.results.length }}}` `` (path and access),
  then `.map(...)`, then the full expression.
- **Read the error to see which phase failed:**
  - `expecting field name`, or `cannot read property 'x' of undefined` on something that isn't a variable
    (`JSON`) → the **template phase** (4.1).
  - `unexpected token in expression: '\'` → a `"` or a line break inside a bracket, or a backslash (4.2b).
  - `invalid property name` and other plain syntax errors → usually a broken tag (an unclosed `{{`, or a
    `}}` inside the code, 4.2) left in the text that then failed in the JavaScript phase.
  - `X is not defined` where X is a *value you inserted* → the **JavaScript phase**: missing backticks or
    quotes (4.5).
  - `cannot read property 'results' of undefined` on a real variable → a wrong nested path (4.3).

### 7. If you are writing a custom step that wants the same `{{var}}` behaviour

Nothing in the platform does this for you: the live test of 2026-10-07 (see the entry "The WASM counterpart of the ActionsJS `gql` helper is the host `betty-blocks-types:data-api` interface" above)
showed a step component receives `{{id}}` placeholders **unreplaced**, and nothing downstream substitutes
them. The native Expression and HTTP steps do it *inside the component*, so a Rust step that wants the same
has to bring its own substitution. Two design points from the above:

- **Decide whether values are inserted as text or kept as typed values.** Text insertion is the reason for
  `[object Object]`, the backtick rule and the escaping rules. For GraphQL especially, prefer passing values
  as **GraphQL variables** (`variables` in the request, declared `$name` in the query) over splicing them
  into the query string; that avoids the Filter-field injection described in the entry on the Data API
  step's Filter field.
- **Escape on insertion, not afterwards**, for whichever context the value lands in (JSON body, query,
  JS string).

**Where this lands:** the crash course's known-issues section (not edited yet).

**Status:** canvas-confirmed by Marcel, October 2026; prefix, `}}`, quote and line-break rules
source-confirmed 2026-10-09; the extra edge cases replayed locally only. Open: the cause of the one canvas
`'\n'` failure is explained by the mechanism but not reproduced on canvas.

---

## Data API step's Filter field: GraphQL literal syntax, and the text is pasted into the query unchecked (2026-10-09)

**Fact, confirmed on canvas (Marcel, October 2026):**
- The Filter field takes **GraphQL literal syntax**: no outer braces, unquoted keys, quoted text. Example:
  `_and:[{specialties:{id:{in:[24]}}},{city:{matches:"Atlanta"}}]`. Enum values work with or without quotes.
- **The filter text is pasted into the query unchecked.** A filter that closes the braces itself can add query
  structure: `id:{eq:1}}) { totalCount } x: allPerson(where:{id:{eq:2}` executed without an error. The
  injected selection did not reach the step output (it returned `null`) but the query was accepted; how far
  an injection can go (other models, mutations) was **not tested**.
- **Never put untrusted text into this field without validating it.** Text from an AI agent or an external
  caller can go straight into the field (no escaping is needed for it to work, unlike the JSON-body case in
  the HTTP entry below), but it still counts as untrusted text.

**Where this lands:** the crash course, wherever it covers the Data API step; also tracked as product feedback outside this repo.

**Status:** confirmed on canvas, October 2026.

---

## HTTP step: Variables in the request body are inserted without JSON escaping, so a `"` in a value breaks the body (2026-10-09)

**Fact, confirmed on canvas (Marcel, October 2026):** a value containing `"` that is inserted into the HTTP
step's JSON body makes the body invalid JSON, and the request is answered with `422 Bad Request` before
anything gets logged. Escape the value in an Expression step first (``JSON.stringify(`{{{v}}}`).slice(1, -1)``,
see the Expression step entry, pattern 5.1) and insert the result.

**Scope:** this applies when an action calls another action through the **HTTP(S) step**. It isn't needed when
the action uses the **Data API request step** directly, which is the simpler route when it's available.

**Also confirmed, same tests:** to call another action through the Data API, POST to
`https://<host>/api/runtime/<app_uuid>` with `Content-Type: application/json` and a body like
`{"query":"mutation { action(id: \"<action id>\", input: $input) }","variables":{"input":{…}}}`. `$input`
did not need a type declaration.

**Where this lands:** the crash course's known-issues section, and any example that builds a request body
from user text.

**Status:** confirmed on canvas, October 2026. Not independently re-run on 2026-10-09; I did not check
which side produced the 422.

---

## A step that makes outbound HTTP calls can be tested end to end against a mock remote app, with `wasmtime-testing-helper`'s `http` feature (2026-10-02)

**Fact, confirmed:** `wasmtime-testing-helper` (the same git dependency the other components use)
has an opt-in `http` feature. With it, a test instantiates the real compiled `.wasm` of a step that
imports `wasi:http/outgoing-handler`, and `harness.mock_http_handler(...)` answers each outgoing
request with whatever the test wants. The handler receives the full request (method, URI, headers,
body), so a test can assert the exact URL, `Authorization` header and JSON body the component
sends, and can return a 200 with a GraphQL `errors` list, a 401, and so on. No network and no
credentials are needed. Enable it with
`wasmtime-testing-helper = { git = "...", features = ["http"] }`; the types are re-exported as
`wasmtime_testing_helper::http::hyper`.

Two details that cost a compile cycle: the generated `call_<function>` methods take `&String`, not
`&str`, for every `string` parameter (and `Option<&String>` for `option<string>`), and the
instantiated component is typed `wasmtime_testing_helper::InstantiatedComponent<bindings::Main>`.

**How found:** porting the six `remote-bb-model` steps (they call another Betty Blocks app's data
API) to `remote-bb-model-wasm-components/`. All 24 end-to-end tests there run the real component
this way, next to 45 native unit tests on the shared logic.

**Related, same project:** six standalone step crates in a disposable test project can share one
pure-Rust `rlib` through a `path` dependency (`remote-bb-common`), so query building and response
parsing exist once. The shared crate's own `workspace = true` fields resolve against its own
workspace, so it also works from the test project. `just build` and `bb functions validate` pass
for all six that way. Whether `bb functions publish` is happy with it is not tested yet.

**Where this lands:** the crash-course section on testing a step, as "how to test a step that
calls out over HTTP", and `testing-a-step-in-a-real-app.md`.

**Status:** confirmed directly, 2026-10-02 (build, native and component tests, `bb functions
validate`). Live canvas, 2026-10-02: Get JWT Token, Get all and Get one then worked against a real remote
app, so the mock-tested request shape matched what a real Betty Blocks data API accepts for reads and for the JWT
action. The write steps (Create, Update, Delete) are not live-tested yet.

---

## Never return an `option<...>` in a step's outputs; it fails at execution with "expected a string, received [object]" (2026-09-29)

**Fact, confirmed live:** a custom step whose output record contained `option<string>` (a Text
output) and `option<u32>` (Number outputs) built, validated, published and could be configured on
canvas. It then failed **when the action executed**, at that step, with `expected a string,
received [object]`. Changing those fields to plain `string`/`u32` fixed it, with nothing else about
the interface changed. The platform receives an `option` as a wrapper object, not as the plain
value an output of that type expects.

**How found:** `dissect-params` (Remote Model Helper Steps port) on a real remote model action in
`genius-sso-test`. Native Stringify accepted the same `params` input fine, so the input was
suspect first. Comparing both compiled components with `wasm-tools component wit` showed the input
declared identically (`params: json-string` vs Stringify's `value: json-string`). A survey of all
24 steps in `bettyblocks/native-wasm-components` and `block-store-wasm-components` showed none
returns an `option` in an output. They use `option` for inputs only (e.g. native HTTP's
`body: option<string>`, liquid-template's `template: option<string>`), and every output record uses
plain types (HTTP: `response-code: u16, as: json-string`). All three option fields were removed in
one change, so which one triggered the error wasn't isolated. "Expected a string" points at the
Text one (`jwt`).

**Practical rule:** `option<...>` is fine for inputs, never for outputs. When an output can
legitimately be absent, return a plain type with an empty value (`""`), or return an error if
absence means misconfiguration. Don't reach for `option`. Add this to the manual function.json/WIT
cross-check before publishing (see the "validate never compares function.json against the WIT"
entry below). `validate`, publish and canvas configuration all accept an option output; only
execution catches it.

**Where this lands:** crash-course section on designing WIT for a step's outputs, next to the
named-`json-string`-alias rule.

**Status:** confirmed directly, first-hand, 2026-09-29 (Dissect Params). Parse as JWT got the same
fix and awaits its own live test.

---

## `bb functions validate` only checks the generic ActionsJS schema and that a `.wasm` exists — it never compares `function.json` against the WIT (2026-09-24)

**Fact, from reading the CLI's source (`@betty-blocks/cli` 26.1.0, `build/functions/validations.js`):** for a WASM project, `validate` (and `publish`'s validation step) does exactly two things per function. It checks `function.json` against the generic ActionsJS JSON schema, fetched from `github.com/bettyblocks/json-schema` (`schemas/actions/function.json`). And it checks that a file ending in `.wasm` exists in the folder. It never opens the `.wasm` or the WIT. So a `function.json` whose option names don't match the WIT's parameter/record-field names, or that uses an option type with no WASM precedent, passes with a green "valid".

**How found:** the Remote Model Helper Steps ports had been passing `validate` all along, while their `function.json` files (carried over from the original ActionsJS versions) had inputs that didn't match the WIT at all. For example, `add-ids` had an option named `array` for a WIT parameter named `records`. Reading the validator's source explained why nothing flagged it. The same source also confirms the symlink entry below: function discovery is Bun's `Glob('**/function.json').scanSync()`, which doesn't follow symlinks.

**Practical rule:** treat a passing `validate` as "the JSON is well-formed", nothing more. Check by hand, before publishing:
- **Names:** every input option's `name` matches a WIT parameter. For a record return, every output option's `name` matches a record field. For a single return, there's exactly one output, conventionally named `result`, as in Betty Blocks' own steps. Our own working steps use the exact kebab-case WIT name. Native HTTP uses camelCase (`responseCode` for `response-code`), which suggests the platform converts between the two, but that's inferred from their files, not tested by us. Exact kebab-case is the proven choice.
- **Types:** stick to option types Betty Blocks' own WASM steps use. For JSON in, that's `Value` with `allowedKinds`, not `Object`. The schema allows `Object` for ActionsJS, but no WASM step uses it. JSON values on either side should use a named `json-string` alias in the WIT (see the named-alias entry below). For numbers, `u32` is the only integer type confirmed working.

**Side note for tests:** once a WIT parameter is typed `json-string` instead of `string`, `wasmtime-testing-helper`'s generated host bindings take `&String` rather than `&str`. Integration tests need `&String::from(...)`, not a bare string literal. This only affects tests. The component behaves the same.

**Where this lands:** crash-course section on writing `function.json`, plus `docs/testing-a-step-in-a-real-app.md`'s build step ("validate passing doesn't mean the manifest matches the component").

**Status:** confirmed directly from the CLI source, 2026-09-24. The name-mismatch failure itself hasn't been observed live (the functions were fixed before publishing).

---

## `bb functions validate`/`publish` silently skip any function whose `function.json` is a symlink — copy `function.json`, symlink only `src/` and `wit/` (2026-09-24)

**Fact:** in a `bb functions init`-scaffolded test project, a function folder whose `function.json` is a **symlink** is invisible to the CLI. `bb functions validate` doesn't list it at all: no error, no warning, and the run still ends with "All your functions are valid and ready to be published!" Replace the symlink with a real file copy and the function shows up and validates immediately. Symlinked `src/` and `wit/` directories are fine, because those are read by `cargo`/`wkg`, which follow symlinks. Only the CLI's own `function.json` discovery ignores them.

**How found:** added 7 more `remote-model-helper-steps-wasm-components` functions (plus re-linking `dissect-params`) to `genius-sso-test` using the documented pattern, with `src/`, `wit/` and `function.json` all symlinked. All 8 built clean with `cargo build --release --target wasm32-wasip2`, but `bb functions validate` (CLI 26.1.0) reported only the 2 pre-existing, non-symlinked SSO functions. Swapping `dissect-params`'s `function.json` symlink for a copy made it appear. Doing the same for the rest made all 10 validate.

**Practical rule:** keep `src/` and `wit/` symlinked so there's one copy of the logic, but **copy** `function.json`. If you edit it in the source repo, re-copy it (or edit both). `docs/testing-a-step-in-a-real-app.md` step 3 has been corrected to match. Rule of thumb: if a function you expect is missing from `bb functions validate`'s list, check for a symlinked `function.json` first.

**Where this lands:** crash-course section on testing a step in a real app. Same correction as the testing guide.

**Status:** confirmed directly, first-hand, 2026-09-24 (`@betty-blocks/cli` 26.1.0). Whether `publish` behaves the same way isn't separately tested. It reuses validate's discovery, so it almost certainly does.

---

## `anyOf`/`schemaModel`-typed `Object` outputs only bind correctly if the WIT field uses a named type alias, not a bare `string` (2026-09-24)

**Fact, confirmed live:** for a custom Wasm step's `Object`-typed output (via `function.json`'s
`"anyOf": [..., {"type": "Object", "schemaModel": "..."}]`, matching the native HTTP step's own
pattern) to actually bind individual fields correctly on canvas (`output.someField` resolving to
that field's real value, not the whole raw string), the underlying WIT field — a record field, or a function's own return
type — must be typed as a **named type alias** (e.g. `type json-string = string; record r { claims: json-string }`),
not a bare, unaliased `string` (`record r { claims: string }`). Both compile to the exact same
`string` at the actual wasm ABI level — the alias makes zero difference to the bytes on the wire —
but the platform apparently reads the component's own embedded WIT type metadata (not just
`function.json`) when deciding how to treat an output field. A bare `string` gets treated as plain
text with no parsing; a field typed via a named alias gets recognized and JSON-parsed for real
field-level binding.

**How this was found:** `inspect-id-token`'s `claims` output kept showing as a raw JSON-text string
on canvas — visible in the log panel, and confirmed for real via a downstream Create-record step
failing validation because schema-bound fields (`claims.name`, `claims.preferred_username`) were
empty. The native HTTP step's own `Response as: Object` binding was confirmed working end-to-end
(built a real record using a bound field from a live HTTP call) as the control. Every other avenue
was ruled out first: `function.json` was verified byte-identical to HTTP's own pattern; the Rust
code was rewritten to literally mirror HTTP's parse/stringify idiom; pretty vs. compact JSON
formatting made no difference; an explicit `is_object()` check confirmed the Rust-side parsing was
already correct. The only thing left unchecked was the WIT file itself — HTTP declares
`type json-string = string;` and uses that alias for its output field; `inspect-id-token` used a
bare `string`. Renaming to match fixed it immediately.

**Practical rule:** when a custom Wasm step declares an `Object`/`Array`-typed output via
`anyOf`/`schemaModel` (the pattern copied from native steps like HTTP), the WIT field backing that
output must use a named type alias, not a bare primitive type — copy the alias, not just the
record/function shape, when replicating this pattern for a new step.

**Status:** confirmed directly, fix verified live, 2026-09-24.

---

## Dragging a Wasm step onto canvas writes to the database — and any DB text column has a 255-character limit that isn't reliably front-end validated (2026-09-23)

**Fact, confirmed by Betty Blocks Product:** adding a step to an action canvas isn't a purely client-side/preview action — it performs a database write (`CreateActionStep`). If any `function.json` text field ends up in a DB column longer than that column's max (255 characters, confirmed for `description`), the write is rejected. This surfaces only as a generic HTTP 500 with no field-specific error — indistinguishable from a real code bug.

**Practical rule:** keep every `function.json` text field (`description`; worth checking `label` and option `info`/`label` text too) under 255 characters, and don't rely on front-end validation to catch a violation — it doesn't always. Check lengths before publishing.

**Status:** confirmed directly, fix verified live, 2026-09-23.

---

## A step that fails "Something went wrong" on canvas despite clean code might just be in the wrong sandbox — check that before debugging code (2026-09-16)

**Fact:** spent several rounds chasing code-level theories for why `decode-and-verify-id-token`
consistently failed to be added to any action canvas — WIT type aliases, `s64` vs `u32`, `anyOf`/
`schemaModel` output shapes, the `base64` and `serde_json` crates individually, even an ampersand
in the step's label. All were cleared one by one via targeted diagnostic probes (the full chain is tracked as product feedback outside this repo). It eventually turned out that a
chunk of that testing (an entire session) was done in a **locked/higher sandbox environment that
doesn't allow editing at all** — which produces this exact same generic "Oops! Something went
wrong — Failed to create action step" symptom for *any* step, including Betty Blocks' own
official, unmodified examples. None of that session's findings were real.

**Why it matters:** this exact symptom is indistinguishable, from the client side, between "this
specific function has a real problem" and "this environment doesn't allow creating action steps
at all right now." Before spending time on code-level diagnosis, confirm you're actually in the
intended editable sandbox — try adding *any* known-good, unrelated step (a native step, or an
official example already known to work elsewhere) as a first sanity check. If even that fails,
the environment is the problem, not the step.

**The actual, confirmed root cause once testing resumed in the correct sandbox:** the same
registration-lineage bug documented for `make-slug`/
`slugify-text` — a specific function *name*/identity stuck in a broken state in a specific app,
unrelated to its code. Fixed by publishing the exact same code under a brand-new function name.

**Where this lands:** crash-course troubleshooting section — "step fails immediately when dropped
on canvas" should list both possibilities (environment-locked vs. registration-lineage) and how
to tell them apart (does *any* step fail, or just this one).

**Status:** confirmed directly, first-hand, 2026-09-16.

**Update, 2026-09-23 — the registration-lineage explanation above was superseded.** The actual
root cause, confirmed by Betty Blocks Product, was a `function.json` `description` field over 255
characters, silently rejected as a database write. It wasn't a stuck function identity at all, and the original claim above that publishing
under a new name fixed it was mistaken: the ~294-character description was carried unchanged
through every rename, and what actually fixed it was shortening the description to 220
characters. The "check you're in an editable sandbox first" advice in this entry still stands on
its own — that part was a real, separate finding.

---

## ~~`decode-and-verify-id-token` crashes — fixed by removing `anyOf`/`schemaModel` and `s64`~~ — WRONG, tested live and did not fix it; real cause is a server-side 500, tracked as product feedback (2026-09-02, corrected same day)

**Fact:** `decode-and-verify-id-token` crashes with a generic "Something went wrong" the moment
it's dragged onto a real action canvas, and disappears again on refresh — despite passing `bb
functions validate`/`publish` cleanly (see the retracted entry above: this was originally
misattributed to `exchange-code-for-tokens`, which is actually fine). Checked every
`meta`/WIT-type pattern this component uses against every confirmed-working custom function in
`block-store-wasm-components` (`generate-random-hex`, `generate-uuid`, `redirect-url`,
`store-file-base64`, `liquid-template`, `format-endpoint-result`, `concat-text`, `split-text`) and
found **two patterns with zero precedent among them, both copied from the native HTTP step**
(`native-wasm-components/functions/http/1.0`) rather than any confirmed custom-function example —
the same mistake class as the `"configuration"`-on-a-Text-option schema-drift bug found earlier
this session:
1. The `claims` output used `"output": {"anyOf": [{"type": "Object", "schemaModel": ...}]}` — no
   confirmed custom function anywhere uses `anyOf`/`schemaModel`. `redirect-url` (confirmed
   working) returns an `Object` output with the plain, unwrapped `"type": "Object"` form instead.
2. The WIT record field `exp` was typed `s64` — no confirmed custom function anywhere uses
   `s64`/`u64` for anything; the only integer type ever confirmed working is `u32`
   (`generate-random-hex`'s `size`, `exchange-code-for-tokens`'s own `expires-in`).

**Change made (compiles/tests/lints clean locally, not yet verified live):** changed `exp` from
`s64` to `u32` in `wit/world.wit` (a JWT Unix-timestamp `exp` claim, currently ~1.8 billion,
comfortably fits — valid to year 2106) and the matching Rust extraction from `.as_i64()` to
`.as_u64()` + `u32::try_from`; simplified `claims`'s `function.json` output to plain
`{"type": "Object"}` and removed the now-unused `schemaModel` input option entirely.

**Update, 2026-09-02 — tested live, republished, dragged onto canvas again: identical failure,
byte-for-byte identical error text.** So this fix was wrong, or at least incomplete — the two
`anyOf`/`schemaModel`/`s64` patterns removed here were real (no confirmed custom-function
precedent for either, worth avoiding regardless), but they were not the cause of this crash. A
browser Network-tab capture (of the "custom Wasm step... failed to create action step" failure) shows the actual failure is a **server-side HTTP 500** from the
`CreateActionStep` GraphQL mutation, with a generic masked message — not something visible from
local validation, and not something either of these two `function.json`/WIT changes could have
addressed. This matches a previously-documented, still-unresolved platform bug class in that same
log entry (`slugify-text`/`make-slug`, 2026-08-10) tied to a function's registration
lineage/identity rather than its actual code.

**Practical implication that does still hold:** `anyOf`/`schemaModel` and `s64`/`u64` remain
patterns with zero confirmed precedent among working custom functions, copied from a native-step
example — worth avoiding on general principle (same reasoning as the `"configuration"`-on-Text
bug), just not the explanation for *this specific* crash. Don't repeat the mistake made here of
treating "no confirmed precedent" as strong enough evidence to declare something fixed without a
live retest — this cost a full round-trip that a "did the error text change at all?" check up
front would have caught immediately.

**Where this lands:** crash-course section on `function.json` option types can still note
`anyOf`/`schemaModel`/`s64` as unprecedented-for-custom-functions and worth avoiding; the actual
platform bug this component hit belongs in product feedback, not here.

**Update, 2026-09-24 — the "avoid `anyOf`/`schemaModel` on general principle" advice above is
superseded.** `inspect-id-token` now ships that exact `anyOf`/`schemaModel` pattern for its
`claims` output, confirmed working live on canvas (see the 2026-09-24 entry, now at the top of
this log, on the WIT named-alias requirement). It's a legitimate, supported pattern — the earlier
"zero confirmed precedent" concern just hadn't been tested through to a working example yet. `s64`
being untested for custom functions is still true and unrelated to this correction.

---

## ~~`exchange-code-for-tokens` crashes when dropped on a real canvas~~ — RETRACTED, wrong component, no bug here (2026-09-02, corrected same day)

**Original (wrong) claim:** believed `exchange-code-for-tokens` was the component crashing with
"Something went wrong" when dragged onto a real action canvas, and diagnosed it as a
`wasi:http/outgoing-handler` version mismatch (`@0.2.6` vs. the only confirmed-working custom-
function precedent, `store-file-base64`'s `@0.2.0`). Changed the version, confirmed it still
compiles/tests/lints clean at `@0.2.0` — all before ever getting live confirmation.

**Correction, same day:** Marcel corrected this — `exchange-code-for-tokens` was already working
fine on canvas. **`decode-and-verify-id-token` is the one that actually crashes** when dropped on
a real action canvas. The `@0.2.0` change was reverted (back to `@0.2.6`, confirmed via `git
restore` against the still-staged pre-change version, rebuilt in both `wasm-blocks-store` and
`genius-sso-test`). Since `decode-and-verify-id-token` has **zero WASI imports** (pure
`base64`+`serde_json` compute), the whole `wasi:http` version theory doesn't even apply to it —
this needs fresh investigation, not a variant of this one.

**Why the original diagnosis felt plausible anyway:** it was built on a real, still-true
observation (no confirmed precedent of any custom function calling `wasi:http` itself, plus the documented precedent for silent link failures) — the reasoning wasn't
unfounded, it was just applied to the wrong component. Worth remembering: a well-evidenced
hypothesis about *a* problem doesn't confirm you have the *right* problem — should have confirmed
which component was actually failing before investigating why.

**Status:** retracted 2026-09-02. See below for the actual `decode-and-verify-id-token`
investigation once it exists.

---

## A step's secret-like inputs (client secrets, tokens) don't need function.json-level masking — Betty Blocks' platform Configuration feature already handles that (2026-09-01)

**Fact:** there's no need to try to mask a secret-like option (e.g. an OAuth `client_secret`) at
the `function.json`/step level — Betty Blocks' platform has its own "Configuration" feature that
serves values like a token URL, client ID, or client secret into a flow, with masking handled
there. A step just needs a plain option (e.g. `meta.type: "Text"`) for these — the flow builder
binds it to a Configuration value on canvas, and masking is the platform feature's job, not
something the step's own schema needs to attempt.

**Why it matters:** this is exactly why the `"configuration": {"placeholder": "******"}` cosmetic
masking attempt on `exchange-code-for-tokens`'s `client-secret` option (added, then removed, this
session) was solving a non-problem. It was unnecessary in the first place. (It was also believed
to be invalid against the live publish schema, but that theory was later contradicted; see the
2026-09-24 update on the schema-drift entry below.) Don't reach for
step-level masking tricks for secret-like inputs; a plain option is correct and the real masking
happens one layer up, in the platform's Configuration feature.

**Where this lands:** crash-course section on designing `function.json` options for
credentials/secrets — plain option, no masking attempt, point at the platform's Configuration
feature for how the actual value gets supplied and protected.

**Status:** relayed directly by Marcel Korporaal (Betty Blocks Solution Engineer) from his own
platform knowledge, 2026-09-01 — not independently re-verified against the Configuration feature's
UI/docs in this session, but from the primary domain expert on this project, not secondhand.

---

## `bb functions init --type wasm` scaffolds a flat, standalone-per-function project — not a workspace like this repo (2026-09-01)

**Fact:** running `bb functions init <identifier> --type wasm` creates a new directory with a
`.wasm-functions` marker and a sample `functions/say-hello/1.0/` folder, but there is **no root
`Cargo.toml`/`Justfile`/`build.rs`** — each function is its own independent Cargo project (its
own `Cargo.toml` with literal dependency versions, its own `Cargo.lock`, and its own per-function
`Justfile` with `fetch_wit_deps`/`build`/`move_wasm_to_root`/`test` targets). This is structurally
different from `wasm-blocks-store`'s (and `block-store-wasm-components`'s) shared-workspace
convention — one root `[workspace]` `Cargo.toml` with pinned `workspace.dependencies`, one root
`Justfile`/`build.rs` driving every function together.

**Why it matters:** moving a step from this repo into a `bb functions init`-created test-app
project for live canvas testing isn't a straight `cp -r` — each function needs its own
standalone `Cargo.toml` (spelling out literal versions instead of `dep.workspace = true`) and its
own `Justfile` written to match that flat convention. Symlinking just `src/`, `wit/`, and
`function.json` from this repo into the test-app project (and writing fresh, throwaway
`Cargo.toml`/`Justfile` files there) avoids maintaining two copies of the actual step logic while
still letting `bb functions publish` build and ship it from the test app.

**Where this lands:** crash-course section on testing a step in a real app before opening a PR —
worth a short note + the symlink approach, since "move the code over" is a real step in the
workflow, not something CONTRIBUTING.md's checklist currently spells out mechanically.

**Status:** confirmed directly, first-hand, 2026-09-01 — scaffolded a test project, then set up
`decode-and-verify-id-token` and `exchange-code-for-tokens` in it via symlinks + standalone
`Cargo.toml`/`Justfile`, and confirmed both build (`wasm32-wasip2` release) and pass their unit
tests there, identically to how they build/test inside `wasm-blocks-store`.

---

## A component can pin its own `wit-bindgen` version (overriding the workspace default) to use `waki` for real outbound HTTP from inside a component, in this same Cargo/Justfile build stack (2026-09-01)

**Fact:** `wasm-blocks-store`'s workspace pins `wit-bindgen = "0.58.0"` for every function by default. Building `exchange-code-for-tokens` (a component that performs its own outbound HTTPS POST, mirroring `native-wasm-components`' own HTTP step) required the older `wit-bindgen = "0.42.0"` + `waki = "0.5.1"` + `wasi = "0.14.2"` combination instead — `waki`'s HTTP client is built against the official `wasi` crate's own generated `wasi:http` bindings, and this crate's own `wit_bindgen::generate!` has to reuse those exact same types via a `with:` remap (`"wasi:http/outgoing-handler@0.2.6": ::wasi::http::outgoing_handler`, etc. — copied verbatim from `native-wasm-components/functions/http/1.0/src/lib.rs`) rather than generating a second, incompatible copy of those types. Simply declaring `wit-bindgen = "0.42.0"` directly in that one function's `Cargo.toml` (instead of `wit-bindgen.workspace = true`) was enough — Cargo is fine resolving two different versions of `wit-bindgen` across different workspace members, since it's a proc-macro/codegen dependency, not something that needs a single ABI-compatible version workspace-wide. `cargo build --release --target wasm32-wasip2`, the native `cargo test` (unit tests), and the wasmtime-based component integration test (`tests/mod.rs`, instantiating the real compiled `.wasm` and calling it through its WIT interface) all passed cleanly with no version-skew errors. `cargo clippy --all-targets -- -D warnings` and `cargo fmt --check` were also clean.

**Why it matters:** the crash-course/skill previously treated `native-wasm-components` as "different build stack (Elixir/mix), you can't build against it" — true for that repo itself, but this confirms the *pattern* it uses (direct `waki`/`wasi:http` calls from inside a component, rather than delegating to a host-provided capability like `store-file-base64` does for uploads) is fully portable into this repo's own Cargo/Justfile stack, on a per-function basis, without touching the workspace's shared `wit-bindgen` pin for every other function.

**Where this lands:** crash-course section on making outbound HTTP calls from a component — as an alternative to the native HTTP step, when a dedicated step needs to perform its own request (e.g. to spare the flow builder from hand-building a correctly-encoded body/headers). Should note the exact version trio and the `with:` remap requirement, since getting either wrong would likely fail at compile time or (worse) produce a component that compiles but panics/traps at the `wasi:http` boundary.

**Status:** confirmed directly, first-hand, 2026-09-01, building `exchange-code-for-tokens` in `wasm-blocks-store`.

---

## `bettyblocks/fusionauth_jwt_rs` proves pure-Rust JWT (RS256, JWKS-based) verification is buildable for `wasm32-wasip2`, but only with `jsonwebtoken`'s `rust_crypto` backend (2026-09-01)

**Fact:** `jsonwebtoken`'s default feature set pulls in `ring`/`aws-lc-rs` (C/assembly, OS-RNG-dependent), which do not compile for `wasm32-wasip2`. `bettyblocks/fusionauth_jwt_rs` is a real, CI-tested-for-wasm32-wasip2 repo that uses `jsonwebtoken` with `default-features = false, features = ["rust_crypto"]` instead — a pure-Rust backend — and does full RS256 JWKS-based signature verification (`iss`/`aud`/`exp` validation, `kid`-matched key selection) plus its own JWKS fetch via `wasi:http/outgoing-handler`. Confirmed by reading that repo's `Cargo.toml`/README directly (not secondhand), not by building it ourselves in this repo.

**Why it matters:** not needed for `decode-and-verify-id-token` (built 2026-09-01) — that component deliberately skips cryptographic signature verification, only checking `iss`/`aud`/`exp` claims, per an explicit product decision that this is a sufficient bar for now. But if a future step ever needs real signature verification (e.g. accepting an `id_token` from a source that isn't a direct, TLS'd call to the provider's own token endpoint), this is the proven starting point — don't reach for plain `jsonwebtoken` with default features, it won't compile for this target.

**Where this lands:** crash-course section on cryptography/JWT handling in components, as a "if you ever need this" pointer — not urgent to write up in detail until a step actually needs it.

**Status:** confirmed by direct inspection of the `fusionauth_jwt_rs` repo's own `Cargo.toml`/README, 2026-09-01. Not independently built/tested in this repo.

---

## `wasco-dev/openid-connect-api`'s checked-in `wit/world.wit` understates its real import surface (2026-09-01)

**Fact:** the checked-in `wit/world.wit` for `wasco-dev/openid-connect-api` (package `wasco-dev:open-id-connect@1.0.1`) declares `world main { export oidc-client; }` with no imports at all, which reads as "zero imports, trivially portable" per this repo's own `docs/wasco-dev.md` heuristic. But the actual published artifact (`ghcr.io/wasco-dev/open-id-connect:1.0.1`, pulled and inspected directly via `wash`) imports `wasi:http/outgoing-handler`/`wasi:http/types` plus the full `wasi:cli` set, `wasi:clocks/monotonic-clock`, `wasi:io/*`, and `wasi:random/insecure-seed` — because its `src/client.rs` performs its own HTTP calls via the `wstd` crate. The checked-in WIT source and the actually-compiled/published component disagree.

**Why it matters:** anyone relying on `docs/wasco-dev.md`'s "check its WIT imports" guidance by reading the repo's checked-in `wit/world.wit` file alone (rather than pulling and inspecting the real published `.wasm`) would wrongly conclude this component is zero-import/portable. It does need the same `wasi:http` capability any HTTP-calling component needs — not a blocker for Betty Blocks (which already supports `wasi:http/outgoing-handler`), but worth knowing before assuming "no imports" from the source alone.

**Where this lands:** `docs/wasco-dev.md`'s "Is it actually portable? Check before assuming" section — add a note that the checked-in WIT source isn't guaranteed to match the published artifact's real import surface for every wasco-dev component; when in doubt, pull and inspect the actual `.wasm`.

**Status:** confirmed by directly pulling and inspecting the published `ghcr.io/wasco-dev/open-id-connect:1.0.1` artifact, 2026-09-01. Worth a heads-up to Chris Obdam at some point; not blocking anything in this repo since `wasm-blocks-store` didn't end up depending on this component (see the OpenID Connect SSO build plan).

---

## `bb functions init`/`publish` have no way to target an existing folder or app ad hoc — confirming this is a CLI limitation, not a repo-shape one (2026-09-01)

**Fact:** two things confirmed directly while working out how to test a step from this repo on a
real app (see `docs/testing-a-step-in-a-real-app.md` for the full write-up):

- `bb functions init <identifier> --type wasm` always scaffolds a **brand-new subdirectory** named
  after `<identifier>` — running it in an empty scratch directory created `./<identifier>/` as a
  new child directory, not by initializing the current directory in place. There is no flag or
  mode to point `init` at an already-existing `functions/` folder instead of scaffolding a fresh
  one.
- `bb functions publish --help` has no `--app`/`--identifier` override flag — only
  `--skip-compile` and `-a`/`--all`. There's no way to tell `publish` "target this specific app"
  ad hoc at the command line either.

Meanwhile, `bb functions validate`/`bb functions publish` already run fine directly against this
repo's existing workspace-shaped `functions/` folder with zero restructuring — validating both of
two newly-built functions here succeeded even though this repo was never `bb functions init`'d for
that path. So this isn't "the shared-catalog repo's shape doesn't match what the CLI expects" —
the repo's shape was never the problem.

**Why it matters:** the app-binding the CLI needs is established once, via `init`, which only ever
creates a fresh project directory — it's a "one directory = one app, forever" design. A shared
catalog repo meant to be cloned by many developers targeting many different apps can't satisfy
that 1:1 relationship by rearranging its own folders; the mismatch is conceptual, not structural.
The working alternative is a separate, disposable `init`'d test project that symlinks the real
`src/`/`wit/`/`function.json` back from this repo — see `docs/testing-a-step-in-a-real-app.md`.

**Where this lands:** already written up as its own doc (`docs/testing-a-step-in-a-real-app.md`),
linked from `CONTRIBUTING.md`'s checklist. Worth folding into the crash-course's publishing
section too at some point.

**Status:** confirmed directly, first-hand, 2026-09-01.

---

## Two gotchas hit publishing from a freshly-`init`'d test project, one confirmed, one still a working theory (2026-09-01)

**Fact 1 (confirmed) — the scaffolded `say-hello` sample blocks publish of everything until it's
dealt with.** `bb functions init --type wasm` ships `functions/say-hello/1.0/` with source but no
compiled `.wasm`. Leaving it in place makes `bb functions publish` fail validation for the whole
project, not just the sample: `Error: Missing .wasm file in .../functions/say-hello/1.0`. Delete
the sample folder, or `just build` it, right after `init` — before symlinking any real code in.

**Fact 2 (working theory, not independently confirmed) — the live publish endpoint's schema is
stricter than the CLI's own local `bb functions validate`/`publish` schema check.** An option
shaped like `{"meta": {"type": "Text", ...}, "configuration": {"placeholder": "******"}, ...}`
passed `bb functions validate` locally, but the live endpoint rejected it: `Error: 400,
#/options/2/meta: Expected exactly one of the schemata to match, but none of them did.` Every
confirmed-working example in `block-store-wasm-components` (`liquid-template`,
`store-file-base64`) only ever pairs `"configuration"` with `meta.type: "MultilineText"` or
`"Property"`, never `"Text"` — so the likely explanation is that `"configuration"` simply isn't
valid for a `"Text"`-typed option server-side, even though the CLI's bundled local schema doesn't
catch that. **This has not been independently re-verified against the live schema's own source —
treat it as a working theory, not a confirmed root cause, until someone checks that removing
`"configuration"` actually resolves the `400` on a real retry.**

**Update, 2026-09-24 — the `"configuration"`-on-`Text` theory is contradicted; treat Fact 2's
explanation as wrong.** Betty Blocks' own shipped steps pair `"configuration"` with `"Text"`:
native HTTP's `url` option is `meta.type: "Text"` + `"configuration": {"placeholder": ...}`, and
native `send-mail` has `"placeholder": "******"` on its password option (`Value` type). The
"never with Text" observation above only held because it looked at `block-store-wasm-components`
alone, not `native-wasm-components`. The error message itself also points at `#/options/2/meta`,
not at `configuration`, so something inside that option's `meta` failed the live schema. The full
original `meta` wasn't recorded, so the real cause can't be recovered now. What does still stand:
local `validate` passed something the live endpoint rejected, so the two schemas really do differ
in *something*. Found while auditing the Remote Model Helper ports, whose `"placeholder": "params"`
hints had been removed on the strength of this theory and have now been restored.

Separately, and independent of whether the above theory holds: a secret-like option (a client
secret, a token) doesn't need step-level masking in the first place. Per Marcel Korporaal (Betty
Blocks Solution Engineer, relayed from his own platform knowledge, not independently re-verified
against the Configuration feature's own UI/docs in this session) — Betty Blocks' platform has its
own Configuration feature for supplying values like a client ID/secret into a flow, with masking
handled there. A step just needs a plain option; there's no need to reach for `function.json`-level
masking tricks like the `"configuration"`/`placeholder` attempt above.

**Where this lands:** both folded into `docs/testing-a-step-in-a-real-app.md`'s Troubleshooting
section. Fact 2 is arguably also worth a product feedback entry (a client-side `validate`
silently accepting something the live server rejects is a real tooling gap) — not added yet since
it isn't independently confirmed; add it once someone verifies the retry.

**Status:** Fact 1 confirmed directly, first-hand, 2026-09-01. Fact 2 (both the schema theory and
the Configuration-feature explanation) not independently confirmed — flagged as open, per this
repo's own discipline around not stating unconfirmed claims as fact.

---

## A plain `cargo build`/`cargo test` (no `--target`) cannot link a WIT component export — build for `wasm32-wasip2` first (2026-08-14)

**Fact:** a crate using `wit_bindgen::generate!` + `export!` only produces symbols the linker can actually resolve when compiled for the `wasm32-wasip2` target. Running plain `cargo build --workspace` (native target, no `--target` flag) on a fresh multi-function workspace fails every single crate at the link step with `Undefined symbols ... "_betty-blocks:<pkg>/<pkg>@1.0.0"` / `ld: symbol(s) not found`. This isn't a code bug in the Rust — the exported WIT function's symbol only exists once the crate is actually compiled as a wasm component. The fix is simply to build with `cargo build --release --target wasm32-wasip2` (exactly what `block-store-wasm-components`'s `Justfile` `build` step already does) before running `cargo test` — the native `cargo test` binary itself links fine (it doesn't need the cdylib's export table), it's only a bare native `cargo build`/`cargo check` of the library crate that trips over this.

**Practical implication:** don't reach for a quick `cargo build` (no target) to sanity-check a new WIT component crate — it will always fail this way regardless of whether the Rust logic is correct, and the error looks like a real linking problem rather than "wrong target." Always build wasm32-wasip2 first, or just run `cargo test` directly (which works without a prior native build).

**Where this lands:** crash-course quick-start / troubleshooting section — "if a fresh WASM component crate fails to `cargo build` with `Undefined symbols` for its own exported WIT function name, that's expected for a native-target build; build `--target wasm32-wasip2` (or just run `cargo test`) instead."

**Status:** confirmed directly, first-hand, 2026-08-14, while scaffolding a new 8-function component workspace.

---

## `serde_json::Map` serializes object keys alphabetically, not in insertion order, unless the `preserve_order` feature is enabled (2026-08-14)

**Fact:** by default `serde_json::Map<String, Value>` is backed by a `BTreeMap`, so `serde_json::to_string(&value)` on any JSON object always emits its keys in alphabetical order — regardless of the order fields were inserted or the order they appeared in the original input JSON. E.g. inserting an `"id"` field into `{"userID": 42, "name": "a"}` serializes as `{"id":42,"name":"a","userID":42}`, not `{"userID":42,"name":"a","id":42}`. This only matters when a test asserts an exact JSON *string*; parsing the result back into a `Value` and comparing field-by-field (or comparing parsed values/maps) is unaffected and is the more robust way to test JSON-string-returning WIT functions in general.

**Where this lands:** crash-course section on writing tests for JSON-string-returning components — recommend asserting on parsed values rather than exact serialized strings, or explicitly note the alphabetical-key behavior if a literal string comparison is used.

**Status:** confirmed directly, first-hand, 2026-08-14.

---

## `bb functions publish` (no flags) shows an interactive checklist of which functions to publish — corrects an earlier entry (2026-08-12)

**Fact:** running plain `bb functions publish` in a project with more than one function does **not** silently publish everything — it validates every function folder first, then presents an interactive multi-select prompt (`? Which wasm functions do you want to publish? ›`, arrow keys + spacebar to toggle each one, Enter to confirm) before publishing only the ones selected. Confirmed directly from Marcel's own terminal, with both `generate-random-hex` and `generate-uuid` present in the project. Passing `-a`/`--all` skips this prompt and publishes every valid function without asking.

**This corrects earlier entries below** (the "second, real WASM publish path" entry and its 2026-08-10 update) which described bare `bb functions publish` as publishing every function in one pass with no selection step — that was based on an earlier real run in `marcel-tes` that, for reasons not yet understood (a different CLI version? a non-interactive shell context?), didn't surface this picker. Treat the interactive-checklist behavior as the current, confirmed default; the earlier "publishes everything automatically" claim is superseded.

**Also confirmed in the same session:** the CLI resolves the project's app *identifier* (the human-readable subdomain given at `bb functions init`/tied to the project folder) into the application's own internal UUID as a distinct step, before the login check. The identifier and the UUID are different things — the identifier is just a friendly name, the UUID is what Betty Blocks' backend actually keys the application on internally. Nothing about typing or supplying that UUID yourself is required; it's resolved automatically from the identifier.

**Where this lands:** `WASM_Actions_Crash_Course.docx` section 3.9 — already updated to describe the validate → choose-what-to-publish → resolve-app → log-in → publish sequence, with the real prompt shown verbatim and `--all` explained as the way to skip the picker.

**Status:** confirmed directly, first-hand, 2026-08-12.

---

## ROOT CAUSE FOUND (via BB product team): the indefinite `store-file-base64` compile hang is a host-capability WIT version skew — `upload-file`'s `input.file-bytes: list<u8>` became `input.file-base64: string` (2026-08-12)

**Fact:** the `store-file-base64` example (identical, byte-for-byte confirmed, in both `block-store-wasm-components-main` and `marcel-tes`) was compiled against an **unversioned** `betty-blocks-utilities:upload-file` interface whose `upload` function takes `input.file-bytes: list<u8>` (raw decoded bytes). Betty Blocks has since moved the real host-capability-provider implementation (`wasm-base-components` repo) to a renamed, versioned interface, **`betty-blocks-types:upload-file@3.0.0`**, whose `input` record now has **`file-base64: string`** instead — the host itself decodes the base64 now, specifically because passing raw bytes as WIT `list<u8>` costs ~48 host bytes per file byte at the component boundary and traps around 2.67 MiB (per the new interface's own comment). The dependent record types were also renamed: `model`/`property` → `betty-model`/`betty-property`, and `data-api`/`types` got version-pinned to `@2.0.0` (though those two kept identical field shapes — only `upload-file`'s payload actually changed structurally).

**Why it manifests as an indefinite hang, not a clean error:** BB's product team found this wasmCloud host log line when a `store-file-base64` step with real values is saved/run:
```
failed to link components, unbinding all plugins error=failed to pre-instantiate during component linking
Caused by:
  0: component imports instance `betty-blocks-utilities:upload-file/upload-file`, but a matching implementation was not found in the linker
  1: instance export `upload` has the wrong type
  2: function implementation is missing
```
The compiled guest component's import (old interface identity + old `input` shape) no longer matches what the host actually exports. This is a **component-linking failure** at wasmCloud instantiation time, and whatever step in the Actions Compiler triggers/health-checks that instantiation apparently doesn't surface a link error cleanly to the UI — it just spins ("Actions compiling..." forever), matching the same `actions_compiler#internal_error: Not yet healthy` symptom logged elsewhere in this project. Confirms that symptom's underlying cause, at least for this trigger: it's not "unhealthy," it's a silently-swallowed link failure.

**Practical implication — this generalizes beyond `store-file-base64`:** any published third-party/Block Store WASM component that imports a `betty-blocks-utilities:*` host capability (not just `upload-file` — also `data-api`, and anything downstream depending on how broadly BB has rolled out the `betty-blocks-types` rename) is a candidate for the same class of link failure the moment the host-side interface it targets has been migrated. `data-api`'s and `types`' record shapes happen to be unchanged field-for-field in the new versioned interfaces, but the **package identity itself changed** (unversioned `betty-blocks-utilities:X` → versioned `betty-blocks-types:X@N.0.0`), and WIT component linking matches on interface identity, not just structural shape — so even a "cosmetic" rename could in principle break linking for any old-style component, independent of whether its record fields still line up. Worth explicitly asking BB whether the host keeps *any* compatibility shim for `betty-blocks-utilities:*` names, or whether every existing published component using those old imports is now at risk.

**The actual code fix needed** (blocked until BB confirms the new host interface is live/stable): in `store-file-base64`'s `src/lib.rs`, remove the `BASE64.decode(&data)` step entirely and pass the raw base64 `data` string straight through as `file_base64` instead of `file_bytes`; re-vendor `wit/deps` from the new `betty-blocks-types:upload-file@3.0.0` / `data-api@2.0.0` / `types@2.0.0` packages (update `wkg.toml` overrides, rerun `wkg wit fetch`); update binding import paths (`betty_blocks_types::...`, `BettyModel`/`BettyProperty` type names); rebuild for a genuinely new checksum; republish. Per BB product's own message, they believe `block-store-wasm-components`'s example itself needs this same update — i.e., this isn't marcel-tes-specific, the canonical example is currently stale against BB's own host.

**Where this lands:** `WASM_Actions_Crash_Course.docx` troubleshooting section — "an indefinite 'Actions compiling...' hang after saving a step with real values can mean the component was built against a stale host-capability WIT version; check whether BB has re-versioned the interface you're importing." Also worth a callout that host-capability interfaces are versioned and can move out from under a previously-working component with no rebuild trigger/warning from the CLI.

**Status:** root cause confirmed via Betty Blocks' own product team + independently verified by diffing the vendored `upload-file.wit`/`types.wit`/`data-api.wit` against the current `bettyblocks/wasm-base-components` repo. Fix is blocked on BB confirming the new interface is actually deployed/stable to build against.

**Update, 2026-08-12 — fix implemented and locally validated in `marcel-tes`; live-app test still pending.** BB product confirmed the exact required change: `import betty-blocks-utilities:upload-file/upload-file;` → `import betty-blocks-types:upload-file/upload-file@3.0.0;`. Applied to `marcel-tes/functions/store-file-base64/1.0`:
- Vendored the new interfaces as `external/{types-v2,data-api-v2,upload-file-v3}.wit` (copied verbatim from `bettyblocks/wasm-base-components`'s current `wit/{types,data-api,upload-file}/*.wit`) and added matching `wkg.toml` overrides (`betty-blocks-types:types`, `betty-blocks-types:data-api`, `betty-blocks-types:upload-file`) alongside the pre-existing `betty-blocks-utilities:*` ones — kept, since our own exported `store-base64` interface's `store-file` function signature (what the platform itself calls into) still uses the old, unversioned `betty-blocks-utilities:data-api`/`types`, and only the internal `upload-file` host-capability import needed to move.
- `wit/world.wit`: changed only the one import line, exactly as product specified.
- `src/lib.rs`: removed the `BASE64.decode(&data)` step entirely (the host now decodes base64 itself — see the new interface's own comment about `list<u8>` costing 48 host bytes/file byte and trapping at ~2.67 MiB) and pass `data` straight through as `input.file_base64`. Since `betty-blocks-types:upload-file@3.0.0` depends on `betty-blocks-types:data-api@2.0.0`/`types@2.0.0` — different WIT package identities than our own function's `betty-blocks-utilities:*`-typed `helper_context`/`model`/`property` params, even though every record has identical fields — added a manual field-by-field conversion into the new types (`BettyModel`/`BettyProperty`/a second `HelperContext`) before calling `upload_file::upload`. wit-bindgen does not consider these interchangeable even though they're structurally identical; this is not a one-line fix for that reason.
- `tests/mod.rs`: updated the mock target string to `"betty-blocks-types:upload-file/upload-file@3.0.0"` (mock keys must match the exact versioned import string from `world.wit`) and its closure's `HelperContext` type to the new package's version. Deleted `fails_with_invalid_base64` — base64 validation is no longer this component's responsibility now that the host decodes it, so the assertion no longer holds.
- **Separately discovered and fixed while doing this:** `store-file-base64/1.0/Cargo.toml` had never actually been adapted for standalone use in `marcel-tes` — it still had `edition.workspace = true` and `build = "../../../build.rs"`, both copied verbatim from `block-store-wasm-components-main`'s *workspace*-based root, which doesn't exist in `marcel-tes` (no root `Cargo.toml`/`[workspace]`, unlike `generate-uuid`'s already-correct standalone style in the same repo). This was latent and invisible until now because the `.wasm` here was always just byte-copied in from the workspace repo, never actually compiled from within `marcel-tes` itself. Fixed by inlining concrete values (`edition = "2024"`, resolved dependency versions from the workspace repo's `Cargo.toml`) and adding a local `build.rs` copy (was previously only reachable via the missing `../../../build.rs`). Also dropped the `base64` dependency (no longer used) and an unused `proptest` dev-dependency that was never actually invoked by this crate's tests.
- **Verified:** `wkg wit fetch` resolved all three new overrides cleanly into `wit/deps/`; `cargo build --release --target wasm32-wasip2` succeeded; `cargo test` passed all 9 tests, including the composed-component harness test that mocks the new `betty-blocks-types:upload-file/upload-file@3.0.0` import — i.e., the WIT-level wiring genuinely links correctly in a real wasmtime component composition, not just "compiles." The rebuilt `.wasm` has a new SHA-1 (`63384917...`, was `6a75ce73...`) and is smaller (95,353 vs 100,922 bytes, consistent with dropping the `base64` crate) — confirmed a genuinely new artifact, not a relabeled one (per the "renaming isn't rebuilding" entry above). Copied into place at `functions/store-file-base64/1.0/store_file_base64.wasm`.

**RESOLVED, 2026-08-12 — confirmed live end-to-end.** Marcel ran `bb functions publish` against `marcel-tes` and re-tested the step with real values wired in (MODEL=`Webuser`, PROPERTY=`file`, real base64 data). Result: **the action compiled successfully (no more indefinite "Actions compiling..." hang) and a real file was saved through the upload flow.** This confirms both halves of the diagnosis: (1) BB's `betty-blocks-types:upload-file@3.0.0` host implementation is genuinely live and reachable, and (2) the fix above — the single import-line change plus the resulting `file_base64`/type-conversion/Cargo.toml work — is complete and correct, not just locally plausible. No further action needed on `store-file-base64` in `marcel-tes`. Whether `block-store-wasm-components`'s own canonical example still needs the same update (per BB product's earlier comment that it does) remains open, but is now a known, mechanical fix rather than an open investigation — this same diff is the template if/when that repo needs it.

**Where this lands:** same crash-course troubleshooting section as above, plus a new callout for `WASM_Actions_Crash_Course.docx`'s Cargo.toml/project-setup section: when duplicating a component from a workspace-based reference repo into a standalone `bb functions init` project, workspace-relative fields (`*.workspace = true`, `build = "../../../build.rs"`) must be resolved to concrete values — they fail silently as "never actually tried to build" rather than an obvious error, until someone attempts a real local rebuild.

---

## Renaming/re-versioning a WIT project's source files does NOT change the compiled `.wasm` at all — you must actually rebuild (2026-08-10)

**Fact:** editing a component's folder name, `Cargo.toml`, and `wit/world.wit` (package/interface/world names, version strings) has **zero effect** on an already-compiled `.wasm` binary. `bb functions publish` uploads whatever file is physically sitting in the function's folder — it does not rebuild anything, and it doesn't complain if the binary's own embedded WIT identity doesn't match the folder name or `function.json`. If you rename/re-version a component to "get a fresh start" without running `cargo build --release --target wasm32-wasip2` again, you are republishing **the exact same bytes** under a new label — indistinguishable to the backend from the original artifact, which defeated several rounds of debugging before this was caught.

**Also confirmed while debugging this:** the WIT **world**'s own name is apparently not embedded in the compiled component's metadata at all — renaming only the `world` block (keeping the same interface/package names) and rebuilding produced a **byte-for-byte identical** binary (verified via `git hash-object`). Only changes to the package name, interface name, or the actual function signatures (params/return type) produce a genuinely different compiled artifact. If you need to prove "this is a fresh binary" for diagnostic purposes, changing the world's name alone doesn't count — check the checksum.

**Practical check:** after any rename/version-bump/rebuild intended to produce a "fresh" artifact, verify with `git hash-object path/to/component.wasm` before and after — if the hash didn't change, nothing actually changed from the platform's point of view, no matter how many labels were edited.

**Where this lands:** `WASM_Actions_Crash_Course.docx` — add to the troubleshooting/quick-start section: "a version bump or rename is not a rebuild; if you're trying to recover from a broken registration, confirm the `.wasm` checksum actually changed."

**Status:** confirmed directly, first-hand, 2026-08-10.

---

## The local machine already has a working Rust + `wasm32-wasip2` toolchain — just not on `PATH` (2026-08-10)

**Fact:** `rustup`/`cargo`/`rustc` (1.97.1) and the `wasm32-wasip2` target are already installed at `~/.cargo/bin` on this machine (from an earlier session), but that directory is not on the default shell `PATH`, so plain `cargo build ...` reports "command not found" as if no toolchain were installed at all. Use the full path or `export PATH="$HOME/.cargo/bin:$PATH"` first. `cargo build --release --target wasm32-wasip2` alone produces a real, directly-usable component (`\0asm` + version `0d 00 01 00`) — no separate `wasm-tools component new` step needed with `wit-bindgen` 0.58.0's `generate_all`.

**Where this lands:** quick-start checklist (section 10) — check for `~/.cargo/bin` on this machine specifically before concluding a rebuild isn't possible.

**Status:** confirmed directly, first-hand, 2026-08-10.

---

## `function.json` `description` has a hard 500-character limit (2026-08-07)

**Fact:** the `description` field in `function.json` is capped at 500 characters. Exceeding it causes a problem at upload/registration time (exact failure mode to be pinned down — see open question below).

**How found:** discovered by Marcel during hands-on testing (source of the finding predates this log; recorded retroactively when the log was created).

**Open question to confirm before documenting as fact:** what exactly happens past 500 characters — hard validation error at upload, silent truncation, or something else? Worth confirming precisely so the docs can tell developers what to expect, not just that a limit exists.

**Where this lands:** `WASM_Actions_Crash_Course.docx`, section 4.4 (`function.json` option types reference) or a new small "field limits" note near the component anatomy section (section 4).

**Status:** confirmed limit exists; failure-mode detail still open.

**Update, 2026-09-23 — the open question is answered, and the number was wrong.** The actual
limit is **255 characters**, not 500, confirmed directly by Betty Blocks Product. Exceeding it
rejects the `CreateActionStep` database write outright (generic HTTP 500, no field-specific
error) rather than truncating silently.

---

## `wkg wit fetch` must run before `wit-bindgen` can see a local `external/*.wit` override (2026-08-07)

**Fact:** if your component imports a host interface via a local vendored file (a `wkg.toml` `[overrides]` entry pointing at `./external/*.wit`, the pattern `store-file-base64` uses), you must actually run `wkg wit fetch` (or have a `build.rs` that does it, like `block-store-wasm-components`'s does) before `cargo build`/`cargo test` — `wit_bindgen::generate!` only resolves packages already materialized under `wit/deps/`, it does not read `wkg.toml`/`external/` directly. Skipping this step produces `error: package 'betty-blocks-utilities:data-api' not found`, which reads like a WIT syntax mistake rather than a missing build step, and cost real debugging time while building the `create-record` learning exercise.

**How found:** hit directly while scaffolding `create-record/` from scratch — no `build.rs` was written for it (unlike the workspace-style repos), so this had to be run by hand.

**Also found:** `wkg` 0.16.0 prints `warning: 'wkg wit <command>' is deprecated, use 'wkg <command>' instead` for `wkg wit fetch` — cosmetic today, but `block-store-wasm-components`'s `build.rs` still calls the deprecated form (`wkg wit fetch`), so this is worth a heads-up if/when that tooling gets bumped to a newer `wkg`.

**Where this lands:** `WASM_Actions_Crash_Course.docx` section 5 (build & test tooling) and/or the quick-start checklist (section 10) — call out explicitly that a component without a shared `build.rs` needs `wkg wit fetch` run manually as its own step before the first build.

**Status:** confirmed directly, first-hand, while building `create-record/`.

---

## WASM steps are not (yet) uploadable from the general Block Store page — the entry point is inside an app's WASM action (2026-08-07)

**Fact:** Betty Blocks currently runs two parallel action systems, ActionJS and WASM. The Block Store's own UI (`my.bettyblocks.com/block-store`, Functions tab) only lets you *browse and edit existing* blocks (`My Blocks` → `Dev blocks`/`Released blocks`) — there is no "create/add new function block" button there. To upload a new custom WASM component for the first time, you open a WASM action inside an actual application's Action Builder (not an ActionJS action) and use an **"Add custom wasm step"** button from within that action's editor. This is presumably what registers the component into your personal `Dev Block` namespace, which is why it's then visible/editable from the Block Store's "My Blocks → Dev blocks" filter afterward.

**This corrects an assumption in `WASM_Actions_Crash_Course.docx` section 3** ("How the pieces fit together" / end-to-end flow step 2), which describes uploading "through the upload UI or an automated publishing pipeline" as if the Block Store itself were the direct upload entry point. It's more accurate to say: the Block Store is where components are *cataloged and browsed* once they exist, but *first-time authoring/upload* happens from inside an app's WASM action editor.

**How found:** Marcel walked through this live while we were trying to find the upload flow ourselves and got stuck on the Block Store's read-only Functions tab.

**Where this lands:** crash course section 3 (the "How the pieces fit together" table/flow) needs the Block Store row and end-to-end flow step 2 corrected to reflect the real entry point.

**Status:** confirmed directly, first-hand, by Marcel.

---

## `function.json` is not part of the interactive "Add custom Wasm step" upload flow at all (2026-08-07)

**Fact:** watched live, step by step, uploading `create-record` via an app's WASM action editor ("Add custom Wasm step" → 3-step wizard: Upload file / Configure / Review). Step 1 ("Upload file") only accepts a single `.wasm` file — there is no separate field to upload `wit/world.wit` or `function.json` anywhere in this flow. Step 2 ("Configure") then lists each exported function **auto-detected directly from the component's own WIT interface** (it showed exactly one entry, named `create-record` — the literal WIT interface/function name, kebab-case, pulled from the binary's embedded component metadata) and presents a manual **Name / Description / Icon / Icon color** form per function, pre-filled with the WIT name and "No description provided" — completely independent of whatever `function.json` says.

**This explains (and revises) an earlier product feedback finding** ("description field is not read into block or step creation"): it's not that the platform silently drops `function.json`'s `description` — it's that **this upload path doesn't consume `function.json` at all**. Naming and describing a step happens by hand, twice, in this wizard (once at the "Block Store details" step for the block as a whole, once per function at the "Configure" step) — separately from anything written into `function.json`. `function.json` appears to matter only for the CLI/CI-driven publish pipeline (the `@betty-blocks/cli`/GitHub Actions OCI-push flow seen in `native-wasm-components`/`block-store-wasm-components`), not for this interactive, human-driven upload path.

**Why it matters for both audiences:**
- For BB developers (documentation): if you're hand-uploading a component via the IDE rather than through an automated publish pipeline, don't bother writing a careful `function.json` description expecting it to show up — you'll re-type it here regardless. Worth being explicit about which upload path reads `function.json` and which doesn't.
- For product devs (feedback): having a `function.json.description` field that's silently unused on one of the two upload paths (and, per an earlier finding, causes an opaque "Internal Server Error" in at least one case) is confusing for component authors — worth asking whether the two paths are meant to converge, and if so when.

**How found:** watched live over Marcel's shoulder while going through the real upload wizard for `create-record` in `marcel-tes.bettyblocks.com` (his own test app), 2026-08-07.

**Status:** confirmed directly, first-hand.

---

## A `list<custom-record>` WIT parameter does NOT auto-generate an inline Map/key-value editor — it renders as a "Select variable" schema-model picker (2026-08-07)

**Fact:** tried changing `create-record`'s `fields` parameter from a plain `string` to a proper `list<field-value>` (with `record field-value { name: string, value: string }`), hoping this would render like the native "Value Mapping" / HTTP-headers Map widget. Instead, the generated option form showed a **"Select variable"** input labeled "Based on schema model field-value_Create Record v3_1" — i.e. it's treated the same way as the `Model`/`Property` types (a strongly-typed reference expecting an existing variable of that exact shape), not an inline editable key-value list. To use this in practice you'd need to first build a `list<field-value>` variable via other steps (e.g. Init Array + Add Variable To Array, once per field) before this step could consume it — much more cumbersome than typing values directly.

**Practical implication**: there is currently no known WIT shape that gets you a friendly inline Map/key-value editor purely from WIT introspection (no `function.json`) through this upload path. The "Map" option type used by `format-endpoint-result`/`liquid-template` in `block-store-wasm-components` is a `function.json`-only concept — it cannot be reproduced by WIT shape alone in this flow. For a "type free-form JSON, or compose it with an Expression step" use case, a plain `string` parameter (see the `create-record` v2 approach) is actually the more usable choice today, even though it means hand-building any JSON structure yourself.

**How found:** built and live-tested `create-record` v3 with the `list<field-value>` redesign specifically to test this, per a suggestion to try mirroring the native Value Mapping pattern.

**Where this lands:** `WASM_Actions_Crash_Course.docx` section 4.4 — add this as a documented limitation/gotcha under "Auto-generated configuration forms."

**Status:** confirmed directly, first-hand.

---

## Name your WIT world something other than `main` if your component has a host import (2026-08-07)

**Fact:** a custom Block Store component whose WIT `world` is named `main` AND declares at least one host `import` (e.g. `betty-blocks-utilities:data-api/data-api`) uploads/registers fine, but fails with a generic "Failed to create action step" error the moment you try to drag it onto a real action's canvas. Renaming the world to anything other than `main` (no other change) fixes it. Components with *no* imports work fine with `world main` (e.g. `slugify-text`); components *with* imports work fine as long as the world isn't named `main` (e.g. the official `store-file-base64`, whose world is named `store-file-base64`).

**How found:** hit directly while building `create-record`, then isolated with a controlled test — confirmed by renaming `create-record`'s world from `main` to `create-record-world`, rebuilding, and re-testing.

**Where this lands:** `WASM_Actions_Crash_Course.docx` quick-start checklist (section 10) and component-anatomy section (section 4) — recommend a distinctly-named world (e.g. matching the component's own name) as the default convention for any component with host imports, rather than the generic `main` used in the simplest pure-computation examples.

**Status:** confirmed directly, first-hand, via a single-variable test.

**Update, 2026-09-24 — this rule is doubtful; likely coincidental, not a real mechanism.**
`exchange-code-for-tokens` (`wasm-blocks-store/functions/exchange-code-for-tokens/1.0/`) declares
`world main` with a real host import (`wasi:http/outgoing-handler@0.2.6`) — the exact shape this
entry says fails — and has worked reliably on canvas since 2026-09-02. More importantly: an earlier elimination chain (`slugify-text`/`make-slug`) already
established that renaming a component's WIT **world** produces a **byte-for-byte identical**
compiled `.wasm` (confirmed via `git hash-object`) — the world's name isn't embedded in the
compiled artifact at all. If it isn't in the binary, it cannot be what the platform reads to
decide success or failure, on *any* upload path. The original `create-record`/`store-file-base64`
comparison was real, but the actual variable was most likely something else entirely — quite
possibly the same 255-character `description` limit documented elsewhere in these logs, not the
world's name. Treat "rename the world away from `main`" as a workaround that happened to work
once, not a confirmed causal rule.

---

## ~~The platform auto-generates real Model/Property picker inputs directly from WIT types~~ — RETRACTED, it's a synthetic placeholder, not a real picker (2026-08-07, corrected 2026-08-10)

**Original (wrong) claim:** uploaded the official `store-file-base64` example (WIT params `model: model` and `property: list<property>`, using the shared `betty-blocks-utilities:types` records) as a step with no `function.json` involved. The generated option form rendered `model`/`property` as "Select variable" inputs labeled "Based on schema model ..." — this was read at the time as genuine, working native Model/Property picker UI generated straight from the WIT signature.

**Correction, 2026-08-10:** that read was wrong. Following the "Based on schema model ..." link this session (`/app/schema-models/<id>`) shows it's a literal, real, **read-only Data Model** the platform auto-creates in the app's own schema — named e.g. `model_Store File Base64_1`, with a single `name: Text` property mirroring the WIT record shape (`record model { name: string }`). It is **not** a reference to any real app Data Model, and there is no adaptive Property filtering by kind (`FILE`/`IMAGE`) as the official `function.json` specifies (`"allowedKinds": ["FILE","IMAGE"]`) — because this upload path never reads `function.json`, so that meta is simply never seen. The "Select variable" picker only ever offers variables matching this synthetic shape, which nothing in a real action can produce.

**Why the original test didn't catch this:** the step accepted the WIT types and rendered *some* option UI without error, and the label said "Model"/"schema model" — plausible-sounding, so it was taken as confirmation without following the link to see what it actually pointed at.

**Practical implication (superseded, see below):** there is currently no known way to get a genuine native Model/Property selector (backed by the app's real schema, adaptive Property list) on a third-party/Block Store component through **this upload path** — only the synthetic-schema-model fallback described above.

**Update, 2026-08-10 — the real fix is to use `bb functions publish` instead of the IDE wizard.** Confirmed live: published the same, byte-identical `store-file-base64` via the CLI (checksums verified against upstream) to the `marcel-tes` app. The resulting step's MODEL option rendered as a genuine "Select model" picker listing the app's real Data Models (Role, User, Webuser); selecting `Webuser`, PROPERTY correctly adapted and showed only `file` as selectable (every non-file/image property greyed out, matching `function.json`'s `allowedKinds: ["FILE","IMAGE"]`). Field labels/info tooltips also matched `function.json` exactly. **So: the synthetic-schema-model fallback is specific to the "Add custom Wasm step" IDE wizard (which never reads `function.json`, per the entry below) — it is not a platform-wide limitation.** For any option type whose UI depends on `function.json` meta (`Model`, `Property`, `Map`, `MultilineText`, etc.), publish via `bb functions publish` (see the CLI entry further below), not the interactive wizard.

**One caveat found in the same test:** the CLI-published step landed as an **app-scoped custom step** (tagged "Application specific" with a wrench icon in the step palette), not as a shared Block Store "dev block" — unlike what the ActionsJS `bb blocks publish`/`release` flow produces. Don't assume `bb functions publish`-ing a WASM function makes it available in other apps yet; unconfirmed whether that's a current limitation or requires an separate, not-yet-found release step.

**Where this lands:** `WASM_Actions_Crash_Course.docx` section 4.4 — document that `Model`/`Property`/etc. option types work correctly, but only when published via the CLI (`bb functions publish`), not the interactive wizard; the wizard's synthetic-schema-model fallback should be documented as a wizard-specific limitation, not a general platform gap.

**Status:** retracted and corrected 2026-08-10, then the practical implication itself superseded the same day once the CLI path was tested live and confirmed working.

---

## `@betty-blocks/cli`'s `bb functions publish` is a second, real WASM publish path — shares the CLI and manifest format with ActionsJS, but not the actual upload mechanics (2026-08-10)

**Fact:** `github.com/bettyblocks/cli` (npm: `@betty-blocks/cli`) ships a `bb functions` command family (`init`, `new`, `publish`, `validate`, `login`, `logout`, `bump`) that handles **both** ActionsJS ("Custom Functions", the WASM predecessor) and WASM function projects through the same commands, branching internally on a marker file in the project root: `.app-functions` → ActionsJS, `.wasm-functions` → WASM (confirmed in `bb-functions-publish.ts`/`bb-functions-init.ts`). `bb functions init --type wasm <identifier>` scaffolds a project with the `.wasm-functions` marker and a `functions/<name>/<version>/{function.json,Cargo.toml,src/lib.rs,wit/world.wit,Justfile}` layout. Our local `block-store-wasm-components-main` snapshot already has this exact shape (root `.wasm-functions` file present), so it's already a valid `bb functions` project as-is.

**What's genuinely shared** (one implementation, used by both project types): the `function.json` manifest format itself (validated against the same schema hosted in `github.com/bettyblocks/json-schema`), and the definition-reading helpers in `functionDefinitions.ts` (`functionDefinition()`, `isFunctionDefinition()`, the "stringify `options`/`paths` before upload" convention).

**What's genuinely different** — two separate implementation files, two different backends:

| | ActionsJS (`publishAppFunctions.ts`) | WASM (`publishWasmBlockStoreFunctions.ts`) |
|---|---|---|
| Packaging | Zips the whole `functions/` project (`AdmZip`) plus an auto-generated `index.js` barrel importing every function; one upload for the whole project | No zip — each selected function's raw `.wasm` uploaded individually, one HTTP request per function |
| Endpoint | `{builderApiUrl}/artifacts/actions/{applicationId}/functions` (the app's own Builder API) | `{blockstoreApiUrl}/blocks/publish?type=wasm-function` (the shared Block Store API) |
| `function.json` sent? | Always has been (JS has no WIT to introspect) | **Yes** — read from the function folder and sent alongside the `.wasm` in the same multipart form. This is the key difference from the interactive "Add custom Wasm step" IDE wizard, which never sends `function.json` at all. |
| Compile trigger | Response carries a `compiled` flag; `--skip-compile` controls whether upload also triggers action recompilation | No compile-related handling in this code path at all — Block Store registration is a separate step from an action's own compile cycle |

**Practical implication:** "same CLI tooling" is accurate (literally the same `bb functions` command, same manifest format) but "same flow" overstates it — the actual publish mechanics, target backend, and payload shape genuinely diverge between the two project types. Don't assume debugging one publish path tells you anything about the other.

**Correction/addition, 2026-08-10: ActionsJS actually has a second, entirely separate CLI publish route into the Block Store — `bb blocks`, not `bb functions` — and WASM has no equivalent of it.** Missed this on first pass. `bb blocks publish` + `bb blocks release` (`src/bb-blocks-publish.ts`, `src/blocks/publishBlocks.ts`, `src/blocks/releaseBlocks.ts`) is a distinct command family, hardcoded to only work on `.app-functions` (ActionsJS) projects (`bb-blocks-new.ts` explicitly checks for that marker file and refuses otherwise). A "block" here is a *named, curated bundle of one or more functions*, defined by its own `blocks/<name>.json` manifest — a different concept from a single `functions/<name>/<version>/function.json`. `bb blocks publish` zips that bundle and POSTs to `{blockstoreApiUrl}/blocks/publish`; `bb blocks release` then calls `GET /blocks/my-dev-blocks` + `POST /blocks/release` to promote it from a draft "dev block" to a "released block" (matches the Block Store UI's own "My Blocks → Dev blocks / Released blocks" split, confirmed in an earlier entry). Once released, any app can pull it in via the Block Store's own browsing UI, outside the CLI entirely.

So for ActionsJS there are, confirmed, three distinct ways functions end up somewhere: (1) `bb functions publish` direct-to-one-app (Builder API), (2) `bb blocks publish`+`release` into the shared Block Store (Block Store API), then installed into any app via the Block Store's own UI. **WASM only ever has path (1)'s shape but targeting path (2)'s backend** — `bb functions publish` on a `.wasm-functions` project always posts to the Block Store endpoint (`?type=wasm-function`), never to the Builder API, and there is no `bb blocks`-equivalent "release" step anywhere for WASM in this CLI. The WASM analogue of "pull an already-published block into an app, outside the CLI" is the Action Builder's own "Wasm steps" search palette (confirmed hands-on this session — that's where `Store File Base64` was found and dragged onto the canvas) rather than a separate Block Store page flow. Unconfirmed: whether a WASM Block Store component needs an analogous dev→released promotion before it's usable in an app other than the one it was published from — no CLI command exists for that, so if it's needed it'd have to happen through the Block Store page UI itself.

This also explains `block-store-action-functions` (BB's own bundle of many ActionsJS utility functions — `chunk`, `jsonpath`, `liquid`, `sub-action`, etc.): it's an `.app-functions` project whose README describes publishing straight to one app via `config.json`'s `applicationId` as a **testing shortcut** (i.e. using `bb functions publish` for fast local iteration) — the repo's real distribution path is presumably `bb blocks publish`/`release`, matching its name.

**Also confirmed:** `bb functions login` uses the same credentials as the IDE (per the command's own description). The CLI infers the target app (`identifier`) and environment (`zone`) from the project directory's name (`<identifier>.<zone>`) unless overridden by a `config.json` — a directory not named after the target app (like `block-store-wasm-components-main`) needs an explicit `config.json` with `identifier`/`zone` set, or it'll resolve to the wrong app.

**How found:** researching Betty Blocks' public GitHub org (`github.com/bettyblocks`) after a coworker mentioned the CLI might support WASM publishing "properly" — read the actual CLI source rather than relying on the secondhand claim.

**Where this lands:** `WASM_Actions_Crash_Course.docx` section 3 ("How the pieces fit together") — add `bb functions publish` as a second, real publish path alongside the IDE wizard, with the table above noting where it diverges from the ActionsJS flow.

**Update, 2026-08-10 — tested live, and the real-world result differs from what the endpoint name suggests.** Ran `bb functions publish` for real against the `marcel-tes` app (fresh `bb functions init --type wasm marcel-tes` project, `store-file-base64` duplicated in). Publish succeeded, `function.json` correctly produced real native Model/Property selectors  — but the resulting step showed up in the `marcel-tes` app's own step palette tagged **"Application specific"** (wrench icon), *not* under the Block Store's "My Blocks → Dev blocks" as a cross-app-installable block. So despite hitting the `/blocks/publish?type=wasm-function` endpoint (the same URL shape as ActionsJS's Block-Store-bound `bb blocks publish`), the actual behavior for WASM is app-scoped registration, matching ActionsJS's *other* command (`bb functions publish`, direct-to-app) in outcome even though it doesn't match it in endpoint. Revise the "WASM only ever has path (1)'s shape but targeting path (2)'s backend" claim above: it's closer to "path (1)'s *outcome*, via path (2)'s *URL*" — the endpoint name is not a reliable guide to where the block actually ends up. Whether WASM blocks can ever reach the shared, cross-app Block Store at all (the way ActionsJS's `bb blocks release` achieves) remains unconfirmed.

**Status:** confirmed directly from source, then confirmed live end-to-end, 2026-08-10.

---

## Reusing a Block Store component standalone just means resolving its workspace shortcuts to concrete values — nothing else needs to change; and `bb functions publish` can ask for the app UUID manually, contradicting an earlier entry (2026-08-19)

**Fact 1 — workspace-relative `Cargo.toml` fields are the *only* thing that needs adapting.** Copied `block-store-wasm-components/functions/generate-uuid/1.0/` (the canonical version — newer `wit-bindgen` 0.58.0, nested `bindings` module pattern, and a real `tests/mod.rs` integration test using `wasmtime-testing-helper` that instantiates the compiled component and calls it) verbatim into a fresh standalone `bb functions init --type wasm` project (`rug-test-ontwikkel`), with only `Cargo.toml`'s workspace shortcuts resolved to concrete values: `edition.workspace = true` → `edition = "2024"`, `wit-bindgen.workspace = true` → `wit-bindgen = "0.58.0"`, `wasmtime-testing-helper.workspace = true` → the same `git = "https://github.com/bettyblocks/wasmtime-testing-helper"` dependency spelled out directly, and `build = "../../../build.rs"` → `build = "build.rs"` with a local copy of the workspace-root `build.rs` placed inside the function's own folder (its `wkg wit fetch` logic is already path-independent). Everything else — `function.json`, `wit/world.wit`, `src/lib.rs`, `tests/mod.rs`, `wkg.lock` — copied byte-for-byte with zero edits. `cargo build --release --target wasm32-wasip2` and `cargo test` both succeeded first try, including the real `wasmtime-testing-helper` integration test. This confirms the fix already described in the "workspace-relative fields" entry above is genuinely sufficient on its own — no other adaptation was needed.

**Fact 2 (corrects the "resolved automatically" claim in the 2026-08-12 entry above) — `bb functions publish -a` can stop and ask for the app's production UUID by hand.** Running `bb functions publish -a` against the fresh `rug-test-ontwikkel` project validated the function successfully, then prompted interactively: `? Please provide the UUID for 'rug-test-ontwikkel' (production) ›` — it did **not** resolve the identifier to a UUID silently, unlike what the earlier entry described ("Nothing about typing or supplying that UUID yourself is required; it's resolved automatically from the identifier"). Not yet diagnosed *why* this project hit the manual-entry path — candidates include: this being the very first publish attempt for this identifier/CLI-session combination (no cached resolution yet), not being logged in yet at prompt time, or a genuine CLI behavior difference from whatever version/context produced the earlier "automatic" observation. Paused here rather than guess at a real production app's UUID; Marcel is running `bb functions publish -a` himself to supply it.

**Where this lands:** crash-course section on reusing a Block Store component in a standalone project — document the exact list of `Cargo.toml` fields to resolve (above) as the complete fix, and note the WIT/Rust/test files need no changes at all. Also flag in section 3.9 (the publish-flow write-up) that the "automatic UUID resolution" claim isn't reliable in every case — a manual UUID prompt can appear, and neither this nor the earlier entry has pinned down the exact trigger yet.

**Status:** Fact 1 confirmed directly, first-hand, 2026-08-19. Fact 2 observed directly, first-hand, 2026-08-19, but root cause not yet diagnosed — treat as an open correction, not a fully understood mechanism.

---

## The WASM counterpart of the ActionsJS `gql` helper is the host `betty-blocks-types:data-api` interface; a Block Store step can call it directly (2026-10-07)

**Fact, confirmed live on a canvas:** ActionsJS steps send Data API queries with the runtime global `gql(query, variables)`. A WASM step has no such global; it imports the host interface `betty-blocks-types:data-api/data-api@3.0.0` and calls `request(helper-context, query, variables)`. This works from a third-party (Block Store) step published with `bb functions publish`, with `helper-context` as the first parameter of the exported function (the platform fills it in; it is not a `function.json` option).

What was observed along the way:
- The host forwards the query as typed; it does not infer GraphQL variable types, and nothing replaces `{{name}}` placeholders (not the platform before the step runs, not the Data API). A step that wants `{{name}}` interpolation, like native Expression and HTTP, has to do it itself. A `Map` option arrives as a JSON object string with typed values (a text variable as a string, a record variable as the whole record object).
- A Data API error does not come back as a successful `{"errors": ...}` body: the host returns `Err`, and the runtime shows it as `ACTION RUN ERROR` with the GraphQL error envelope as an escaped JSON string in `message`.
- A `json-string` return value is JSON-parsed by the platform: returning non-JSON text from a step whose WIT return type is `json-string` fails with `JSON.parse: unexpected character at line 1 column 1 of the JSON data`.
- An empty `query()` is invalid GraphQL (`syntax error before ')'`), which is easy to mistake for a variables problem.

**How found:** a throwaway step that forwarded a raw query to the host and echoed what it received, tested on a canvas while porting the ActionsJS Data API Request step (`functions/data-api-request/`).

**Status:** confirmed first-hand.

---

## The Data API accepts a `take` of at most 200 (2026-10-08)

**Fact:** a query without `take` returns 200 records, which is also the maximum; a `take` of 201 or more is rejected as an unsupported request. This is not in the Data API documentation (which lists a 5 s query timeout, 10 queries per request, 4 relation levels and read-rate limits). The ActionsJS Data API Request step's README says "max 5,000" and its code clamps to 5000, so that cap does not match what the server accepts. Reading more than 200 records takes several queries using `skip`, ideally with a unique sort order so pages do not overlap; the WASM Data API Request step does this itself.

**How found:** tested against the Data API while building the WASM Data API Request step.

**Status:** reported after testing; the exact error text was not captured.

---

## A step that returns records as JSON: `json-string` binds as a Record or Collection; `list<betty-record-json>` does not; and without `id` later steps fail (2026-10-08)

**Fact, confirmed live:** the same JSON array of records returned from a step in three WIT shapes, then used in an action that fetched two records, looped over them and updated a property from each loop item:
- `result<json-string, string>` (a named alias of `string`): the result is a real array of objects, and the loop's Update step works. A single record returned the same way, bound as a Record, also works.
- `result<betty-record-json, string>` (the alias native Create uses for its Record output): identical behaviour.
- `result<list<betty-record-json>, string>` (one JSON string per record): each element stays a string, so the result is an array of strings and the loop's Update fails with `can't convert undefined to BigInt`.

So a custom step can hand a model-typed Collection or Record to later steps by returning it as one JSON string, with `{"type": "Collection", "model": "model"}` (or `Record`) in the `anyOf` of the Result option. The result only contains the properties that were queried: without `id`, an Update step fails with `can't convert undefined to BigInt` (the `id` identifies the record), and a property that was not queried reads as empty. The same holds for the properties of a relation.

**How found:** canvas tests of the WASM Data API Request step.

**Status:** confirmed first-hand.

---

## Drift: `wasm-base-components` HEAD has `upload-file@4.0.0`, while our docs and the live-tested `store-file-base64` fix use `@3.0.0` (2026-10-07)

**Fact:** on 2026-09-16 `wasm-base-components` moved every WIT package to a fresh major ("move every WIT package and component to a fresh major"), and HEAD now has `betty-blocks-types:upload-file@4.0.0` (same payload shape as `@3.0.0`'s `file-base64: string`, but `use`-ing `data-api@3.0.0`/`types@3.0.0`). The `@3.0.0` fix recorded above (2026-08-12) was live-confirmed and may still work, but any doc, copy-paste or new component that pins the older versions should be re-checked against what the platform currently provides before being trusted. Native `create` on `main` currently locks `betty-blocks-types:{crud,data-api,types}@3.0.0`. The repo also notes `betty-blocks-types:types@3.1.0` exists on its `dev` branch with mutable tags — don't pin to a `dev`-only version.

**How found:** comparing `wit/` in `wasm-base-components` HEAD and `functions/create/1.0/wkg.lock` in `native-wasm-components` HEAD while looking for the current `data-api` version.

**Status:** observed in source, 2026-10-07. Not verified which versions the live host serves; the Data API Request spike checks `data-api@3.0.0` live.

