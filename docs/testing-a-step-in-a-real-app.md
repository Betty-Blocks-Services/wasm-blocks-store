# Testing a step from this repo in a real Betty Blocks app

The last item on [`CONTRIBUTING.md`](../CONTRIBUTING.md)'s checklist is "actually published and
tested live" — this page is the how-to for that step.

## Why you can't just `bb functions publish` from inside this repo

It's tempting to think a shared catalog repo could be directly publishable to whichever app you
want, on demand — clone `wasm-blocks-store`, run `bb functions publish`, get prompted fresh for
login and a target app. **That doesn't work, and it's not a repo-shape problem you can fix by
restructuring `functions/`.** Confirmed directly:

- `bb functions validate`/`bb functions publish` already run fine against this repo's existing
  workspace-shaped `functions/` folder with zero restructuring — validating a function here works
  even though this repo was never `bb functions init`'d. So "make the repo look like a
  CLI-scaffolded project" isn't the fix for anything.
- `bb functions init <identifier> --type wasm` always scaffolds a **brand-new subdirectory**
  named after `<identifier>` — it does not bind an already-existing `functions/` folder to an app
  in place. There's no flag or mode that points `init` at a folder that already exists.
- `bb functions publish --help` has no `--app`/`--identifier` override flag (only `--skip-compile`
  and `-a`/`--all`) — there's no way to tell `publish` "target this specific app" ad hoc on the
  command line either.

The app-binding the CLI needs is established once, via `init`, and `init` only ever creates a
fresh project directory — it's a "one directory = one app, forever" design. A shared catalog repo
that many developers clone to target many different apps can't satisfy that 1:1 relationship no
matter how its own folders are arranged; the mismatch is conceptual, not structural. (We didn't
push a live test far enough to capture the exact error/prompt this produces at the
login/identifier-resolution step, specifically to avoid risking an unintended publish going
through on a possibly-cached login session. If you want to pin that exact message down, it's safe
to test yourself with your own Betty Blocks credentials, inside this repo, since you'd be the one
authenticating — just stop before confirming any actual publish.)

## The working pattern: a separate, disposable test project that symlinks the real code

1. **Scaffold a personal test project, outside this repo.** In a sibling directory (not inside
   `wasm-blocks-store`):

   ```bash
   bb functions init <your-app-identifier> --type wasm
   ```

   This creates `./<your-app-identifier>/` with a placeholder `functions/say-hello/1.0/` example —
   that's the project the CLI now knows how to publish to.

2. **Deal with the scaffolded `say-hello` sample immediately — before you symlink anything in.**
   `init` ships `functions/say-hello/1.0/` with source but no compiled `.wasm`. Leaving it in
   place makes `bb functions publish` fail validation for the **whole project**, not just the
   sample:

   ```
   Error: Missing .wasm file in .../functions/say-hello/1.0
   ```

   It's also your reference for step 4 below (what a standalone `Cargo.toml`/`Justfile` looks
   like), so look at it first — then either delete `functions/say-hello/` entirely, or run
   `just build` inside it so it has a `.wasm` file. Do this now, not right before you try to
   publish.

3. **For each function you want to test, symlink the real code in — don't copy it.** Inside the
   new project, under `functions/<name>/1.0/`, symlink `src/`, `wit/`, and `function.json` back to
   their real location in `wasm-blocks-store/functions/<name>/1.0/`:

   ```bash
   ln -s /path/to/wasm-blocks-store/functions/<name>/1.0/src src
   ln -s /path/to/wasm-blocks-store/functions/<name>/1.0/wit wit
   ln -s /path/to/wasm-blocks-store/functions/<name>/1.0/function.json function.json
   ```

   This keeps exactly one copy of the actual step logic. Edits made in `wasm-blocks-store` are
   what you're testing live, with nothing to sync back afterward.

4. **Write a standalone `Cargo.toml` and `Justfile` for that function.** `wasm-blocks-store` is a
   Cargo workspace, so its functions use `dep.workspace = true` shortcuts and share a root
   `Justfile`/`build.rs`. A `bb functions init`-scaffolded project has none of that — it's a flat,
   standalone-per-function layout, so each function needs its own `Cargo.toml` (literal dependency
   versions, no `.workspace = true`) and its own `Justfile` (`fetch_wit_deps` / `build` /
   `move_wasm_to_root` / `test` targets). Don't assume these are identical to `wasm-blocks-store`'s
   root versions — they aren't. Copy the shape from the `say-hello` sample you looked at in step 2
   instead of guessing.

5. **Build and test standalone, inside the function's own directory:**

   ```bash
   just build && just test
   ```

   This confirms the symlinked code still builds and passes its unit tests outside the workspace,
   with the flat project's own `Cargo.toml`/`Justfile`.

6. **Publish from the test project's root** — this is where the real login/app-identifier flow
   happens, against the app that project was `init`'d for:

   ```bash
   bb functions publish
   ```

7. **Drag the step onto a real action canvas in that app and confirm it works with real input
   values** — not just default/empty ones. This is what the `CONTRIBUTING.md` checklist item is
   actually asking for.

8. **Once the step is fully verified, the test project is disposable.** Nothing unique lives in
   it — everything was symlinked from `wasm-blocks-store` — so it can be deleted once you're done.

   The one exception: if you're building a step specifically for one customer/developer's own app
   rather than for reuse via `wasm-blocks-store`, the separate project is the one that should be
   kept and maintained going forward, not discarded. That's a case-by-case call, not the default.

## Troubleshooting

**`400` error at publish time on an option that passed `bb functions validate` locally.** The
live publish endpoint's schema appears to be stricter than the CLI's own bundled local schema —
seen with an option shaped like:

```json
{
  "meta": { "type": "Text", "validations": { "required": true } },
  "configuration": { "placeholder": "******" },
  "name": "client-secret",
  "label": "Client Secret"
}
```

This passes `bb functions validate` locally, but the live endpoint rejects it:

```
Error: 400, #/options/2/meta: Expected exactly one of the schemata to match, but none of them did.
```

The working theory (not independently confirmed against the live schema's own source) is that
`"configuration"` isn't valid alongside `meta.type: "Text"` server-side — every confirmed-working
example in `block-store-wasm-components` (`liquid-template`, `store-file-base64`) only ever pairs
`"configuration"` with `"MultilineText"` or `"Property"`, never `"Text"`. If you hit this, try
removing the `"configuration"` block and publishing a plain `"Text"` option instead. This is also
one less thing to solve than it looks: a secret-like input (a client secret, a token) doesn't need
step-level masking at all — Betty Blocks' platform has its own Configuration feature for supplying
values like this into a flow, with masking handled there, so a plain option is the right shape
regardless of whether the `"configuration"` block turns out to be the actual cause of the `400`.

## Summary

| Question | Answer |
|---|---|
| Can I `bb functions publish` directly from `wasm-blocks-store`? | No — it's never been bound to an app via `init`, and there's no flag to target one ad hoc. |
| Do I need to restructure this repo to fix that? | No — the repo's shape was never the problem. |
| What do I do instead? | `init` a separate, disposable test project elsewhere; symlink the real `src/`/`wit/`/`function.json` in; write standalone `Cargo.toml`/`Justfile`; `just build && just test`; `bb functions publish` from there. |
| What happens to the test project afterward? | Discard it, unless it's actually a dedicated per-app project you're keeping long-term. |
