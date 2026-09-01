## What this does and why

<!-- The problem this solves. If it's a new step, what it does and why it's useful. -->

## Testing

<!-- Which Betty Blocks app / canvas did you actually test this on (not just `cargo test`)?
     What did you try beyond the happy path? -->

## Checklist

<!-- Delete whichever section below doesn't apply to this PR. -->

**New or updated step:**

- [ ] Not a port of an existing native or Block Store step (checked
      [block-store-wasm-components](https://github.com/bettyblocks/block-store-wasm-components)
      and [native-wasm-components](https://github.com/bettyblocks/native-wasm-components))
- [ ] `function.json` complete (`description`, `label`, `category`, `icon`, every option's `info`)
- [ ] Unit tests pass (`just test`)
- [ ] Actually published (`bb functions publish`) and dragged onto a real action canvas —
      not just built and unit-tested locally
- [ ] `just format-check` and `just quality-check` pass
- [ ] Versioned correctly (new step at `1.0/`; breaking change to an existing step gets a new
      version directory, not an in-place edit)

**Knowledge-only (docs / log entry, no step):**

- [ ] Something you tested yourself, not just relayed secondhand — or explicitly flagged as
      unverified if it is
- [ ] Added to `docs/product-feedback-log.md` or `docs/developer-learnings-log.md`, following
      the existing entry format

## Anything non-obvious

<!-- A platform quirk, gotcha, or corrected assumption worth other developers knowing, even if
     it's not the main point of this PR. -->
