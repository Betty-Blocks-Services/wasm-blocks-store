# WASM Actions — developer learnings log (for Betty Blocks developers using WASM features)

Shared, living log of non-obvious, hands-on-discovered facts worth folding into the crash-course docs in `crash-course/` for Betty Blocks developers (people building applications on BB who use its WASM Actions feature) — as opposed to feedback aimed at BB's own product developers, which is tracked separately in `product-feedback-log.md`.

This log started in one person's local learning project and now lives here so it compounds across everyone using this repo — see `../.claude/skills/wasm-actions/SKILL.md` for how findings get logged here going forward, and `../CONTRIBUTING.md` for the discipline around verifying a claim before writing it down as fact (see the entries below marked RETRACTED/corrected for why that discipline exists).

Each entry: the fact, how it was found, and where it should eventually land in the docs.

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

**This explains (and revises) the very first product-feedback entry above** ("description field is not read into block or step creation"): it's not that the platform silently drops `function.json`'s `description` — it's that **this upload path doesn't consume `function.json` at all**. Naming and describing a step happens by hand, twice, in this wizard (once at the "Block Store details" step for the block as a whole, once per function at the "Configure" step) — separately from anything written into `function.json`. `function.json` appears to matter only for the CLI/CI-driven publish pipeline (the `@betty-blocks/cli`/GitHub Actions OCI-push flow seen in `native-wasm-components`/`block-store-wasm-components`), not for this interactive, human-driven upload path.

**Why it matters for both audiences:**
- For BB developers (documentation): if you're hand-uploading a component via the IDE rather than through an automated publish pipeline, don't bother writing a careful `function.json` description expecting it to show up — you'll re-type it here regardless. Worth being explicit about which upload path reads `function.json` and which doesn't.
- For product devs (feedback): having a `function.json.description` field that's silently unused on one of the two upload paths (and, per the earlier product-feedback entry, causes an opaque "Internal Server Error" in at least one case) is confusing for component authors — worth asking whether the two paths are meant to converge, and if so when.

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

**How found:** hit directly while building `create-record`, then isolated with a controlled test (see `product-feedback-log.md` for the full comparison and reasoning) — confirmed by renaming `create-record`'s world from `main` to `create-record-world`, rebuilding, and re-testing.

