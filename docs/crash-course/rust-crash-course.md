# Rust Crash Course for Betty Blocks WASM Actions

*Just enough Rust to read, write, and debug a custom WASM component*

This is a companion to "WASM Actions on Betty Blocks." It doesn't try to teach Rust in general — it teaches the slice of Rust that actually shows up in Betty Blocks' example WASM components (concat-text, generate-uuid, generate-random-hex, split-text, liquid-template, redirect-url, store-file-base64), so you can read them line by line, then adapt them into your own component. Where something is a general Rust concept worth knowing more deeply, we point you to the free official Rust Book rather than reproducing it here.

------------------------------------------------------------------------

## 1. Before you start: setup

This is covered in the main crash course's build tooling section, repeated briefly here since you'll want it open while following along:

```bash
# Install Rust (once)
curl https://sh.rustup.rs -sSf | sh

# Add the WASM target components compile to
rustup target add wasm32-wasip2

# Confirm
cargo --version
rustc --version
```

Everything below assumes you have a component folder open with a `Cargo.toml` and `src/lib.rs` in it (see the main guide's section 4 for the exact layout), and that you're building with `cargo build --release --target wasm32-wasip2` or just `build`.

## 2. The shape of a Rust project

A Rust project (a "crate") is described by `Cargo.toml` — the manifest — and its code lives under `src/`. For a WASM component you'll always see:

  ---------------- ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------
  **File**         **What it's for**

  `Cargo.toml`      Declares the crate's name, version, dependencies, and `crate-type = ["cdylib"]` (tells Rust to build a library other things can load, which is what a wasm component needs).

  `src/lib.rs`      The actual code — this is a library crate (`lib.rs`), not a program with a `main()` function. There's no `main()` in these components; the platform calls specific exported functions instead.

  `tests/mod.rs`    A separate test file, run with `cargo test`, for tests that exercise the compiled component rather than plain Rust logic.
  ---------------- ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------

Every statement in Rust ends with a semicolon `;` — this trips people up coming from languages where it's optional. The one common exception: the last expression in a function or block, with no semicolon, is that function's return value.

```rust
fn add_one(x: i32) -> i32 {
x + 1 // no semicolon: this is the returned value
}
```

## 3. Variables and basic types

- `let x = 5;` declares a variable. Variables are immutable (can't be reassigned) by default — you need `let mut x = 5;` if you plan to change it.

- Common scalar types you'll see: `String` and `&str` (text — see below), `bool`, `u32`/`u64` (unsigned integers of a given bit width, i.e. no negative numbers — used for sizes/counts like generate-random-hex's `size: u32`), `i32` (signed integers, can be negative), `u8` (a single byte — `Vec<u8>` is "a list of bytes", used for raw file data).

String vs &str — the one that trips everyone up

Rust has two text types, and both appear throughout these components:

- `String` — an owned, growable piece of text. This crate owns the memory and can modify or hand it off.

- `&str` ("string slice") — a borrowed view into text you don't own, often a literal like `"hello"`.

In the examples, function parameters coming from the platform (WIT strings) always arrive as owned `String`. You'll see conversions like `String::from("hi")` and `.to_string()` to build a `String` from a literal, and `format!("...")` to build one from a template — both are used throughout store-file-base64 and the others.

## 4. Ownership — just enough to not get stuck

Rust's headline feature is that it checks, at compile time, that you never use a value after something else has taken ownership of it ("moved" it) or after it's been freed. This is what people mean by "the borrow checker." You don't need the full mental model to work with these components, but three things will save you time:

- Passing a `String` into a function usually moves it — the caller can't use that variable afterwards unless the function gives it back or you clone it first (`my_string.clone()`) or pass a reference (`&my_string`) instead.

- A function signature like `fn concat_text(first_text: String, second_text: String) -> String` takes ownership of both inputs and returns a brand new owned `String` — this is the pattern used throughout, and it's the easiest one to reason about: values come in, a new value goes out, nothing is shared.

- If the compiler says "value moved here" or "borrow of moved value", the fix is almost always one of: clone it (`.clone()`), restructure so you use the value once, or take a reference (`&`) instead of taking ownership. This is a compile-time error, not a runtime bug — your build simply won't produce a `.wasm` until it's resolved, which is a good thing.

For the full mental model (borrowing, lifetimes, etc.), the Rust Book's ownership chapter is the standard reference — but for the size of function most WASM actions need, you'll rarely go beyond the three points above.

## 5. Structs, traits, and impl — the pattern every component uses

Every example component follows the same shape: define an empty struct to represent "this component", then implement a trait (an interface) that the platform's tooling generated for you from your WIT file.

```rust
struct ConcatText; // a struct with no fields — just a "marker" type

impl Guest for ConcatText { // "ConcatText implements the Guest interface"
fn concat_text(first_text: String, second_text: String) -> String {
format!("{}{}", first_text, second_text)
}
}
```

- `struct ConcatText;` — declares a type with no data. It exists purely so something can implement the trait on it.

- `trait` — Rust's word for an interface: a set of function signatures a type promises to implement. You never write the `Guest` trait yourself; it's generated for you from your `wit/world.wit` file by a macro (see section 7).

- `impl Guest for ConcatText { ... }` — "here is how ConcatText implements the Guest trait." The function bodies inside are your actual logic; their names and signatures must match what `Guest` declares (which in turn matches your WIT interface exactly).

This is the one pattern to internalize: your job, in every component, is to fill in the function bodies inside one of these `impl Guest for` blocks. Everything else is scaffolding.

## 6. Option, Result, and error handling

Rust has no null and no exceptions. Instead, "a value that might not be there" and "an operation that might fail" are both ordinary types you handle explicitly.

  ---------------- ---------------------------------------------------------- --------------------------------------------------------------------------------------------------------------------------------------------
  **Type**         **Means**                                                  **Example from the components**

  `Option<T>`      Either `Some(value)` or `None` — a value that may be absent.   `template: Option<String>` in liquid-template's function signature — the template argument may or may not be provided.

  `Result<T, E>`   Either `Ok(value)` (success) or `Err(error)` (failure).        `store_file(...) -> Result<String, String>` in store-file-base64 — returns the file reference on success, or an error message on failure.
  ---------------- ---------------------------------------------------------- --------------------------------------------------------------------------------------------------------------------------------------------

The ? operator

Writing out a match on every `Result` would get repetitive, so Rust has a shorthand: put `?` after an expression that returns a `Result`, and if it's an `Err`, the function returns that error immediately; if it's `Ok`, you get the unwrapped value and execution continues. This only works in a function that itself returns a `Result` (or `Option`) with a compatible error type.

```rust
let file_bytes = BASE64
.decode(&data)
.map_err(|error| format!("Failed to decode base64 source: {error}"))?;
// if decode() failed, the function returns here with Err(...);
// otherwise file_bytes is the decoded Vec<u8> and execution carries on
```

- `.map_err(|closure|)` transforms the error type inside a `Result` — used constantly to turn a low-level error into the `String` error type the WIT interface expects.

- `.ok_or("message")` turns an `Option` into a `Result`, substituting the given error if it was `None` — used in store-file-base64 to turn "no property was provided" into a proper error: `property.into_iter().next().ok_or("Failed to fetch file property")?;`

## 7. Macros you'll see (and don't need to fully understand)

A few lines appear in every component that look like function calls but end in `!` — these are macros, which generate code at compile time. You don't need to know how they work internally, just what they're for:

  ------------------------------------------ -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------
  **Macro**                                  **What it does**

  `wit_bindgen::generate!({ generate_all })`   Reads your `wit/world.wit` file and generates Rust types and a `Guest` trait matching it. This is what makes the `Guest` trait in section 5 exist at all — you never hand-write it.

  `export!(YourStruct)`                        Registers `YourStruct` as the thing that actually implements the generated interface, wiring your `impl Guest` block up to the compiled component's public interface.

  `format!("{}{}", a, b)`                      Builds a `String` by substituting values into a template — Rust's equivalent of string interpolation / f-strings.

  `#[test]` / `#[cfg(test)]`                 Marks a function as a test (run by `cargo test`) or a whole module as test-only code (not included in the compiled component). See section 9.

  `assert_eq!(a, b)` / `assert!(cond)`           Used inside tests: fails the test with a clear message if the two values differ, or if the condition is false.
  ------------------------------------------ -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------

The bindings module at the top of every `lib.rs` (`mod bindings { wit_bindgen::generate!({ generate_all }); use crate::YourStruct; export!(YourStruct); }`) is boilerplate — copy it from an existing component and just swap the struct name.

## 8. Collections and everyday patterns

- `Vec<T>` — a growable list, Rust's equivalent of an array/list. `Vec<String>` is "a list of text"; `Vec<u8>` is "a list of bytes" (used for raw file data).

- `.join(separator)` — joins a `Vec<String>` into one `String`, used in concat-text: `text_list.join("")`.

- `.into_iter().map(...).collect()` — the standard "transform every item in a list" pattern, seen converting a list of `&str` literals into owned `String`s: `vec!["hi", "ha"].into_iter().map(String::from).collect()`.

- Closures — anonymous functions written as `|args| expression`, e.g. `|| format!("{:X}", rng.random_range(0..16))` in generate-random-hex. Read `||` as "a function that takes no arguments and returns...", or `|x| ...` as "a function that takes x and returns...".

- `std::iter::repeat_with(closure).take(n)` — calls a closure repeatedly to build a sequence of n items; used to generate n random hex characters one at a time.

- `.trim()` / `.is_empty()` — string helpers for checking/removing leading and trailing whitespace, used in store-file-base64's validation helper: `string.trim().is_empty()`.

## 9. Pattern matching: if let and match

Because `Option` and `Result` are ordinary enums (types with a fixed set of variants), Rust encourages handling them by pattern matching rather than by checking a flag. You'll see both the explicit `match` form and the shorter `?` form (section 6) in practice; `match` is worth recognizing even if you mostly reach for `?`:

```rust
match some_result {
Ok(value) => println!("Got {}", value),
Err(error) => println!("Failed: {}", error),
}
```

## 10. Testing patterns in these components

You'll see tests written two different ways, matching the two levels described in the main guide's build tooling section:

Plain unit tests, inline in `src/lib.rs`

```rust
#[test]
fn can_concat_two_strings() {
let result = ConcatText::concat_text(String::from("hi"), String::from("ha"));
assert_eq!(result, String::from("hiha"));
}
```

These call your Rust functions directly — fast, no WASM involved — good for testing pure logic.

Component tests in `tests/mod.rs`

```rust
mod bindings {
wasmtime_testing_helper::bindgen!("main");
wasmtime_testing_helper::setup!(Main);
}

#[test]
fn concats_strings() {
let harness = bindings::harness();
let mut component = bindings::instantiate(harness);
let interface = component.component.betty_blocks_concat_text_concat_text();
let result = interface.call_concat_text(&mut component.store, "hi", "hooo").unwrap();
assert_eq!("hihooo", result);
}
```

These load the actual compiled `.wasm` and call it through its real WIT interface — closer to how the platform will call it, but requires the component to be built first (just test runs the build automatically before testing).

`.unwrap()` forces a `Result` open and panics if it's an `Err` — fine inside a test where you want any failure to fail the test loudly, but avoid it in real component logic, where you should handle the `Err` case properly (see section 6) instead of letting the component crash.

## 11. Putting it together: store-file-base64, annotated

Here's the fullest example from the main guide, with the Rust-specific pieces called out:

```rust
impl StoreGuest for StoreFileBase64 {
fn store_file( // matches the WIT function signature exactly
helper_context: HelperContext,
model: Model,
property: Vec<Property>, // Vec<T> — a list of Property
data: String,
filename: String,
file_extension: String,
) -> Result<String, String> { // Ok(reference) or Err(message)
if string_is_empty(&filename) { // &filename: borrow, not a move — we still need
return Err(String::from("Filename must be set")); // filename later in this same function
}
if string_is_empty(&file_extension) {
return Err(String::from("File extension must be set"));
}

let property = property
.into_iter() // consume the Vec, one item at a time
.next() // Option<Property>: the first item, if any
.ok_or("Failed to fetch file property")?; // turn None into an Err and return early

let file_bytes = BASE64
.decode(&data)
.map_err(|error| format!("Failed to decode base64 source: {error}"))?;

let full_filename = if file_extension.starts_with('.') {
format!("{filename}{file_extension}")
} else {
let file_extension = file_extension.to_lowercase();
format!("{filename}.{file_extension}")
};

let upload_result = upload_file::upload(&helper_context, &upload_file::Input {
model, property, file_bytes, full_filename,
}).map_err(|error| format!("Upload failed: {error}"))?;

Ok(upload_result.reference) // success: wrap the value in Ok(...)
}
}
```

Notice the shape: validate early and return `Err` on bad input (an `if` with an early return), use `?` to bail out of anything that can fail partway through, and end with `Ok(...)` wrapping the real result. This is a very common shape for a WASM action's implementation and a good template to copy for your own.

## 12. Common compiler errors you'll hit, and what they mean

  ----------------------------------------------------- ------------------------------------------------------------------------------------------------------------------ ---------------------------------------------------------------------------------------------------------------
  **Error (roughly)**                                   **What's going on**                                                                                                **Typical fix**

  `` expected `;`, found ... ``                             Missing semicolon at the end of a statement.                                                                       Add the semicolon. Remember: the very last expression in a function is the exception.

  mismatched types: expected `String`, found `&str`         You handed a borrowed string literal where an owned String was expected.                                           Wrap it: `String::from("text")` or `"text".to_string()`.

  `use of moved value`                                    You used a variable after passing it (by value) into something else that took ownership of it.                     Clone it beforehand (`value.clone()`), reorder your code, or pass a reference (`&value`) instead.

  `cannot borrow as mutable`                              You're trying to change something through a reference that isn't allowed to change it.                             Add `mut` where the variable is declared and where it's borrowed: `let mut x`, `&mut x`.

  `` no method named `foo` found for type ``                Usually a typo, or you're missing a use statement that brings a trait/type into scope.                             Check spelling; check whether the method comes from a trait that needs `use SomeTrait;` at the top of the file.

  `this function takes N arguments but M were supplied`   Signature mismatch, often after editing a WIT interface without updating the Rust impl to match (or vice versa).   Compare your `fn` signature to the exact function declaration in `wit/world.wit`.
  ----------------------------------------------------- ------------------------------------------------------------------------------------------------------------------ ---------------------------------------------------------------------------------------------------------------

## 13. Where to go deeper

This guide deliberately only covers what shows up in Betty Blocks' example components. For anything beyond that — generics, lifetimes in more depth, async, more of the standard library — the official, free Rust Book (doc.rust-lang.org/book) is the standard reference and assumes no prior Rust experience. The "Understanding Ownership" and "Enums and Pattern Matching" chapters are the most directly useful follow-ups to this guide.

For the WIT/WASM-specific side of things — defining interfaces, host imports, the build and publish pipeline — see the main "WASM Actions on Betty Blocks" crash course, particularly its component anatomy and quick-start checklist sections.
