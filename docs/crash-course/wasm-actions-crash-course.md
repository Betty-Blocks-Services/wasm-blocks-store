# WASM Actions on Betty Blocks

*A crash course for building and using custom WASM components*

This guide is written for people building applications on the Betty Blocks platform — low-code developers and business technologists — who want to understand and use WASM actions: converting existing actions to run as WASM, and building brand-new custom components for the Block Store. It's based on Betty Blocks' public component examples and platform behavior, plus the latest platform updates shared in product reviews, current as of August 2026.

------------------------------------------------------------------------

## 1. What is a WASM Action?

A WASM Action is a single reusable "step" that can be dragged into an action flow in the Betty Blocks IDE, exactly like the built-in steps (Create Record, Condition, Expression, etc.). Under the hood it is a small WebAssembly component, compiled from Rust (occasionally JavaScript for legacy "Native" functions), that exposes one or more functions through a WIT (WebAssembly Interface Type) interface.

Betty Blocks is investing in WASM actions to solve two problems at once:

- Extensibility — new low-code steps (connectors, utility functions, integrations) can be added to the platform's action palette without waiting on core platform releases.

- Portability & openness — WASM components run in a sandboxed runtime, so custom or partner-authored logic can execute safely alongside Betty Blocks' own steps, and — unlike the older ActionJS format — the underlying logic isn't locked into a proprietary format.

You'll see two flavours of WASM step in the platform:

- Native Wasm — first-party steps built and maintained by Betty Blocks (e.g. Reassign Variable, Authenticate User) that ship as part of the platform itself.

- Block Store Wasm — components published to the Block Store, by Betty Blocks or by partners/customers, that any application can install. This is the category you'll build in when you extend the ecosystem yourself.

A note on languages

The WebAssembly Component Model itself is language-agnostic — in principle a component can be authored in Rust, JavaScript/TypeScript, Go, C, and others, as long as the toolchain can compile it to a wasm32-wasip2 component with the right WIT interface. Betty Blocks' own first-party Native Wasm steps are a mix of Rust and JavaScript internally, so the platform clearly can run components built in more than one language.

That said, every publicly documented, customer-facing example — the full block-store-wasm-components repository this guide is built on, and every Block Store component in Betty Blocks' own tracking — is written in Rust. There is currently no documented, supported toolchain (equivalent to the Cargo workspace + wit-bindgen setup shown here) for authoring a Block Store component in Node.js or another language. This guide is written for Rust accordingly. If a JavaScript authoring path becomes officially supported, this guide should be extended with it — until then, treat Rust as the reliable, documented option, and check with your Betty Blocks contact if you want to explore building in another language.

## 2. Two ways to get a WASM action

There are two distinct paths to running WASM instead of the older ActionJS engine, and they solve different problems. Converting an action (below) moves something you've already built onto the newer execution model without touching its logic. Building a brand-new component (section 3) is how you add a step that doesn't exist yet — for your own applications or to publish for others.

### 2.1 Converting an existing action — a possible future option

Betty Blocks may, in the future, offer a way to convert an existing ActionJS action to run as WASM without rebuilding it from scratch. This isn't available today — if you're deciding whether to build a new step as WASM now versus waiting on a conversion path, check with your Betty Blocks contact for the current status.

The underlying motivation would be the same reason to build new components as WASM in the first place: WASM actions are meant to be more open and portable than ActionJS ones, which matters if you want to inspect or run your action logic outside Betty Blocks, or want to get ahead of an eventual platform-wide move away from ActionJS. It isn't primarily a reliability fix — ActionJS's own long-standing memory-usage, crash, and file-size issues have already been addressed independently of any conversion tooling.

### 2.2 Building a brand-new custom component

If the step you need doesn't exist yet — a connector to an external service, a utility function, a piece of logic you want to reuse across applications or publish to the Block Store — you build it as a WASM component from scratch. That's the focus of the rest of this guide, starting with section 3.

## 3. Step-by-step: build and test your first custom component, start to finish

This walkthrough builds one real, working component — generate-random-hex, a step that produces a random string of hexadecimal characters at a length you choose — from nothing, all the way to a step that's live and working in a real application. It uses the Betty Blocks CLI (bb functions) rather than the interactive upload wizard; section 3.11 at the end covers how that other path differs.

A note before you start typing: almost every command in this walkthrough only reads or writes files inside one project folder on your own computer — nothing here deletes anything or reaches outside that folder. The one exception is publishing (section 3.9), which does send things to a real application over the network — it's clearly called out when we get there. Wherever a command's effect isn't obvious from what it's called, this guide explains what it actually does before asking you to run it.

### 3.1 Before you start

You need three things installed. Check each one before assuming you need to install it — you may already have some of these.

- Rust. Check whether you already have it by running rustc --version in a terminal. If that prints a version number, you already have Rust — you can skip the install command below and go straight to adding the wasm32-wasip2 target a little further down. If it says something like "command not found," install it using the official installer, by running:

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

