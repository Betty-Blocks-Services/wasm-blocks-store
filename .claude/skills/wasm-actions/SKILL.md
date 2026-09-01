---
name: wasm-actions
description: >
  Betty Blocks WASM Actions knowledge base — architecture, Rust/WIT patterns, known platform
  gotchas, and the contribution bar for this repo. Use this whenever working on a Betty Blocks
  WASM Action / custom Wasm step: building or debugging a component, touching function.json,
  wit/world.wit, or a functions/<name>/<version>/ folder, running `bb functions`/`just`/`wkg`
  commands, or asking anything about how WASM Actions, the Block Store, or wasmCloud work on
  Betty Blocks. Also trigger when discovering something new and non-obvious about the platform
  through hands-on testing — this skill explains where that finding should be logged.
---

# Betty Blocks WASM Actions

This repo is where Betty Blocks developers (not the product team — see the root
[README.md](../../README.md)) share original custom WASM steps, and the knowledge that comes
from building them. That knowledge only compounds if it's actually read before repeating work
that's already been done, and actually written down the moment something new is confirmed. Both
halves are this skill's job.

## Git workflow: branch + PR, not direct pushes to main

Once there's more than a single-person bootstrap happening, don't push directly to `main` —
create a branch and open a PR, even for a small change (a log entry, a docs fix), so the review
step in [CONTRIBUTING.md](../../CONTRIBUTING.md) actually happens. This mirrors Betty Blocks'
own product team's real convention in `block-store-wasm-components`/`native-wasm-components`
(confirmed from their actual commit history, not assumed):

- Branch name: `<type>/<short-kebab-description>` — e.g. `fix/get-fields-empty-list-panic`,
  `docs/update-crash-course-uuid-resolution`.
