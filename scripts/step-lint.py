#!/usr/bin/env python3
"""Deterministic checks for the steps under functions/<name>/<version>/.

Run it with `just step-lint`. It needs only python3 (standard library). It checks what
`bb functions validate` does not: that `function.json` matches the WIT, and the rules recorded in
docs/developer-learnings-log.md that a reviewer would otherwise have to check by reading.

Errors fail the run (exit 1). Warnings are printed and do not.

Limits: it reads the step's own wit/*.wit with regular expressions, not a WIT parser. It does not
follow types imported with `use`, and it treats every `name: func(...)` as a function (resource
methods included), picking the one named after the step's folder.

Usage: step-lint.py [functions-dir]    (default: functions)
Tests: python3 scripts/test_step_lint.py
"""

import json
import os
import re
import subprocess
import sys

# Betty Blocks writes step text to database columns limited to 255 characters. The log confirms it
# for `description` (docs/developer-learnings-log.md, 2026-09-23); the other text fields are held
# to the same limit as a precaution, because they are written the same way.
TEXT_LIMIT = 255
REQUIRED_KEYS = ("description", "label", "category", "icon", "options")
RECOMMENDED_KEYS = ("yields",)
# Injected by the platform; not an option in function.json.
PLATFORM_PARAMS = {"helper-context"}

OPTION_TYPE = re.compile(r"\boption\s*<")
IDENTIFIER = re.compile(r"[A-Za-z][\w-]*")


def strip_comments(text):
    # One pass, so a `//` line comment that mentions `/*` does not start a block comment.
    return re.sub(r"//[^\n]*|/\*.*?\*/", "", text, flags=re.DOTALL)


def split_top(text):
    """Split on commas that are not inside <...>, so `result<a, b>` stays in one piece."""
    parts, depth, current = [], 0, ""
    for character in text:
        if character == "<":
            depth += 1
        elif character == ">":
            depth -= 1
        if character == "," and depth == 0:
            parts.append(current)
            current = ""
        else:
            current += character
    parts.append(current)
    return [part.strip() for part in parts if part.strip()]


def parse_wit(text):
    """Returns (functions, records, definitions, aliases) of one WIT file's text.

    `records` maps a record to its fields. `definitions` maps every record, variant and alias to
    the text of the types it uses (not the field or case names), so a type that hides an
    `option<...>` behind a name can be followed. `aliases` maps `type x = y;` to `y`.
    """
    text = strip_comments(text)
    records, definitions, aliases = {}, {}, {}
    for match in re.finditer(r"record\s+([\w-]+)\s*\{(.*?)\}", text, re.DOTALL):
        fields = {}
        for field in split_top(match.group(2)):
            name, _, type_ = field.partition(":")
            fields[name.strip()] = type_.strip()
        records[match.group(1)] = fields
        definitions[match.group(1)] = " ".join(fields.values())
    for match in re.finditer(r"variant\s+([\w-]+)\s*\{(.*?)\}", text, re.DOTALL):
        definitions[match.group(1)] = " ".join(re.findall(r"\(([^)]*)\)", match.group(2)))
    for match in re.finditer(r"type\s+([\w-]+)\s*=\s*([^;]+);", text):
        aliases[match.group(1)] = match.group(2).strip()
        definitions[match.group(1)] = match.group(2).strip()
    functions = {}
    for match in re.finditer(
        r"([A-Za-z][\w-]*)\s*:\s*func\s*\((.*?)\)\s*(?:->\s*([^;]+?))?\s*;", text, re.DOTALL
    ):
        params = {}
        for param in split_top(match.group(2)):
            name, _, type_ = param.partition(":")
            params[name.strip()] = type_.strip()
        functions[match.group(1)] = (params, (match.group(3) or "").strip())
    return functions, records, definitions, aliases


def ok_type(return_type):
    inner = re.fullmatch(r"result\s*<(.*)>", return_type, re.DOTALL)
    if inner:
        return split_top(inner.group(1))[0]
    return return_type


def resolve_alias(type_text, aliases, records):
    """Follow `type a = b;` while `a` is a plain alias of another named type."""
    for _ in range(20):
        target = aliases.get(type_text)
        if type_text in records or target is None or not IDENTIFIER.fullmatch(target):
            break
        type_text = target
    return type_text


