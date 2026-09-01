# wasco.dev — a public, generic WASM component registry

[`github.com/wasco-dev`](https://github.com/wasco-dev) is a separate, public GitHub org started
by Chris Obdam, with a wider goal than Betty Blocks: a registry of generic WebAssembly
components — built with plain wasmCloud/WASI tooling (`wash`, `wkg`), not `bb functions` — meant
to be usable by any service that runs WASM components, not just Betty Blocks. As of 2026-09-01
it has 11 public repos: a landing site + registry docs (`wasco-dev`), a shared CI/CD workflows
repo (`workflows`), a Claude Skill collection (`skills`), and 8 actual components (`datetime`,
`openai-api`, `openid-connect-api`, `oauth`, `glyphic-api`, `heyreach-api`, `ironpress`,
`string_json_functions`).

This is not hypothetical or "someday" — Betty Blocks' own product team already depends on it:
`bettyblocks/block-store-wasm-components`'s CI uses `wasco-dev/workflows/.github/actions/
install-wkg` directly, and wasco-dev's own CD pipeline pushes every component not just to the
public `ghcr.io/wasco-dev/*` registry but also to `DEV_AZURE_REGISTRY`/`PROD_AZURE_REGISTRY` —
the same dev/prod Azure split `block-store-wasm-components` uses for its own official examples.

**One real gap, worth knowing before treating this as fully "open source":** none of the 11
repos have an OSS LICENSE file (checked directly). They're public and clonable, but not
licensed for reuse in the legal sense yet. There's also no `CONTRIBUTING.md` anywhere in the
org — no documented process for proposing a new component. If you want to contribute something
back, that means reaching out to Chris Obdam directly, not opening a PR against an established
process.

## Reusing an existing wasco-dev component inside Betty Blocks

Before building something from scratch, check whether it already exists at
[github.com/wasco-dev](https://github.com/wasco-dev). If it does, and it's generic enough (see
"Is it actually portable?" below), you can use it as-is:

1. **Find the component's package coordinates.** On its GitHub repo page, the right-hand
   sidebar has a "Packages" link — click through to see published versions. If it says "No
   packages published," that component hasn't been published yet; ask the author.
2. **Pull the `.wasm` file.** Install [`wash`](https://wasmcloud.com/docs/installation) (must be
   2.0.0+ — `wash --version` to check), then:
   ```bash
   wash oci pull ghcr.io/wasco-dev/<component-name>:<version>
   ```
   This overwrites any existing `component.wasm` in your current directory — run it somewhere
   clean.
3. **Upload it into your Betty Blocks app via the interactive "Add custom Wasm step" wizard.**
   This is the key fact that makes this work with zero changes: the wizard never reads
   `function.json` at all (confirmed in `product-feedback-log.md`) — it builds the option form
   purely from the compiled `.wasm`'s own WIT interface. Most wasco-dev components don't ship a
   `function.json` (only `string_json_functions` does, as of this writing), but that's fine —
   the wizard doesn't need one. You'll get a working step, just with plain WIT-derived labels
   instead of a hand-written description, and none of the `function.json`-only option types
   (Map, Model, Property, MultilineText) — same limitation any WIT-only component has here.

## Is it actually portable? Check before assuming

Not every wasco-dev component is a drop-in `.wasm` you can hand to the wizard as-is:

- **Check its WIT imports.** A component with zero imports (e.g. `datetime`) is trivially
  portable. One that imports only standard WASI interfaces (e.g. `openai-api` imports just
  `wasi:http/outgoing-handler@0.2.0`, the same capability Betty Blocks' own official examples
  use) should also work. A component importing something wasco-dev-specific that Betty Blocks'
  host doesn't provide would not.
- **Check if it needs composition first.** Some components are explicitly designed to be
  *composed* with another component rather than run standalone — `openai-api`'s own README says
  it's meant to be linked with a separate HTTP-proxy component via `wac plug` before it's a
  single deployable `.wasm`. Read the component's README before assuming a straight pull-and-
  upload works.

## Where a new step should actually land

If you're about to build something new that has nothing Betty-Blocks-specific about it — no
`betty-blocks-types:*` host imports, no dependency on BB-only `function.json` option types, just
generic logic or a generic third-party API wrapper — consider whether it belongs on wasco-dev
instead of (or in addition to) this repo. See [`CONTRIBUTING.md`](../CONTRIBUTING.md)'s
"Is this actually a wasco-dev contribution?" section for how to think about that split, and
remember there's no PR process there today — that means talking to Chris Obdam directly.

wasco-dev also has its own Claude Skill for generating a new component from an OpenAPI spec
(`wasco-dev/skills`, `openapi-to-wasm`) — worth a look if what you're building wraps a
documented REST API, since it's almost certainly what produced several of the existing
components' near-identical shape (strict naming convention `{namespace}:{api-name}-api@
{version}`, credentials passed as WIT parameters rather than environment variables, since WASM
components are stateless and isolated).
