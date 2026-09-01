# Contributing

This repo only accepts steps that are actually finished. It's tempting to merge
something the moment it compiles and passes local tests — resist that. A step that
only *looks* done (builds, unit-tests pass, `function.json` written) but was never
tried on a real canvas has burned people before: things that build clean locally
have failed silently or hung indefinitely once a real Model/Property value hit them
on an actual Betty Blocks app. Local tests don't catch that.

## Checklist before opening a PR

- [ ] **It's original.** Not a verbatim or near-verbatim port of an existing native
      or Block Store step. If you built something as a learning exercise by copying
      an existing example, that's genuinely useful practice — it just doesn't belong
      in this repo. Open an issue instead if you think the *official* example needs
      a fix (e.g. a stale host-interface import) — that goes upstream to
      `bettyblocks/block-store-wasm-components`, not here.
- [ ] **`function.json` is complete**: accurate `description`, `label`, `category`,
      `icon`, and every option documented with a real `info` string. Keep the
      description under 500 characters — it's silently truncated past that, and it's
      not currently surfaced in the BB IDE at block/step creation time anyway, so
      don't rely on it being visible there.
- [ ] **Unit tests exist and pass** (`just test`).
- [ ] **Actually published and tested live**: run `bb functions publish` against
      a real Betty Blocks app, drag the step onto an actual action canvas, and
      confirm it works with real input values — not just default/empty ones.
      Say which app you tested against in the PR description.
- [ ] **Formatted and lint-clean**: `just format-check` and `just quality-check`
      both pass (CI enforces this, but check locally first).
- [ ] **Versioned correctly**: new step starts at `1.0/`. A breaking change to an
      existing step's interface gets a new version directory (e.g. `2.0/`) rather
      than mutating `1.0/` in place, so apps already using it aren't broken under
      them.

## PR description

Include, at minimum:

- What the step does and why it's useful (the problem it solves).
- Which Betty Blocks app / canvas you tested it on, and what you tried (edge cases,
  not just the happy path).
- Anything you found non-obvious while building it — if it's a platform quirk worth
  other developers knowing, consider it belongs in the PR description even if it's
  not strictly about this step.
- If building it taught you anything worth logging, add it to `docs/product-feedback-log.md`
  or `docs/developer-learnings-log.md` (see [`docs/README.md`](docs/README.md)) as part of the
  same PR rather than as a separate follow-up.

## Grouping related steps into a block

If several steps are meant to be used together (not standalone, like `slugify-text`, but a set
that forms one logical capability), you can group them with a manifest in `blocks/` once **every
one of them already exists individually in `functions/` and has cleared the checklist above on
its own**. See [`blocks/README.md`](blocks/README.md) for the manifest shape and why it's framed
as a forward-looking convention rather than a real, CLI-consumed feature today.

## Contributing knowledge without a step

Not every PR needs a new component. If you confirmed something new and non-obvious while
debugging, testing, or just reading platform source — a bug, a gotcha, a corrected assumption —
that's a valid PR on its own: an update to `docs/product-feedback-log.md` or
`docs/developer-learnings-log.md`, following the existing entry format (what was found, how,
why it matters, status). Verify it yourself before writing it as fact; if you're relaying
something secondhand (a teammate's claim, a meeting note) rather than something you tested
directly, say so explicitly in the entry rather than stating it as confirmed — see the
retracted/corrected entries already in those logs for why that distinction matters.

## Review

Another contributor should actually pull the branch, build it, and ideally publish
it to their own app before approving — not just read the diff. This is a small,
early repo; a few minutes of real testing per PR is cheap now and expensive to skip
once there are dozens of steps depending on each other's conventions being right.
