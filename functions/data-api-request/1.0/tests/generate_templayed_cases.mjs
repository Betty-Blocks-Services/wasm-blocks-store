// Regenerates templayed_cases.json: the expected output of the ORIGINAL templayed.js for the
// cases the Rust renderer in src/template.rs has to match.
//
//   curl -L https://raw.githubusercontent.com/Betty-Blocks-Services/Data-API-request-step/main/functions/utils/templayed.js -o templayed.mjs
//   node generate_templayed_cases.mjs
//
// Cases where templayed.js itself throws are skipped (and listed on stderr). The Rust side
// deliberately differs from templayed there (see the module docs of src/template.rs).
import templayed from "./templayed.mjs";
import fs from "node:fs";

const cases = [
  ["id: { eq: 1 }", {}],
  ["name: { eq: \"{{name}}\" }, id: { eq: {{ id }} } ", { name: "Jan", id: 7 }],
  ["id: { eq: {{id}} } ", { id: "1" }],
  ["a{{nope}}b", {}],
  ["[{{f}}][{{t}}][{{n}}][{{s}}][{{z}}] ", { f: false, t: true, n: null, s: "", z: 0 }],
  ["{{v}} / {{{v}}} / {{&v}} ", { v: "a&b\"c<d>e'f" }],
  ["[{{ids}}] [{{mixed}}] [{{empty}}] ", { ids: [1, 2, 3], mixed: [1, null, "x", [2, 3], false, true], empty: [] }],
  ["{{o}} ", { o: { a: 1 } }],
  ["{{webuser.id}} {{webuser.tags.1}} {{webuser.tags.length}} {{webuser.name.length}} ", { webuser: { id: 1, tags: ["a", "b"], name: "héllo" } }],
  ["{{a}} {{b}} {{c}} {{d}} {{e}} ", { a: 1.0, b: 1.5, c: -3, d: 100000000000.0, e: 0.1 }],
  ["a{{! a note }}b ", {}],
  ["{{#users}}<{{name}}>{{/users}} ", { users: [{ name: "a" }, { name: "b" }] }],
  ["{{#list}}{{.}}{{../sep}}{{/list}} ", { list: [1, 2], sep: "-" }],
  ["{{#yes}}A{{/yes}}{{#no}}B{{/no}} ", { yes: true, no: false }],
  ["{{^empty}}1{{/empty}}{{^full}}2{{/full}}{{^no}}3{{/no}}{{^yes}}4{{/yes}}{{^missing}}5{{/missing}}{{^zero}}6{{/zero}}{{^blank}}7{{/blank}} ", { empty: [], full: [1], no: false, yes: true, zero: 0, blank: "" }],
  ["{{#rows}}[{{#cells}}{{.}}{{/cells}}]{{/rows}} ", { rows: [{ cells: [1, 2] }, { cells: [3] }] }],
  ["line one\nline \"two\" {{x}}\nline three ", { x: "X" }],
  ["{ and: [ { id: { in: [{{ids}} ] } }, { name: { eq: \"{{ name }}\" } } ] } ", { ids: [4, 5, 6], name: "Jan" }],
  ["{{#items}}{{id}}{{^last}},{{/last}}{{/items}} ", { items: [{ id: 1 }, { id: 2, last: true }] }],
  ["{{  spaced  }} {{ ../x }} ", { spaced: "s", x: "top" }],
  ["{{record}} ", { record: { id: 1, name: "N" } }],
  ["{{#a}}{{#b}}{{.}}{{../c}}{{/b}}{{/a}} ", { a: [{ b: [1, 2], c: "c" }] }],
];

const out = [];
for (const [template, vars] of cases) {
  try {
    out.push({ template, variables: vars, expected: templayed(template)(vars) });
  } catch (e) {
    console.error("JS throws, skipped:", JSON.stringify(template), String(e).slice(0, 60));
  }
}
fs.writeFileSync("templayed_cases.json", JSON.stringify(out, null, 2) + "\n");
console.log(out.length, "cases");
