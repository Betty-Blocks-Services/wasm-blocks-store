# blocks/

**Forward-looking convention, not a real feature yet.** Nothing in the current `bb` CLI or
Betty Blocks platform reads anything in this folder. It exists so that a group of steps meant
to be used together (e.g. the several helper steps that make up a single logical capability)
already reads as one unit in this repo, before there's tooling to actually package them that
way for WASM. Betty Blocks is aware WASM has no equivalent of ActionsJS's `bb blocks` yet and
is expected to build one — when they do, revisit this convention against whatever they actually
ship rather than assuming this guess matches it exactly.

## Why this shape

ActionsJS has a real, working version of this today: `bb blocks new <name>` creates
`blocks/<name>.json`, and `bb blocks publish` zips the listed functions (plus a generated
`package.json`/`index.js`) and uploads them as one unit. A real example, from Betty Blocks'
own [`block-store-action-functions`](https://github.com/bettyblocks/block-store-action-functions/blob/main/blocks/generative-ai.json):

```json
{
  "dependencies": ["sentence-splitter"],
  "functions": ["collectionSearch 1.0", "aiQueryGenerator 1.0", "chunk 1.1"],
  "includes": []
}
```

`dependencies` and `includes` exist because that publish step builds an actual npm package
around the block (pulling `dependencies` from the project's root `package.json`, generating an
`index.js` that `require`s each function). That's npm/JS-bundling machinery with no WASM
equivalent — each WASM function is already an independently compiled `.wasm` binary, no
bundler involved. So the proposed shape here drops both fields and keeps only the part that
still makes sense:

```json
{
  "functions": ["add-ids 1.0", "dissect-params 1.0", "get-fields 1.0"]
}
```

One JSON file per block, filename = block name (kebab-case, matching this repo's `functions/`
naming — no camelCase translation needed the way ActionsJS's `collectionSearch` vs.
`collection-search` folder required). Each string in `functions` is `"<name> <version>"`,
referencing an existing `functions/<name>/<version>/` folder in this repo.

## When to add one

Only once every function it lists already exists under `functions/` and has individually
cleared the checklist in [../CONTRIBUTING.md](../CONTRIBUTING.md) — a block manifest groups
steps that are already real and working, it isn't a way to plan or scaffold steps that don't
exist yet.