This downloads and runs Rust's own official install script (the same one linked from rust-lang.org). It will explain what it's about to do and ask you to confirm before installing anything — read its prompts rather than blindly accepting them. It installs three things: rustc (the Rust compiler), cargo (Rust's package manager and build tool — you'll use this a lot below), and rustup (a version manager for Rust itself, which is what the next command uses).

Whether you just installed Rust or already had it, add the specific target this guide needs next:

```bash
rustup target add wasm32-wasip2
```

This tells your Rust installation how to compile to WebAssembly instead of to your computer's own processor — wasm32-wasip2 is the specific WebAssembly flavor Betty Blocks' platform expects. If a terminal says cargo or rustc isn't found even though you just installed Rust, close and reopen your terminal first (a fresh terminal window picks up the new installation); if it still isn't found, check whether `~/.cargo/bin` is on your shell's PATH — run `export PATH="$HOME/.cargo/bin:$PATH"` and try again.

- The Betty Blocks CLI (@betty-blocks/cli, on npm), which gives you the bb command used throughout this walkthrough. Check whether you already have it by running bb --version. If you don't already have Node.js and npm installed, install those first from nodejs.org. Then, whether you already have the CLI or not, running `npm install -g @betty-blocks/cli` is safe either way — it installs it fresh if you don't have it yet, or updates it to the latest version if you already do.

- A Betty Blocks application to publish into — any application you already have access to in the Betty Blocks IDE will do. You'll need to know its identifier, which is the subdomain in the app's URL (for example, if your app opens at my.bettyblocks.com/marcel-tes, its identifier is marcel-tes).

### 3.2 Create the project

If you're starting completely fresh, with no functions project yet, let the CLI create one for you:

```bash
bb functions init --type wasm <your-app-identifier>
```

Replace `<your-app-identifier>` with your actual app's identifier from 3.1 — this is what ties everything you build in this project to that one specific application. This command creates a new folder (named after your identifier) containing:

- A .wasm-functions file — this file is empty on purpose. It's just a marker: the CLI looks for it to recognize "this folder holds WASM-style functions," as opposed to Betty Blocks' older, different kind of custom-function project (used by its older action engine, ActionJS), which uses a differently-named marker instead.

- A functions/ folder, containing one working example function called say-hello, so you have something real to test the rest of this walkthrough's commands against before touching your own code.

You don't need that say-hello example — you can delete the whole functions/say-hello/ folder right now if you'd rather start clean, and nothing else in this guide depends on it. This walkthrough leaves it in place on purpose, though: it means the next step shows you exactly what it looks like to add a second component to a project that already has one in it — which is what you'll actually be doing most of the time once you're past your very first component.

Now, from inside that project folder, add your real component. Instead of creating its folder and files by hand, let the CLI scaffold it for you, then move into that new folder — from here on, unless a step explicitly says otherwise, every command in this walkthrough is run from inside it:

```bash
cd <your-app-identifier>
bb functions new generate-random-hex
cd functions/generate-random-hex/1.0
```

This creates functions/generate-random-hex/1.0/ with five files already in it, each with a placeholder example inside (not empty):

- Cargo.toml — the Rust project's manifest: its name, version, and which external code libraries ("crates," in Rust terminology) it depends on.

- wit/world.wit — the interface: a plain-text contract, written in a small language called WIT (WebAssembly Interface Type), that states exactly what function this component exposes, what input(s) it takes, and what it returns. This is what the Betty Blocks platform reads to know your step exists at all and what shape its inputs/outputs are — before it ever looks at a single line of your Rust code.

- src/lib.rs — the actual Rust code that does the work.

- function.json — describes how the step should look and behave inside the Betty Blocks IDE: its label, icon, category, and the input/output fields shown in its configuration form.

- Justfile — optional shortcut commands for building and testing this component; more on this in section 3.8, since you likely don't have the tool it needs installed yet.

The rest of this walkthrough is editing the first four of those files, one at a time, into the real generate-random-hex component.

### 3.3 Edit the interface — wit/world.wit

As a reminder from 3.2: this file is the interface, and its only job is to describe your step's shape (its function name, inputs, and output) — it doesn't contain any actual logic. Open the generated wit/world.wit. It currently describes the placeholder say-hello-shaped function; replace its entire contents with:

```wit
package betty-blocks:generate-random-hex@1.0.0;

interface generate-random-hex {
generate-random-hex: func(size: u32) -> string;
}

world main {
export generate-random-hex;
}
```

This declares one exported function, generate-random-hex, taking one input named size — a u32, meaning a whole number that's never negative — and returning a string (plain text). package/interface/world are WIT's own structural keywords: package names and versions the whole file, interface groups related functions together, and world declares what this component exports (or, for more advanced components, also imports — see section 5.3 for an example that does).

