# Contributing

This repo only accepts steps that are actually finished. It's tempting to merge
something the moment it compiles and passes local tests — resist that. A step that
only *looks* done (builds, unit-tests pass, `function.json` written) but was never
tried on a real canvas has burned people before: things that build clean locally
have failed silently or hung indefinitely once a real Model/Property value hit them
on an actual Betty Blocks app. Local tests don't catch that.

## Git workflow

This mirrors how Betty Blocks' own product team works in `block-store-wasm-components` and
`native-wasm-components` (checked directly against their real commit history, not guessed) —
same conventions, minus the internal Jira ticket references those repos have and this one
doesn't.

- **Branch naming**: `<type>/<short-kebab-description>`, e.g. `feat/add-dissect-params`,
  `fix/get-fields-empty-list-panic`, `docs/update-crash-course-uuid-resolution`. If there's a
  GitHub issue, referencing its number in the PR is enough — no ticket suffix required.
- **Commit messages**: [Conventional Commits](https://www.conventionalcommits.org/) — `type:
  short summary` in the imperative mood (`fix: handle empty options list`, not `fixed` or
  `fixes`). Common types here: `feat`, `fix`, `refactor`, `test`, `docs`, `chore`, `ci`. Add a
  body when the *why* isn't obvious from the summary alone — see the existing commit history in
  this repo for the level of detail worth including.
- **No direct pushes to `main`** once more than one person is actively working here — open a PR
  from a branch, even for a small change, so the review step below actually happens. (The very
  first few commits that scaffolded this repo were pushed straight to `main` before there was
  anyone to review against — that was a one-time bootstrapping exception, not the ongoing norm.)
- **CI must pass** (`ci.yml` — format, build, lint, test) before a PR is merged.
- Merge via the PR (GitHub's default merge commit is fine — that's what both product repos do;
  no need to squash or rebase unless you personally prefer a cleaner history on your own branch).
- **Branches delete themselves on merge** — this repo has GitHub's "Automatically delete head
  branches" turned on, so there's no manual cleanup step after merging. (Neither product repo
  nor wasco-dev has this on, and there's no cleanup workflow anywhere to copy — this is just the
  plain repo setting, turned on here because remembering to delete a branch by hand every merge
  is exactly the kind of thing that stops happening once more than one person is doing it.)

## Designing for the people who'll actually use this

The person configuring a step on canvas is almost always a business analyst or Betty
Blocks power user — not a developer, and not the person who wrote the step's code.
They can't read the Rust, don't have a stack trace to go on, and are debugging
entirely through labels, info text, and error messages. Design for that:

- **Plain English, but don't overcorrect.** Prefer everyday words over spec/protocol
  jargon in labels (`Expected Client ID` over `aud`). Don't go so far the other way
  that someone who *does* know the domain gets confused by an oversimplified label —
  the goal is clarity, not dumbing down.
- **Every option gets a real `info` string, always — no exceptions for "the label is
  self-explanatory."** Whether a label needs explaining isn't our call to make on
  behalf of whoever's actually configuring the step — if they already understand it,
  they just won't read the info text; but deciding for them that they don't need it
  is exactly the kind of judgment we're not in a position to make.
- **A step should basically only error on misconfiguration.** The person hitting an
  error is rarely the person who wrote the code and usually can't read it. So:
  validate every input against what's expected as early in the step as possible
  (fail fast, not three computations deep), and every error message should name what
  was expected, what was actually found, and — wherever there's a sensible fix — say
  what to check or change (e.g. `"...found \"Y\" — check that \"Expected Issuer\"
  matches the value your identity provider actually issues"`, not just `"...found
  \"Y\""`).
- **Flat output shapes over nested ones.** A canvas builder binds `step.output` by
  clicking through a picker, not by reading a schema — every extra layer of nested
  structure is one more thing to understand. Prefer several flat outputs, or one
  `SchemaModel`-typed `Object` output the builder can define fields on directly, over
  a nested record.
- **`required: true` on an output means "this is the reason the step exists," not
  "this happens to always be non-empty."** A step usually has one or two outputs that
  are the actual point of using it — those should be required. An output that's a
  genuinely optional extra (useful for some flows, irrelevant to most) should stay
  unmarked, even if it's always technically populated on success — marking it
  required forces the platform to materialize it as an available variable whether or
  not the builder needs it, which is pure clutter for the common case that doesn't
  use it.
- **Use `category` *and* icon `color` together** for discoverability — a step is
  found by scanning a palette, not by reading documentation first. Match sibling
  steps' conventions rather than inventing new ones per step.
- **Every `function.json` text field stays under 255 characters** (`description`,
  `label`, option `info`) — see the checklist item below; this is a hard platform
  constraint, not a style preference.

## Checklist before opening a PR

- [ ] **It's original.** Not a verbatim or near-verbatim port of an existing native
      or Block Store step, and not something that already exists as a generic, reusable
      component. Check
      [bettyblocks/block-store-wasm-components](https://github.com/bettyblocks/block-store-wasm-components),
      [bettyblocks/native-wasm-components](https://github.com/bettyblocks/native-wasm-components),
      and [wasco-dev](https://github.com/wasco-dev) before you start building, not after — this
      repo's own `functions/` folder started empty specifically because two locally-built
      "candidates" turned out to already exist in the first of these. If you built something as
      a learning exercise by copying an existing example, that's genuinely useful practice — it
      just doesn't belong in this repo. Open an issue instead if you think the *official*
      example needs a fix (e.g. a stale host-interface import) — that goes upstream to
      `bettyblocks/block-store-wasm-components`, not here. If something already exists on
      wasco-dev, see [`docs/wasco-dev.md`](docs/wasco-dev.md) for how to reuse it directly.
- [ ] **`function.json` is complete**: accurate `description`, `label`, `category`,
      `icon`, and every option documented with a real `info` string. Keep every text
      field — `description` (confirmed), and `label`/option `info` text (not yet
      individually confirmed, treat as the same limit until checked) — under 255
      characters. Past that, the value is **rejected outright** as a database write
      the moment the step is dragged onto a canvas (`CreateActionStep` fails with a
      generic "Something went wrong", not a length-specific error), not silently
      truncated — and front-end validation does not reliably catch it before publish.
      See `docs/product-feedback-log.md`'s resolved "custom Wasm step fails 'Add step
      to canvas'" entry for the full story; it cost a multi-day investigation the
      first time.
- [ ] **Designed for a non-developer user**: labels/info text/errors follow
      ["Designing for the people who'll actually use this"](#designing-for-the-people-wholl-actually-use-this)
      above — plain English where it helps, a real `info` string on *every* option,
      error messages that say what to check, flat output shapes, and `required` used
      to mean "the point of the step," not "always non-empty."
- [ ] **Unit tests exist and pass** (`just test`).
- [ ] **Actually published and tested live**: run `bb functions publish` against
      a real Betty Blocks app, drag the step onto an actual action canvas, and
      confirm it works with real input values — not just default/empty ones.
      Say which app you tested against in the PR description. See
      [`docs/testing-a-step-in-a-real-app.md`](docs/testing-a-step-in-a-real-app.md)
      for how to do this from a clone of this repo — `bb functions publish` can't
      run directly against `wasm-blocks-store` itself.
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

## Is this actually a wasco-dev contribution?

[wasco-dev](https://github.com/wasco-dev) is a separate, public org (started by Chris Obdam,
not this team) building generic WebAssembly components meant to be usable by any service that
runs WASM, not just Betty Blocks — see [`docs/wasco-dev.md`](docs/wasco-dev.md) for the full
picture. Before building something new here, ask whether it actually belongs there instead:

- If it has **nothing Betty-Blocks-specific about it** — no `betty-blocks-types:*` host
  imports, no dependency on `function.json`-only option types (Model, Property, Map,
  MultilineText) — it's a candidate for wasco-dev rather than (or in addition to) this repo. A
  generic string/date utility or a plain third-party-API wrapper is exactly this shape.
- If it genuinely needs a Betty Blocks host capability (real Model/Property selectors, the
  platform's own data API, anything only `bb functions publish` exposes), it belongs here, not
  there.

If you decide something belongs on wasco-dev: **there's no `CONTRIBUTING.md` or documented PR
process there** (checked directly — it doesn't exist). That means proposing it to Chris Obdam
directly, not opening a PR against an established process. This isn't a reason to avoid it, just
a reason not to assume the same PR-and-review flow this repo uses applies over there.

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