**Where this lands:** `WASM_Actions_Crash_Course.docx` quick-start checklist (section 10) and component-anatomy section (section 4) — recommend a distinctly-named world (e.g. matching the component's own name) as the default convention for any component with host imports, rather than the generic `main` used in the simplest pure-computation examples.

**Status:** confirmed directly, first-hand, via a single-variable test.

---

## ~~The platform auto-generates real Model/Property picker inputs directly from WIT types~~ — RETRACTED, it's a synthetic placeholder, not a real picker (2026-08-07, corrected 2026-08-10)

**Original (wrong) claim:** uploaded the official `store-file-base64` example (WIT params `model: model` and `property: list<property>`, using the shared `betty-blocks-utilities:types` records) as a step with no `function.json` involved. The generated option form rendered `model`/`property` as "Select variable" inputs labeled "Based on schema model ..." — this was read at the time as genuine, working native Model/Property picker UI generated straight from the WIT signature.

**Correction, 2026-08-10:** that read was wrong. Following the "Based on schema model ..." link this session (`/app/schema-models/<id>`) shows it's a literal, real, **read-only Data Model** the platform auto-creates in the app's own schema — named e.g. `model_Store File Base64_1`, with a single `name: Text` property mirroring the WIT record shape (`record model { name: string }`). It is **not** a reference to any real app Data Model, and there is no adaptive Property filtering by kind (`FILE`/`IMAGE`) as the official `function.json` specifies (`"allowedKinds": ["FILE","IMAGE"]`) — because this upload path never reads `function.json`, so that meta is simply never seen. The "Select variable" picker only ever offers variables matching this synthetic shape, which nothing in a real action can produce. Full writeup: `product-feedback-log.md`, "Open question: can third-party Wasm steps ever get real native Model/Property selectors?".

**Why the original test didn't catch this:** the step accepted the WIT types and rendered *some* option UI without error, and the label said "Model"/"schema model" — plausible-sounding, so it was taken as confirmation without following the link to see what it actually pointed at.

**Practical implication (superseded, see below):** there is currently no known way to get a genuine native Model/Property selector (backed by the app's real schema, adaptive Property list) on a third-party/Block Store component through **this upload path** — only the synthetic-schema-model fallback described above.

**Update, 2026-08-10 — the real fix is to use `bb functions publish` instead of the IDE wizard.** Confirmed live: published the same, byte-identical `store-file-base64` via the CLI (checksums verified against upstream) to the `marcel-tes` app. The resulting step's MODEL option rendered as a genuine "Select model" picker listing the app's real Data Models (Role, User, Webuser); selecting `Webuser`, PROPERTY correctly adapted and showed only `file` as selectable (every non-file/image property greyed out, matching `function.json`'s `allowedKinds: ["FILE","IMAGE"]`). Field labels/info tooltips also matched `function.json` exactly. **So: the synthetic-schema-model fallback is specific to the "Add custom Wasm step" IDE wizard (which never reads `function.json`, per the entry below) — it is not a platform-wide limitation.** For any option type whose UI depends on `function.json` meta (`Model`, `Property`, `Map`, `MultilineText`, etc.), publish via `bb functions publish` (see the CLI entry further below), not the interactive wizard. Full writeup: `product-feedback-log.md`'s resolved "third-party Wasm steps *can* get real native Model/Property selectors" entry.

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

**Update, 2026-08-10 — tested live, and the real-world result differs from what the endpoint name suggests.** Ran `bb functions publish` for real against the `marcel-tes` app (fresh `bb functions init --type wasm marcel-tes` project, `store-file-base64` duplicated in). Publish succeeded, `function.json` correctly produced real native Model/Property selectors (see `product-feedback-log.md`'s resolved entry) — but the resulting step showed up in the `marcel-tes` app's own step palette tagged **"Application specific"** (wrench icon), *not* under the Block Store's "My Blocks → Dev blocks" as a cross-app-installable block. So despite hitting the `/blocks/publish?type=wasm-function` endpoint (the same URL shape as ActionsJS's Block-Store-bound `bb blocks publish`), the actual behavior for WASM is app-scoped registration, matching ActionsJS's *other* command (`bb functions publish`, direct-to-app) in outcome even though it doesn't match it in endpoint. Revise the "WASM only ever has path (1)'s shape but targeting path (2)'s backend" claim above: it's closer to "path (1)'s *outcome*, via path (2)'s *URL*" — the endpoint name is not a reliable guide to where the block actually ends up. Whether WASM blocks can ever reach the shared, cross-app Block Store at all (the way ActionsJS's `bb blocks release` achieves) remains unconfirmed.

**Status:** confirmed directly from source, then confirmed live end-to-end, 2026-08-10.

---

## Reusing a Block Store component standalone just means resolving its workspace shortcuts to concrete values — nothing else needs to change; and `bb functions publish` can ask for the app UUID manually, contradicting an earlier entry (2026-08-19)

**Fact 1 — workspace-relative `Cargo.toml` fields are the *only* thing that needs adapting.** Copied `block-store-wasm-components/functions/generate-uuid/1.0/` (the canonical version — newer `wit-bindgen` 0.58.0, nested `bindings` module pattern, and a real `tests/mod.rs` integration test using `wasmtime-testing-helper` that instantiates the compiled component and calls it) verbatim into a fresh standalone `bb functions init --type wasm` project (`rug-test-ontwikkel`), with only `Cargo.toml`'s workspace shortcuts resolved to concrete values: `edition.workspace = true` → `edition = "2024"`, `wit-bindgen.workspace = true` → `wit-bindgen = "0.58.0"`, `wasmtime-testing-helper.workspace = true` → the same `git = "https://github.com/bettyblocks/wasmtime-testing-helper"` dependency spelled out directly, and `build = "../../../build.rs"` → `build = "build.rs"` with a local copy of the workspace-root `build.rs` placed inside the function's own folder (its `wkg wit fetch` logic is already path-independent). Everything else — `function.json`, `wit/world.wit`, `src/lib.rs`, `tests/mod.rs`, `wkg.lock` — copied byte-for-byte with zero edits. `cargo build --release --target wasm32-wasip2` and `cargo test` both succeeded first try, including the real `wasmtime-testing-helper` integration test. This confirms the fix already described in the "workspace-relative fields" entry above is genuinely sufficient on its own — no other adaptation was needed.

**Fact 2 (corrects the "resolved automatically" claim in the 2026-08-12 entry above) — `bb functions publish -a` can stop and ask for the app's production UUID by hand.** Running `bb functions publish -a` against the fresh `rug-test-ontwikkel` project validated the function successfully, then prompted interactively: `? Please provide the UUID for 'rug-test-ontwikkel' (production) ›` — it did **not** resolve the identifier to a UUID silently, unlike what the earlier entry described ("Nothing about typing or supplying that UUID yourself is required; it's resolved automatically from the identifier"). Not yet diagnosed *why* this project hit the manual-entry path — candidates include: this being the very first publish attempt for this identifier/CLI-session combination (no cached resolution yet), not being logged in yet at prompt time, or a genuine CLI behavior difference from whatever version/context produced the earlier "automatic" observation. Paused here rather than guess at a real production app's UUID; Marcel is running `bb functions publish -a` himself to supply it.

**Where this lands:** crash-course section on reusing a Block Store component in a standalone project — document the exact list of `Cargo.toml` fields to resolve (above) as the complete fix, and note the WIT/Rust/test files need no changes at all. Also flag in section 3.9 (the publish-flow write-up) that the "automatic UUID resolution" claim isn't reliable in every case — a manual UUID prompt can appear, and neither this nor the earlier entry has pinned down the exact trigger yet.

**Status:** Fact 1 confirmed directly, first-hand, 2026-08-19. Fact 2 observed directly, first-hand, 2026-08-19, but root cause not yet diagnosed — treat as an open correction, not a fully understood mechanism.

---