### 3.4 Edit the implementation — src/lib.rs

This file's job is different from the last one: where wit/world.wit only describes the shape of your step, src/lib.rs is where the actual logic lives — the real code that runs when someone uses this step. Open the generated src/lib.rs and replace its placeholder logic with:

```rust
use crate::exports::betty_blocks::generate_random_hex::generate_random_hex::Guest;
use rand::RngExt;

wit_bindgen::generate!({ generate_all });

struct Component;

impl Guest for Component {
fn generate_random_hex(size: u32) -> String {
let mut random_number_generator = rand::rng();
(0..size).map(|_| format!("{:X}", random_number_generator.random_range(0..16))).collect()
}
}

export! {Component}
```

Some of this is boilerplate you'll write almost identically in every component you ever build; the rest is specific to generating a random hex string. Worth telling the two apart, since it tells you what you can copy-paste unchanged next time versus what you'll actually need to rewrite:

- Boilerplate — the same shape in every component: use crate::exports::...::Guest (brings in the Guest trait — a "trait" in Rust is a set of functions something must implement; this one was generated automatically from your wit/world.wit by the next line, and it requires exactly one function per the interface you wrote). wit_bindgen::generate!({ generate_all }); (runs at compile time, reads wit/world.wit, and writes the matching Rust plumbing for you — you never see that generated code directly, but without this line nothing above it would be valid). struct Component; (declares a new, empty named type; "struct" is how Rust defines a new kind of thing — it's empty because this component doesn't need to remember anything between calls, it's really just a label to attach our code to). impl Guest for Component { ... } (this is how Rust says "Component provides the implementation of the Guest trait" — the wrapper every component's real logic sits inside). export! {Component} (registers Component as the real implementation WASM should call into — without this line, nothing above would ever run).

- Specific to this component: use rand::RngExt (brings in the one piece of the rand crate this particular component needs, for generating randomness — a component that didn't need randomness wouldn't have this line at all). The function signature, `fn generate_random_hex(size: u32) -> String`, matches whatever you declared in your own wit/world.wit. And the body is the actual unique logic, explained below.

The function body, in four parts:

- let mut random_number_generator = rand::rng(); — creates a new random-number generator and stores it in a variable. mut means this variable is allowed to change as it's used (a plain let, without mut, would lock it as read-only).

- 0..size — a sequence of whole numbers from 0 up to (but not including) size. In effect: "repeat the next part exactly size times."

- `.map(|_| format!("{:X}", random_number_generator.random_range(0..16)))` — runs this once per number in that sequence. The `|_|` means we don't care about the number itself, only that we're repeating. random_number_generator.random_range(0..16) picks a random whole number from 0 to 15 — one hexadecimal digit's worth. format!("{:X}", ...) converts that number into a single uppercase hex character (0–9 or A–F).

- .collect() — glues all of those one-character strings together into the one final String this function returns.

Optional, but recommended: add a quick test at the bottom of the same file, so you can check the logic without a WASM build yet. For a component this simple it's worth the extra minute; for something more involved, writing a genuinely useful test can be a lot harder than the component itself, so don't feel obliged if it's slowing you down — you can skip straight to section 3.7 without one.

```rust
#[test]
fn produces_the_requested_length() {
let random_hex_value = Component::generate_random_hex(32);
assert_eq!(random_hex_value.len(), 32);
}
```

`#[test]` marks the function below it as a test rather than real program logic; assert_eq! fails the test loudly if the two values it's given don't match — here, checking that asking for 32 characters actually produces 32 characters.

### 3.5 Add the rand dependency — Cargo.toml

This file's job: it's Rust's project manifest, listing your component's name, version, and every external code library ("crate") it needs — nothing here is executable logic, it's all configuration. The generated Cargo.toml only lists wit-bindgen, which every scaffolded component needs. Open it and make it read:

```toml
[package]
name = "generate_random_hex"
version = "1.0.0"
edition = "2024"

[lib]
crate-type = ["cdylib"]

[dependencies]
wit-bindgen = "0.42.0"
rand = "0.10.1"
```

Going through each section:

- `[package]` name/version — the project's own name and version number; the CLI fills these in for you and you generally don't need to touch them.

- edition — this is the one line worth being careful with. It is NOT the current calendar year, even though the value looks like one. "Edition" is Rust's own name for a small, fixed set of language versions it supports — as of this writing, that set is 2015, 2018, 2021, and 2024, and no others exist. Typing in an actual future year (2026, say) is not a valid edition and will fail to build with a confusing-looking error. Always use exactly what bb functions new already generated for you here (2024) unless you have a specific reason to change it.

- `[lib] crate-type = ["cdylib"]` — tells Rust to compile this into the specific library shape that can later be turned into a WebAssembly component. Leave this exactly as generated.

