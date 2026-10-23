"""Regression tests for scripts/step-lint.py: builds throwaway steps in a temp dir and checks the
lint's verdict on each. Run: python3 scripts/test_step_lint.py  (python3 standard library only)."""
import json, os, shutil, subprocess, sys, tempfile
SCRIPT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "step-lint.py")
def opt(name, kind="Text", info="i", **extra):
    meta = {"type": kind}; meta.update(extra)
    return {"name": name, "label": name, "info": info, "meta": meta}
def manifest(options, **over):
    d = {"description": "d", "label": "l", "category": "c", "icon": "i", "yields": "NONE", "options": options}
    d.update(over); return d
RES = lambda name="result", **kw: opt(name, "Output", output=kw.get("output", {"anyOf": [{"type": "Text"}]}))
def run(wit, man, raw_json=None, raw_wit=None, extra=None):
    root = tempfile.mkdtemp()
    d = os.path.join(root, "functions", "demo", "1.0"); os.makedirs(os.path.join(d, "wit"))
    open(os.path.join(d, "Cargo.toml"), "w").write("[package]\n")
    open(os.path.join(d, "wit", "world.wit"), "wb").write(raw_wit if raw_wit is not None else wit.encode())
    p = os.path.join(d, "function.json")
    if raw_json is not None: open(p, "wb").write(raw_json)
    else: json.dump(man, open(p, "w"))
    if extra: extra(d)
    r = subprocess.run([sys.executable, SCRIPT, os.path.join(root, "functions")], capture_output=True, text=True)
    shutil.rmtree(root)
    return r
def W(ret, extra="", params="a: string"):
    return f"package a:demo@1.0.0;\ninterface demo {{\n{extra}\n demo: func({params}) -> {ret};\n}}\nworld w {{ export demo; }}\n"
ok = 0; bad = 0
def check(name, r, want_in=(), want_not=(), code=None):
    global ok, bad
    out = r.stdout + r.stderr
    good = all(s in out for s in want_in) and not any(s in out for s in want_not) and "Traceback" not in out
    if code is not None: good = good and r.returncode == code
    ok += good; bad += not good
    print(("PASS " if good else "FAIL ") + name + ("" if good else "\n" + out))
A = opt("a")
# baseline
check("baseline passes", run(W("result<json-string, string>", "type json-string = string;"), manifest([A, RES()])), ["0 error"], code=0)
# 1 field-name false positive
check("field name equals input record (no FP)", run(W("result<r, string>", "record meta { x: option<string> }\nrecord r { meta: string }", "a: meta"),
      manifest([opt("a"), RES("meta")])), want_not=["option<"])
# 2 crashes
for label, raw in [("[]", b"[]"), ("null", b"null"), ("bad utf8", b"\xff\xfe{"), ]:
    check("function.json " + label, run("", None, raw_json=raw, raw_wit=W("string").encode()), ["ERROR"], code=1)
check("options dict", run(W("string"), manifest({"a": 1})), ["ERROR"], code=1)
check("options string", run(W("string"), manifest("x")), ["ERROR"], code=1)
check("option no name", run(W("string"), manifest([{"info": "i", "meta": {"type": "Text"}}])), ["ERROR"], code=1)
check("meta is a string", run(W("string"), manifest([{"name": "a", "info": "i", "meta": "x"}, RES()])), ["ERROR"], code=1)
check("non-dict option", run(W("string"), manifest([5])), ["ERROR"], code=1)
check("non-utf8 wit", run("", manifest([A, RES()]), raw_wit=b"\xff\xfe interface x {}"), ["ERROR"], code=1)
def dangling(d):
    os.symlink("/nonexistent", os.path.join(d, "wit", "x.wit"))
check("dangling wit symlink", run(W("string"), manifest([A, RES()]), extra=dangling), ["ERROR"], code=1)
# 3 unit return
r = run(W("result<_, string>"), manifest([A])); check("unit return, no outputs: clean", r, ["0 error", "0 warning"], code=0)
r = run(W("result<_, string>"), manifest([A, RES()])); check("unit return with an output option: error", r, ["returns nothing"], code=1)
r = run("package a:demo@1.0.0;\ninterface demo { demo: func(a: string); }\nworld w { export demo; }\n", manifest([A])); check("no return, no outputs: clean", r, ["0 error", "0 warning"], code=0)
# 4 aliased record
r = run(W("result<r2, string>", "record r { x: string, y: string }\ntype r2 = r;"), manifest([A, opt("x", "Output", output={})])); check("aliased record resolved (missing field y)", r, ["`y` has no output option"], code=1)
# 5 spaced option
r = run(W("result<option < string >, string>"), manifest([A, RES()])); check("option < string > detected", r, ["option<"], code=1)
# 6 comment containing /*
r = run("package a:demo@1.0.0;\n// see /* wildcard\ninterface demo {\n demo: func(a: string) -> result<string, string>;\n}\n/* real */\nworld w { export demo; }\n", manifest([A, RES()])); check("// /* does not eat the function", r, ["0 error"], code=0)
# 7 yields only a warning
m = manifest([A, RES()]); del m["yields"]; r = run(W("result<string, string>"), m); check("missing yields is a warning", r, ["WARN", "0 error"], code=0)
# 8 record field bare string with schemaModel
sm = opt("claims", "Output", output={"anyOf": [{"type": "Object", "schemaModel": "sm"}]})
r = run(W("result<r, string>", "record r { claims: string }"), manifest([A, sm])); check("record field bare string + schemaModel warns", r, ["bare `string`"], code=0)
r = run(W("result<r, string>", "type js = string;\nrecord r { claims: js }"), manifest([A, sm])); check("record field alias + schemaModel is fine", r, ["0 warning"], code=0)
# 9 option in error type is not an output
r = run(W("result<string, option<string>>"), manifest([A, RES()])); check("option in error type is fine", r, want_not=["option<"], code=0)
# earlier defects still caught
r = run(W("result<option<string>, string>"), manifest([A, RES()])); check("option in ok type is an error", r, ["option<"], code=1)
r = run(W("result<v, string>", "variant v { a(option<string>), b }"), manifest([A, RES()])); check("option hidden in a variant", r, ["option<"], code=1)
r = run(W("result<h, string>", "type h = option<string>;"), manifest([A, RES()])); check("option hidden in an alias", r, ["option<"], code=1)
r = run(W("string"), manifest([A, RES()], description="x" * 256)); check("description 256 chars", r, ["256 characters"], code=1)
def sym(d):
    os.rename(os.path.join(d, "function.json"), os.path.join(d, "real.json")); os.symlink("real.json", os.path.join(d, "function.json"))
r = run(W("string"), manifest([A, RES()]), extra=sym); check("symlinked function.json", r, ["symlink"], code=1)
r = subprocess.run([sys.executable, SCRIPT, "no-such-dir"], capture_output=True, text=True); check("missing directory fails", r, ["not a directory"], code=1)
r = run(W("result<json-string, string>", "type json-string = string;"), manifest([A, sm])); check("schemaModel output with a named alias is fine", r, ["0 warning", "0 error"], code=0)
r = run(W("string"), manifest([A, sm])); check("schemaModel output with a bare string warns", r, ["bare `string`"], code=0)
print(f"{ok} passed, {bad} failed"); sys.exit(1 if bad else 0)