- Commit messages: [Conventional Commits](https://www.conventionalcommits.org/) — `type: summary`
  in the imperative mood. Types in use here: `feat`, `fix`, `refactor`, `test`, `docs`, `chore`,
  `ci`.
- Open the PR, don't merge it yourself — that's the human review step, not something to skip
  just because CI is green. If you don't have push access to open a PR, say so explicitly
  rather than silently pushing to `main` anyway or dropping the change.

Full detail: [CONTRIBUTING.md](../../CONTRIBUTING.md)'s "Git workflow" section.

## Before starting any WASM Actions work, read

1. **`docs/product-feedback-log.md`** and **`docs/developer-learnings-log.md`** — every confirmed
   finding so far, newest first. Skim recent entries for anything touching what you're about to
   do. Some are platform bugs already root-caused with a known fix or workaround — re-discovering
   them from scratch wastes time. A few examples of investigations already closed out that don't
   need repeating: a WIT world named `main` combined with a host import breaks step-creation
   (name it anything else); the `store-file-base64` example's stale `betty-blocks-utilities:
   upload-file` import (moved to `betty-blocks-types:upload-file@3.0.0`); renaming/re-versioning
   source files does not change an already-built `.wasm` — you must actually rebuild. Don't take
   this list as complete — read the logs themselves.
2. **`docs/crash-course/wasm-actions-crash-course.md`** and **`docs/crash-course/
   rust-crash-course.md`** — the architecture/platform primer and the Rust primer, respectively.
   Written for a total beginner to both Rust and the `bb` CLI — if you're extending these docs,
   keep that bar (explain every command, never assume prior Rust/CLI familiarity).
3. **[bettyblocks/block-store-wasm-components](https://github.com/bettyblocks/block-store-wasm-components)**
   — Betty Blocks' own production Block Store component examples (generate-uuid, generate-random-hex,
   store-file-base64, etc.). The best source of truth for current `function.json`/WIT/Rust
   patterns; check it before trusting this repo's docs if they disagree, since the product
   team's repo moves independently of this one.
4. **[bettyblocks/native-wasm-components](https://github.com/bettyblocks/native-wasm-components)**
   — Betty Blocks' first-party, built-into-the-platform steps (Create Record, Update, Upsert,
   Delete, Expression, HTTP, Send Mail, Logging, etc.). Different repo, different build stack
   (Elixir/mix, not this repo's Cargo/Justfile setup) — you can't build against it, but you can
   read its `functions/` (or equivalent) listing to see what already ships natively.

**Check both of these before starting to build anything new** — see "Before building a new
step" below for why this specific check matters here.

## While working: verify before you write something down as fact

This log has already had to publicly retract two claims that turned out to trace back only to
secondhand meeting notes rather than a primary source, and a third that was originally wrong
and only caught because someone followed the link and checked what it actually pointed at (see
`docs/developer-learnings-log.md`'s "RETRACTED" entry, and `docs/product-feedback-log.md`'s
"Correction" entries). The lesson that cost: a plausible-sounding result from one test isn't
confirmation — check what it actually resolved to before writing it down as settled. Concretely:

- Something you tested yourself, live, with a reproducible before/after → write it as fact.
- Something a teammate mentioned, or that you read in a meeting summary, ticket, or chat →
  either verify it yourself first, or write it into the log explicitly flagged as
  unverified/secondhand, not as fact.
- If a live test's result *could* mean more than one thing (e.g. "it rendered a picker labeled
  Model" doesn't by itself mean "it's a real Model selector") — follow through and check what it
  actually resolved to before concluding.

## When you confirm something new

Log it immediately, in the same turn you confirm it — don't wait to be asked, and don't batch it
for later. Add it to `docs/product-feedback-log.md` if it's feedback for Betty Blocks' product
developers (a platform bug, gap, or confusing behavior in the WASM tooling itself), or
`docs/developer-learnings-log.md` if it's a non-obvious fact worth other BB developers knowing
(a gotcha, a build-tooling quirk, a corrected assumption). Match the existing entry format: what
was observed/the fact, how it was found, why it matters or where it should land in the docs, and
a status line (open / confirmed / resolved / retracted). Newest entries go at the top.

Then **actually get it back into this shared repo**, not just left in your local working copy —
commit the log update (and update the relevant crash-course doc if the finding changes something
it currently says) on a branch, following "Git workflow" above, and open a PR. That commit-and-PR
step is the entire point: a finding that stays on one person's machine doesn't compound for
anyone else.

## Before building a new step: check it doesn't already exist

Before writing any code for a step someone wants to add here, search both
[block-store-wasm-components](https://github.com/bettyblocks/block-store-wasm-components) and
[native-wasm-components](https://github.com/bettyblocks/native-wasm-components) for a step that
already does this. If either has it, building it here anyway is a learning exercise (genuinely
worth doing for practice), not a contribution to this repo — say so up front, before any code
gets written, not after.

This isn't a hypothetical: this repo's own `functions/` folder is still empty specifically
because `generate-random-hex` and `generate-uuid` — both built locally as practice — turned out
to be near-verbatim ports of existing `block-store-wasm-components` examples, not original
steps, and neither cleared this bar. Checking the two product repos *first* catches this before
time goes into building and testing something that was never going to be eligible, rather than
after.

## Contributing an actual step (not just a log entry)

A step only belongs in `functions/` once it clears the bar in [CONTRIBUTING.md](../../CONTRIBUTING.md)
— genuinely original (not a port of something that already ships as a native or Block Store
step), and confirmed working on a real, live action canvas, not just locally built and unit
tested. Read that file before proposing a new step, and don't treat "it compiles and passes
`cargo test`" as "it's done."

If several steps are meant to work together as one logical capability (not standalone), once
every one of them individually clears that bar, group them with a manifest in
[`blocks/`](../../blocks/README.md) — a forward-looking convention (WASM has no `bb blocks`
CLI/platform support yet, but Betty Blocks has confirmed it's coming), not a currently-consumed
feature.

## A note on how this knowledge reaches people

This skill only activates for someone who has cloned this repo and opened Claude Code inside it
— it isn't (yet) distributed as an installable plugin. That's a deliberate, current choice, not
a limitation to work around: the workflow is "clone `wasm-blocks-store` first," and this skill
is what turns that clone into an onboarded, context-aware session automatically. If the audience
outgrows "people who'd clone this repo anyway," packaging this as a proper plugin/marketplace
entry is the natural next step — but that's future work, not something to set up preemptively.