- `[dependencies]` — the external crates your code needs. wit-bindgen was already there; rand is the one new line you're adding here, since src/lib.rs (section 3.4) now calls into it for randomness.

### 3.6 Describe the step — function.json

This file's job is different again from the previous three: it doesn't affect what your component computes at all — it controls what the Betty Blocks IDE actually shows for this step. That's the label and icon someone sees while searching the step palette, and — importantly — the real, editable input and output fields (like Size and Result below) that appear in the step's configuration form once it's dragged onto a canvas. The generated function.json is mostly an empty shell: a label guessed from the function's name, and no options yet. Replace its contents with:

```json
{
"category": "String Functions",
"description": "Generates a string of random hexadecimal characters with a given size",
"icon": { "color": "Blue", "name": "QuestionIcon" },
"label": "Random Hex",
"options": [
{
"meta": { "type": "Number", "default": 8, "validations": { "required": true, "min": 1 } },
"name": "size",
"label": "Size",
"info": "The length of the result string"
},
{
"meta": { "type": "Output", "output": { "type": "Text" } },
"name": "result",
"label": "Result",
"info": "The result as the concatenated string"
}
],
"yields": "NONE"
}
```

label, description, category, and icon control how the step looks in the IDE's step palette, before anyone has even added it to an action. The two entries under options control the configuration form shown after it's dragged onto a canvas: the size entry (type Number, with a default and a minimum) becomes a real, editable input field — this is what lets whoever uses this step type in how many characters they want, rather than the length being fixed in your code. The result entry declares the step's output. See section 5.4 for the full reference of what other option types (Text, Model, Property, Map, and so on) are available for other kinds of inputs.

### 3.7 Run the tests

Skip this step entirely if you skipped adding the test in 3.4. Otherwise, from inside functions/generate-random-hex/1.0 (where you already are, from 3.2), run:

```bash
cargo test
```

This runs the `#[test]` you added in 3.4 as ordinary Rust, compiled for your own computer — no WASM involved yet — so it's the fastest way to catch a logic mistake before spending time on a real build.

### 3.8 Build the real component

cargo test in 3.7 did compile your code, but only enough to run on your own computer and execute the test — it's a completely separate build from the one you need here, which specifically targets WebAssembly instead. That's the actual reason this next build is worth running as its own step even though "compiling" already just happened once.

The generated Justfile (section 3.2) is a convenience file for a separate tool called just — think of it like npm's package.json scripts section, if you've used that before: a way to give short names to longer commands. just is not installed automatically, and you probably don't have it — this guide didn't either, until it was installed on purpose (on a Mac, that's brew install just). If you don't have it and don't want to install it, that's completely fine: skip straight to the three commands below, which are exactly what just build would have run for you anyway. Still inside functions/generate-random-hex/1.0, run:

```bash
wkg wit fetch
cargo build --release --target wasm32-wasip2
mv ./target/wasm32-wasip2/release/*.wasm .
```

wkg wit fetch pulls in any WIT dependencies your component imports from the host — this component imports nothing from the host, so this is a no-op here, but it's always safe to run and worth doing out of habit for components that do need it. cargo build --release --target wasm32-wasip2 is the real compile step — unlike cargo test, this one specifically targets wasm32-wasip2 (WebAssembly) rather than your own computer, and builds in optimized "release" mode rather than the quick debug mode cargo test uses. That combination is what actually produces a .wasm file, and is why this is the slowest command you've run so far (it may take anywhere from a few seconds to a minute or two, mostly the first time). The mv command just moves the resulting file from deep inside Rust's own build output folder (target/) up to sit next to function.json, which is where publishing (section 3.9) expects to find it.

It's worth double-checking that what you just built is a genuine WebAssembly component and not something subtly wrong. Run:

```bash
od -A x -t x1 generate_random_hex.wasm | head -1
```

This is entirely safe to run — od just prints out a file's raw bytes for you to look at; it can't change or delete anything. -A x -t x1 tells it to show byte offsets and each byte in hexadecimal, and head -1 keeps just the first line, since that's all we need. The first bytes should read 00 61 73 6d 0d 00 01 00 — the first four are the fixed `\0asm` signature every WebAssembly file starts with, and the next four (0d 00 01 00) specifically mean "this is a component," the packaging format the Betty Blocks platform expects. If you saw 01 00 00 00 instead in that second group, something compiled to a plain "core module" rather than a component — worth knowing the difference exists, even though everything in this walkthrough is set up to avoid it.

### 3.9 Publish it

This is the one step in this whole walkthrough that reaches outside your own computer: it sends your component to a real Betty Blocks application over the network, and registers it there as an actual step other people using that application could see and use.

First, move back up to the project's root folder — the one containing .wasm-functions, two levels above where you've been working:

```bash
cd ../../..
```

Then run:

```bash
bb functions publish
```

Running it plain like this walks you through several distinct stages, in order:

- Validation — it first checks every function folder inside functions/ (in this walkthrough, both say-hello and generate-random-hex, if you kept say-hello) and confirms each one is well-formed before anything is sent anywhere.

- Choosing what to publish — you're then asked which of the valid functions you actually want to publish, as an interactive checklist: use the arrow keys to move, the spacebar to select or deselect each one (the circle fills in), and Enter to confirm your selection. It looks like this:

```text
Validating functions in <your project path>
✓ Validate: generateRandomHex-1.0
✓ Validate: generateUuid-1.0

✓ All your functions are valid and ready to be published!
? Which wasm functions do you want to publish? ›
○ generate-random-hex/1.0
○ generate-uuid/1.0
```

If you'd rather skip this checklist and publish every valid function without being asked, run bb functions publish --all (or -a) instead.

- Resolving your application — next, the CLI looks up the actual application behind the identifier you gave at bb functions init (section 3.2). Worth knowing these are two different things: the identifier is just the friendly subdomain name you chose or already had; behind it, Betty Blocks' own systems track that application by its own internal UUID (a long, unique ID, unrelated to the identifier's spelling). Usually the CLI resolves this UUID automatically from the identifier with no input from you — but this isn't guaranteed: it's been observed asking directly, with a prompt like `? Please provide the UUID for '<your-identifier>' (production) ›`, and the exact trigger for when it asks versus resolves silently isn't pinned down yet. If you're asked and don't know your application's UUID, you can find it in the Betty Blocks IDE (it's part of several of the app's own URLs) rather than guessing. If the resolution step fails outright instead, it usually means the identifier doesn't match a real application you have access to.