def mentions_option(type_text, definitions):
    """Whether the type, or a record, variant or alias it uses (at any depth), has an `option<...>`."""
    seen, queue = set(), [type_text]
    while queue:
        current = queue.pop()
        if OPTION_TYPE.search(current):
            return True
        for name in IDENTIFIER.findall(current):
            if name in definitions and name not in seen:
                seen.add(name)
                queue.append(definitions[name])
    return False


class Report:
    def __init__(self):
        self.errors = 0
        self.warnings = 0

    def error(self, where, message):
        self.errors += 1
        print(f"ERROR  {where}: {message}")

    def warn(self, where, message):
        self.warnings += 1
        print(f"WARN   {where}: {message}")


def load_manifest(directory, report):
    """The parsed function.json, or None after reporting why it cannot be used."""
    path = os.path.join(directory, "function.json")
    if os.path.islink(path):
        report.error(
            directory,
            "function.json is a symlink: `bb functions validate` silently skips it, and `publish` "
            "almost certainly does too (copy function.json, symlink only src/ and wit/)",
        )
        return None
    if not os.path.isfile(path):
        report.error(directory, "function.json is missing")
        return None
    try:
        with open(path, encoding="utf-8") as handle:
            manifest = json.load(handle)
    except (json.JSONDecodeError, UnicodeDecodeError, OSError) as error:
        report.error(directory, f"function.json cannot be read as JSON: {error}")
        return None
    if not isinstance(manifest, dict):
        report.error(directory, "function.json must be a JSON object")
        return None
    return manifest


def check_manifest_text(directory, manifest, report):
    """Required keys, the text limit, and `info`. Returns (inputs, outputs) as lists of dicts."""
    for key in REQUIRED_KEYS:
        if manifest.get(key) in ("", None):
            report.error(directory, f"function.json has no `{key}`")
    for key in RECOMMENDED_KEYS:
        if key not in manifest:
            report.warn(directory, f"function.json has no `{key}` (every step so far sets it)")
    for key in ("description", "label"):
        value = manifest.get(key)
        if isinstance(value, str) and len(value) > TEXT_LIMIT:
            report.error(directory, f"`{key}` is {len(value)} characters (limit {TEXT_LIMIT})")

    options = manifest.get("options")
    if not isinstance(options, list):
        report.error(directory, "`options` must be a list")
        return None
    inputs, outputs = [], []
    for position, option in enumerate(options):
        if not isinstance(option, dict) or not isinstance(option.get("name"), str):
            report.error(directory, f"options[{position}] must be an object with a string `name`")
            continue
        name = option["name"]
        meta = option.get("meta")
        if not isinstance(meta, dict):
            report.error(directory, f"option `{name}` has no `meta` object")
            meta = {}
        option = {**option, "meta": meta}
        (outputs if meta.get("type") == "Output" else inputs).append(option)
        if not option.get("info"):
            report.error(directory, f"option `{name}` has no `info`")
        for key in ("label", "info"):
            value = option.get(key)
            if isinstance(value, str) and len(value) > TEXT_LIMIT:
                report.error(
                    directory, f"option `{name}` `{key}` is {len(value)} characters (limit {TEXT_LIMIT})"
                )
    return inputs, outputs


def read_wit(directory, report):
    """(functions, records, definitions, aliases) of every wit/*.wit, or None after reporting."""
    wit_dir = os.path.join(directory, "wit")
    names = []
    if os.path.isdir(wit_dir):
        names = [name for name in sorted(os.listdir(wit_dir)) if name.endswith(".wit")]
    if not names:
        report.error(directory, "no wit/*.wit file")
        return None
    functions, records, definitions, aliases = {}, {}, {}, {}
    for name in names:
        try:
            with open(os.path.join(wit_dir, name), encoding="utf-8", errors="replace") as handle:
                found = parse_wit(handle.read())
        except OSError as error:
            report.error(directory, f"wit/{name} cannot be read: {error}")
            return None
        for target, source in zip((functions, records, definitions, aliases), found):
            target.update(source)
    return functions, records, definitions, aliases


