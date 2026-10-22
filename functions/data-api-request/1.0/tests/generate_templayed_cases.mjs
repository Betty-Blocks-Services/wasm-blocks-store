// Regenerates templayed_cases.json: what the ORIGINAL templayed.js renders for the cases the
// Rust renderer in src/template.rs has to match.
//
//   ./fetch_original.sh                       # the original, at a pinned commit
//   node generate_templayed_cases.mjs
//
// Cases where templayed.js itself throws cannot be matched; they are listed under "skipped"
// in the output so that dropping a case is visible in the diff. After regenerating, update
// EXPECTED_CASES in src/template.rs.
import fs from "node:fs";
import templayed from "./.original/functions/utils/templayed.mjs";

const cases = [
  ["id: { eq: 1 }", {}],
  ["name: { eq: \"{{name}}\" }, id: { eq: {{ id }} } ", { name: "Jan", id: 7 }],
  ["id: { eq: {{id}} } ", { id: "1" }],
  ["a{{nope}}b", {}],
  ["[{{f}}][{{t}}][{{n}}][{{s}}][{{z}}] ", { f: false, t: true, n: null, s: "", z: 0 }],
  ["{{v}} / {{{v}}} / {{&v}} ", { v: "a&b\"c<d>e'f" }],
  ["[{{ids}}] [{{mixed}}] [{{empty}}] ", { ids: [1, 2, 3], mixed: [1, null, "x", [2, 3], false, true], empty: [] }],
  ["{{o}} ", { o: { a: 1 } }],
  ["{{webuser.id}} {{webuser.tags.length}} {{webuser.name.length}} ", { webuser: { id: 1, tags: ["a", "b"], name: "héllo" } }],
  ["{{a}} {{b}} {{c}} {{d}} {{e}} ", { a: 1.0, b: 1.5, c: -3, d: 100000000000.0, e: 0.1 }],
  ["a{{! a note }}b ", {}],
  ["line one\nline \"two\" {{x}}\nline three ", { x: "X" }],
  ["{ and: [ { id: { in: [{{ids}} ] } }, { name: { eq: \"{{ name }}\" } } ] } ", { ids: [4, 5, 6], name: "Jan" }],
  ["{{  spaced  }} ", { spaced: "s" }],
  ["{{record}} ", { record: { id: 1, name: "N" } }],
];

// templayed.js throws on these; the Rust renderer deliberately does something else.
const throwing = [
  ["{{webuser.tags.1}} ", { webuser: { tags: ["a", "b"] } }],
  ["{{ ../x }} ", { x: "top" }],
];

const render = (template, vars) => templayed(template)(vars);
const out = { cases: [], skipped: [] };
for (const [template, variables] of cases) {
  out.cases.push({ template, variables, expected: render(template, variables) });
}
for (const [template, variables] of throwing) {
  try {
    render(template, variables);
    throw new Error(`expected templayed.js to throw for ${JSON.stringify(template)}`);
  } catch (error) {
    if (error.message.startsWith("expected templayed.js")) throw error;
    out.skipped.push({ template, variables, reason: String(error).slice(0, 80) });
  }
}
fs.writeFileSync("templayed_cases.json", JSON.stringify(out, null, 2) + "\n");
console.log(out.cases.length, "cases,", out.skipped.length, "skipped");