- Logging in — if you aren't already logged in (or your session has expired), you'll be prompted to log in with the same email/password or SSO you'd use to open the Betty Blocks IDE itself in a browser.

- Publishing — only now does it actually send the selected function(s) to Betty Blocks. On success you'll see a line per function, like:

```text
✔ Published generate-random-hex/1.0
```

### 3.10 Confirm it actually works

Open the application in the Betty Blocks IDE, open (or create) an action, and search the Wasm steps palette for “Random Hex” — the label you gave it in function.json (section 3.6). It'll be tagged with a wrench icon and “Application specific.” Drag it onto the canvas: this is the real test. A component compiling and publishing cleanly doesn't guarantee it can actually be added to a flow — that's a separate, later point of failure (see section 9's known issues). If it drops onto the canvas and opens for configuration — with a real, editable Size field, since you gave it type Number in function.json — with no error, you have a tested, working custom step.

### 3.11 The other path: the interactive wizard

This walkthrough uses bb functions publish, which reads and sends function.json — the option types in section 5.4 (Model, Property, Map, and so on) only render correctly through this path. The alternative, the in-IDE “Add custom Wasm step” upload wizard, only accepts a raw .wasm file: it never reads function.json at all, builds its option form purely from the WIT interface, and asks you to name and describe each function by hand instead. Publishing a component as an installable Block Store block through that wizard, and step creation through it, is a distinct workflow — covered separately from this walkthrough.

Everything from here to section 10 is reference material, not a required prerequisite — you've just built and tested a real component without needing any of it. Come back to these sections as they become relevant: for the fuller architectural picture, other example components to copy from, the complete function.json option-type table, platform quirks to watch for, and where WASM actions are headed next.

## 4. How the pieces fit together

A handful of platform pieces participate in the lifecycle of a WASM action. Understanding the boundaries between them makes the rest of this guide easier to follow.

  ------------------------------- -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------
  **Component**                   **Role**

  Block Store                     The catalog of installable components. Accepts an upload of a .wasm file + wit interface + function.json, stores it, and serves it to applications that install the block.

  Platform backend                Receives an uploaded component and its function.json, validates it, and stores the metadata so the IDE can offer it as a step.

  IDE (Palette + Action Editor)   Lists installed custom WASM steps in the action step palette, renders the option form (Model, Property, Text, Value, Map, Output fields, etc.) from function.json, and lets you drag the step into a flow.

  Actions Compiler                Combines every WASM function used inside an application's action flow into one composed, deployable component, alongside the application's own compiled action logic.

  Execution runtime               Runs the compiled action at request time in a sandboxed environment, and satisfies any host capability a component asks for (outgoing HTTP calls, reading/writing application data, uploading files, randomness, etc.).
  ------------------------------- -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------

End-to-end flow