def lint_step(directory, report):
    manifest = load_manifest(directory, report)
    if manifest is None:
        return
    checked = check_manifest_text(directory, manifest, report)
    if checked is None:
        return
    inputs, outputs = checked

    wit = read_wit(directory, report)
    if wit is None:
        return
    functions, records, definitions, aliases = wit

    step_name = os.path.basename(os.path.dirname(directory))
    if step_name in functions:
        function_name = step_name
    elif len(functions) == 1:
        function_name = next(iter(functions))
    else:
        report.error(directory, f"cannot tell which exported WIT function is the step ({sorted(functions)})")
        return
    params, return_type = functions[function_name]

    # Inputs: WIT parameters <-> non-output options, by exact (kebab-case) name. A SchemaModel option
    # only types an output, so it may have no WIT parameter (inspect-id-token) or one (data-api-request).
    wit_params = {name for name in params if name not in PLATFORM_PARAMS}
    option_names = {option["name"] for option in inputs}
    schema_model_options = {
        option["name"] for option in inputs if option["meta"].get("type") == "SchemaModel"
    }
    for name in sorted(wit_params - option_names):
        report.error(directory, f"WIT parameter `{name}` has no option in function.json")
    for name in sorted(option_names - wit_params - schema_model_options):
        report.error(directory, f"option `{name}` has no WIT parameter of that name")

    # Outputs: a record return maps field -> output option; a single return has one output; a
    # function that returns nothing has none.
    ok_raw = ok_type(return_type)
    ok = resolve_alias(ok_raw, aliases, records)
    output_names = {option["name"] for option in outputs}
    if ok in ("", "_", "result"):
        if outputs:
            report.error(
                directory,
                f"the WIT function returns nothing but function.json has output option(s): {sorted(output_names)}",
            )
    elif ok in records:
        fields = set(records[ok])
        for name in sorted(fields - output_names):
            report.error(directory, f"record field `{name}` has no output option in function.json")
        for name in sorted(output_names - fields):
            report.error(directory, f"output option `{name}` is not a field of record `{ok}`")
    elif len(outputs) != 1:
        report.warn(directory, f"a single return value should have exactly one output option, found {len(outputs)}")

    # An option<...> anywhere in the ok type fails at execution (the error type is not an output).
    if mentions_option(ok_raw, definitions):
        report.error(
            directory,
            "the output type contains `option<...>`: it fails at execution with "
            '"expected a string, received [object]" (docs/developer-learnings-log.md, 2026-09-29)',
        )

    # An Object output typed with a schemaModel only binds if its WIT type is a named alias of string.
    for output in outputs:
        if "schemaModel" not in json.dumps(output["meta"].get("output", {})):
            continue
        wit_type = records[ok].get(output["name"]) if ok in records else ok_raw
        if wit_type == "string":
            report.warn(
                directory,
                f"output `{output['name']}` is typed with a schemaModel but its WIT type is a bare `string`: "
                "use a named alias (`type json-string = string;`), otherwise its fields will not bind "
                "(docs/developer-learnings-log.md, 2026-09-24)",
            )


def tracked_wasm_files(root):
    result = subprocess.run(
        ["git", "ls-files", "*.wasm"], cwd=root, capture_output=True, text=True, check=False
    )
    if result.returncode != 0:
        return None
    return [line for line in result.stdout.splitlines() if line]


def main():
    root = sys.argv[1] if len(sys.argv) > 1 else "functions"
    if not os.path.isdir(root):
        print(f"ERROR  {root}: not a directory")
        return 1
    report = Report()

    steps = []
    for current, directories, files in os.walk(root):
        directories[:] = sorted(d for d in directories if d not in ("target", "external", ".original"))
        if "Cargo.toml" in files:
            steps.append(current)
    if not steps:
        print(f"No steps under {root}/ — nothing to lint.")
        return 0

    for step in sorted(steps):
        lint_step(step, report)

    tracked = tracked_wasm_files(root)
    if tracked:
        report.error(root, f"compiled .wasm files are tracked in git: {', '.join(tracked)}")

    print(f"step-lint: {len(steps)} step(s), {report.errors} error(s), {report.warnings} warning(s)")
    return 1 if report.errors else 0


if __name__ == "__main__":
    sys.exit(main())
