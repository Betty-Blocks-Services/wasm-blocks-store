# wasm-blocks-store

Community-maintained WASM Action components for Betty Blocks, built by Betty Blocks
developers — **not** by the product team. This lives in the `Betty-Blocks-Services`
org specifically so steps written by BB developers and their peers can be shared with
other Betty Blocks developers, rather than living forever in someone's local sandbox.

This is a sibling to, not a replacement for, Betty Blocks' own
[`bettyblocks/block-store-wasm-components`](https://github.com/bettyblocks/block-store-wasm-components) —
that repo is the product team's own shipped examples and is the best reference for
current `function.json`/WIT/Rust patterns. This repo is for steps that don't exist
there: original custom steps solving a real need, built and maintained by the
community.

## What belongs here

Only **completed, working** steps. Concretely, before something lands on `main`:

- It's genuinely a new step — not a verbatim or near-verbatim copy of a component
  that already ships as a native or Block Store step (those already exist; porting
  one for learning purposes is great practice, but it isn't community contribution).
- It's been published via `bb functions publish` and actually dragged onto a live
  action canvas in a real Betty Blocks app, and confirmed to work — not just
  "builds and passes `cargo test` locally."

See [CONTRIBUTING.md](CONTRIBUTING.md) for the full checklist and PR process.

## Layout

Each step is its own crate under `functions/<step-name>/<version>/`, following the
same convention as `block-store-wasm-components`:

```
functions/
  <step-name>/
    1.0/
      Cargo.toml
      function.json      # Betty Blocks step definition — options, labels, category
      wit/world.wit       # the exported WIT interface
      src/lib.rs
      tests/mod.rs
```

## Prerequisites

- Rust with the `wasm32-wasip2` target: `rustup target add wasm32-wasip2`
- [`just`](https://github.com/casey/just) — `brew install just`
- [`wkg`](https://github.com/bytecodealliance/wasm-pkg-tools) — fetches WIT
  dependencies during build
- [`@betty-blocks/cli`](https://www.npmjs.com/package/@betty-blocks/cli) (`bb`) —
  to publish a step to your own app for testing

## Build & test

```sh
just build           # cargo build --release --target wasm32-wasip2, then copies
                      # each .wasm next to its function.json
just test             # build, then cargo test (runs each function's tests/mod.rs)
just format            # cargo fmt
just format-check      # cargo fmt --check (what CI runs)
just quality-check     # cargo clippy --all-targets -D warnings
```

## Try a step in your own app

```sh
bb functions publish
```

`publish` prompts you to log in automatically if needed — no separate login step.
It publishes every function in the project in one pass.

## Status

Freshly scaffolded — no steps merged yet. The first ones will land once they clear
the checklist in [CONTRIBUTING.md](CONTRIBUTING.md).