- 1. You author a Rust WASM component: a WIT interface describing its exported function(s), a function.json describing the IDE step UI, and (if it's a Block Store component) a project laid out as `functions/<name>/<version>/`.

- 2. You build it to wasm32-wasip2 and upload the .wasm + wit + function.json to the Block Store, either through the upload UI or an automated publishing pipeline.

- 3. The platform backend receives the uploaded component, reads function.json, and stores it so it can be attached to an application.

- 4. The new step appears in the action palette. You drag it into a flow and fill in the option form generated from function.json — the upload form itself shows a live preview of how the step will look.

- 5. When you compile your application, the Actions Compiler fetches the referenced wasm functions from the Block Store and combines them into the deployable component.

- 6. At runtime, the execution environment runs the composed component. Any host capability the component asks for (outgoing HTTP, reading/writing application data, file upload, randomness) is provided by Betty Blocks.

- 7. Later lifecycle events — publishing a new version of a component, or uninstalling one — flow back through the same Block Store pipeline, and are reflected in the IDE (see section 8).

## 5. Anatomy of a WASM component

The clearest way to learn the pattern is to read the block-store-wasm-components repository (github.com/bettyblocks/block-store-wasm-components), which is a Cargo workspace of small, independent example components. Every component lives at `functions/<name>/<version>/` and contains:

  ---------------------------- ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------
  **File**                     **Purpose**

  Cargo.toml                   Crate manifest. `crate-type = ["cdylib"]`, depends on wit-bindgen (and wasmtime-testing-helper for dev/test).

  wit/world.wit                The WIT interface: what the component exports (and imports, if it needs host capabilities).

  src/lib.rs                   The Rust implementation. Uses wit_bindgen::generate!({ generate_all }) to generate bindings, then implements the generated Guest trait.

  function.json                The IDE-facing metadata: label, description, category, icon, and the list of input/output "options" that build the step's configuration form. Not present for pure helper functions like concat-text/split-text that aren't standalone IDE steps.

  `external/*.wit` (optional)   Copies of host-provided WIT packages a component imports from (e.g. betty-blocks-utilities:data-api), pinned via wkg.toml overrides.

  tests/mod.rs                 Component-level tests using wasmtime-testing-helper, which instantiates the compiled .wasm and calls its exported interface directly (as opposed to unit tests inside src/lib.rs that test plain Rust logic).
  ---------------------------- ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------

### 5.1 The simplest possible component — concat-text

concat-text exports four pure string functions and imports nothing from the host. This is the template to copy for any stateless utility function.

```wit
package betty-blocks:concat-text@1.0.0;

interface concat-text {
concat-text: func(first-text: string, second-text: string) -> string;
concat-text-with-separator: func(first-text: string, second-text: string, separator: string) -> string;
concat-text-list: func(text-list: list<string>) -> string;
concat-text-list-with-separator: func(text-list: list<string>, separator: string) -> string;
}

world main {
export concat-text;
}
```

The Rust side wires the generated bindings to a struct and implements the Guest trait:

```rust
mod bindings {
wit_bindgen::generate!({ generate_all });
use crate::ConcatText;
export!(ConcatText);
}

use crate::bindings::exports::betty_blocks::concat_text::concat_text::Guest;

struct ConcatText;

impl Guest for ConcatText {
fn concat_text(first_text: String, second_text: String) -> String {
format!("{}{}", first_text, second_text)
}
// ...
}
```

Note: concat-text and split-text have no function.json — they're internal building blocks, not directly exposed as an IDE step. generate-uuid, generate-random-hex, liquid-template, redirect-url, format-endpoint-result and store-file-base64 all have a function.json and do appear as steps.

### 5.2 A component with a real IDE form — generate-uuid

function.json drives the option form the builder sees in the IDE. Here the step has a single Output option:

```json
{
"category": "Misc",
"description": "Generate a randomized UUIDv4",
"icon": { "color": "Orange", "name": "ActionsIcon" },
"label": "Generate UUID",
"options": [
{
"meta": { "type": "Output", "output": { "type": "Text" } },
"name": "result",
"label": "Result",
"info": "Generate UUIDv4"
}
],
"yields": "NONE"
}
```

### 5.3 Calling back into the platform — store-file-base64

Most real integrations need to do more than pure computation — they need to call the platform (to look up a model/property, to upload a file) or the outside world (outgoing HTTP). Components declare these as WIT imports, and the host (wasmCloud, wired up by Betty Blocks) satisfies them at runtime. store-file-base64 is the best worked example of this pattern:

```wit
world store-file-base64 {
import wasi:http/outgoing-handler@0.2.0;
import wasi:random/random@0.2.0;
import betty-blocks-utilities:data-api/data-api;
import betty-blocks-utilities:upload-file/upload-file;
import betty-blocks-utilities:types/types;

export store-base64;
}
```

The `betty-blocks-utilities:*` packages are Betty Blocks' own host interfaces (not part of standard WASI). Their WIT source is vendored locally under `external/*.wit` and pinned in wkg.toml so the build is reproducible:

- data-api — a request(helper-context, query, variables) function that lets a component fire a GraphQL-style query against the application's data API. helper-context carries application-id, action-id, log-id, and auth (jwt / encrypted-configurations).

- upload-file — an upload(helper-context, input) function for pushing bytes into the application's asset store and getting back a reference.

- types — shared record types (model, property, property-path, property-mapping) used across the platform-facing interfaces.

On the function.json side, this is also the first example that uses input option types tied to the application's data model (Model and Property, with a dependsOn/CLEAR relationship so changing the model clears the previously chosen property) rather than plain scalar inputs.

### 5.4 function.json option types reference

Across the example components, the option "meta.type" values in use are:

  --------------- ----------------------------------------- --------------------------------------------------------------------------------------------------------------------------------------------
  **type**        **Seen in**                               **Notes**

  Text            store-file-base64, redirect-url           Plain single-line string input.

  MultilineText   liquid-template                           Multi-line string input (e.g. a template body).

  Value           liquid-template, format-endpoint-result   A value picker that can resolve to a variable/expression; allowedKinds constrains which variable kinds are selectable (e.g. `["STRING"]`).

  Map             liquid-template, format-endpoint-result   Key/value pairs (e.g. template context, HTTP headers).

  Number          format-endpoint-result                    Numeric input, supports a default.

  Model           store-file-base64                         Lets the builder pick one of the application's data models.

  Property        store-file-base64                         Lets the builder pick a property on the previously selected model; allowedKinds restricts to e.g. FILE/IMAGE.

  Output          all step-producing components             Declares the step's output variable; output.type is Text or Object depending on shape.
  --------------- ----------------------------------------- --------------------------------------------------------------------------------------------------------------------------------------------

Other function.json fields worth knowing: category groups the step in the palette (Misc, Templates, Web Flow, Endpoint, ...); icon.name/color set its palette icon; yields (seen as "NONE" throughout the examples) is reserved for steps that branch/yield control flow; configuration.dependsOn lets one option clear or otherwise react to another option changing (used by Property depending on Model).

Auto-generated configuration forms

Everything above describes hand-writing function.json yourself, which remains the reliable way to build a step's form today. Betty Blocks has also started generating this form automatically: once you push a component to the Block Store, the platform reads your WIT interface and builds the configuration form for you, for a growing set of supported option types. As of this writing that covers Betty Blocks model and property selectors, value-mapping options, and the create/update/delete grid operations — coverage is expected to expand over time.

For option types outside that supported set, you'll still need to hand-author function.json as shown in this section. And if a step's option panel comes up empty, or you hit a "no matching output type" error after pushing a component, that's a known rough edge in the automatic generator rather than necessarily a mistake in your WIT file — falling back to a hand-written function.json is the safe option today if you run into it.

## 6. Build & test tooling

The example repo is a single Cargo workspace (`members = ["functions/*/*"]`) so every component builds together. Useful commands, from the Justfile:

```bash
just build # cargo build --release --target wasm32-wasip2, then copies each
# .wasm into its own functions/<name>/<version>/ folder
just test # build, then cargo test (unit + wasmtime-testing-helper tests)
just format # cargo fmt
just format-check # cargo fmt --check
just quality-check # cargo clippy --all-targets -D warnings
just clean # cargo clean
```

build.rs runs wkg wit fetch before every build to pull WIT dependencies (including Betty Blocks' own host interfaces) — install the wkg CLI locally or the build will emit a warning and fall back to whatever's already cached.

Testing at two levels

- Plain Rust unit tests inside src/lib.rs (e.g. concat-text's `#[test]` fns) — fastest, test pure logic directly, no wasm involved.

- Component tests in tests/mod.rs via wasmtime-testing-helper — these instantiate the actual compiled .wasm in a Wasmtime store and call the exported WIT interface, so they exercise the real component boundary (bindgen!("main"), setup!(Main), then `component.component.<generated_interface>().call_<fn>(...)`).

- Integration tests in Deno/TypeScript (deno task test) using `@bytecodealliance/jco-transpile` and preview2-shim — these transpile the wasm component to JS and run it under a WASI preview2 shim, useful for testing from a JS-consumer's perspective.

Property-based tests (proptest) show up for components like split-text/store-file-base64 where you want broad input coverage rather than a handful of hand-picked cases.

## 7. Packaging, versioning, and publishing

Components are versioned by directory (`functions/<name>/1.0/`, .../2.0/, ...) and by the `@x.y.z` suffix on the WIT package declaration — both must match. An automated pipeline can build every component and publish each one as an OCI artifact to a component registry (GitHub Container Registry and/or an Azure Container Registry, depending on setup), which is what the Block Store and Actions Compiler pull from at runtime.

For a component you're uploading directly through the Block Store's upload UI (rather than through an automated pipeline), the same three artifacts are required: the compiled .wasm, its wit/world.wit (plus any `external/*.wit` it depends on), and function.json.

## 8. Managing components over time

Once a component exists, it goes through the same handful of stages regardless of whether you built it yourself or installed one someone else published:

- Uploading — a new component is submitted to the Block Store with its .wasm, WIT interface, and function.json.

- Becoming usable — once accepted, the component's function(s) appear as a step in the action palette, with a configuration form built from function.json (or auto-generated — see section 5.4).

- Compiling — when you compile an application that uses the step, the Actions Compiler resolves and pulls in the referenced component.

- Publishing a new version — a component author can push an updated version; applications already using an older version keep running it until you explicitly update to the new one.

- Uninstalling — removing a component from an application removes its step from that application's palette; this is a separate action from deleting the component from the Block Store entirely.

Updating and uninstalling are both surfaced directly in the IDE, so you don't need to go back to the Block Store to manage a component you've already installed.

## 9. Known issues & things to watch for

A few practical, platform-level things worth knowing before you build and ship WASM actions:

- Compiling takes longer. WASM actions currently compile noticeably slower than the older ActionJS format — if you're generating or changing several actions in a row, expect the wait for each compile to be longer once WASM is involved.

- Avoid special characters in names. Component and block names with unescaped special characters have been known to crash the Actions Compiler — stick to plain alphanumeric names with hyphens or underscores.

- "Action could not be found" after a successful compile. This has been reported even when compilation itself succeeded — if you hit it, try recompiling before assuming your action is broken.

- There's a runtime execution ceiling of roughly 55 seconds. A WASM step that calls a slow external service can hit a timeout around this mark — build your own timeout/retry handling for slow external calls rather than relying on the platform's ceiling.

- Assigning an empty collection to a relation from a WASM action can trigger a server error — a known limitation rather than a mistake in your action design.

- The automatic configuration-form generation (section 5.4) can occasionally throw a "no matching output type" error — fall back to a hand-written function.json if you hit this.

- Tuples aren't supported as a WIT return type today — use a record, or return multiple values via separate calls (see split-text's commented-out split_once for an example of this limitation).

On the positive side: the execution runtime has recently had reliability work done (memory handling and cleanup improvements), and the older ActionJS engine's own long-standing memory, crash, and file-size issues have also been resolved.

## 10. Where this is headed

Betty Blocks continues to invest in WASM actions as the primary way to grow the platform's action palette — both with its own components and ones built by partners and customers. Two directions are worth keeping an eye on, though neither has a committed timeline or is available today:

- Broader options for pulling components from sources other than Betty Blocks' own Block Store, useful if you want to manage your own catalog of components.

- An easier way to bring an existing ActionJS action onto the WASM model without rebuilding it — see section 2.1.

If you're planning a component for your own or a customer's application, checking with your Betty Blocks contact on current plans before investing significant build time is worthwhile, since specifics here can change.

## 11. Quick-start checklist: building your first custom component

- 1. Install: Rust + rustup target add wasm32-wasip2, the wkg CLI, and cargo, or clone the block-store-wasm-components repo and reuse its toolchain/CI setup as a starting template.

- 2. Scaffold the component: bb functions new `<your-component>` (or bb functions init --type wasm `<identifier>` if this is your first one) — generates Cargo.toml, wit/world.wit, src/lib.rs, function.json, and a Justfile for you, pre-filled with a placeholder example to edit. See section 3 for the full walkthrough.

- 3. Define the WIT interface first. Keep it to primitive types the compiler is known to support (string, `list<T>`, `option<T>`, record, `result<T, E>`); tuples are explicitly unsupported today (see split-text's commented-out split_once).

- 4. Implement the Guest trait generated by wit_bindgen::generate!({ generate_all }); export!(YourStruct) inside a bindings module.

- 5. Write function.json describing the IDE-facing step: label, description, category, icon, and one options entry per input/output using the type table in section 5.4 — or push a first draft and see what the automatic form generator produces for you.

- 6. Test at both levels: plain `#[test]` unit tests for pure logic, and a tests/mod.rs using wasmtime-testing-helper to exercise the compiled component through its real WIT interface.

- 7. Build with just build (or cargo build --release --target wasm32-wasip2 + manual copy) and confirm the .wasm lands next to function.json.

- 8. Publish with bb functions publish (see section 3), which reads function.json and registers the step directly in your application — or use the IDE's interactive upload wizard for a one-off component that doesn't need function.json-driven option types (see section 3.11).

- 9. Drag the new step into an action flow in the IDE, fill in the generated form, and compile the application to confirm it all works end-to-end.

- 10. Keep an eye on the known issues in section 9 while you iterate — several show up as confusing platform-side errors rather than validation errors in your own code.

## 12. A note on sources

This crash course is based entirely on Betty Blocks' public GitHub repositories: the [block-store-wasm-components](https://github.com/bettyblocks/block-store-wasm-components) example components, and the [@betty-blocks/cli](https://github.com/bettyblocks/cli) tooling (the bb command) used to build, test, and publish them throughout section 3. If something in here seems to need more detail than it has, or you hit platform behavior not covered in this guide, that's a good signal to check with your Betty Blocks contact directly.
