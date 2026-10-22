// Regenerates data_api_request_cases.json: the query the ORIGINAL ActionsJS step (1.7) sends to
// gql() for a set of inputs, and what it returns for a mocked answer, which src/query.rs has to
// reproduce (queries compared ignoring whitespace).
//
//   ./fetch_original.sh                       # the original, at a pinned commit
//   node generate_query_cases.mjs
//
// Left out on purpose, because the Rust step differs there (see the README): a `take` above
// 200 (the original sends it as is), a query data without `id` (the Rust step adds it), and an
// invalid sort order. After regenerating, update EXPECTED_CASES in src/query.rs.
import fs from "node:fs";
import step from "./.original/functions/dataApiRequest/1.7/index.mjs";

let sent;
globalThis.gql = async (query) => {
  sent = query;
  const field = query.match(/\b((?:one|all)\w+)\(/)[1];
  const data = field.startsWith("one")
    ? { [field]: { id: 1, name: "A" } }
    : { [field]: { results: [{ id: 1 }, { id: 2 }], totalCount: 7 } };
  return { data };
};

const base = { type: "collection", queryData: "id name", filter: "", filterVariables: [], skip: undefined, take: undefined, sortOrder: "ASC", sortField: "", model: { name: "Webuser" }, outputType: "results" };
const variants = [
  {},
  { type: "record", filter: "id: { eq: {{id}} } ", filterVariables: [{ key: "id", value: "1" }] },
  { outputType: "totalCount" },
  { sortField: "name", sortOrder: "DESC" },
  { sortOrder: "DESC" },
  { take: 50, skip: 10 },
  { take: 0 },
  { type: "record", skip: 3, take: 5 },
  { type: "record", outputType: "totalCount" },
  { filter: "" },
  { filter: "name: { eq: \"{{name}}\" }, id: { in: [{{ids}} ] } ", filterVariables: [{ key: "name", value: "Jan" }, { key: "ids", value: [1, 2, 3] }] },
  { queryData: "id name company { id name }" },
  { type: "record", queryData: "id\n  name\n  company { id name }", model: { name: "Task" } },
];

const out = [];
for (const variant of variants) {
  const input = { ...base, ...variant };
  const returned = await step(input);
  out.push({
    model: input.model.name, queryType: input.type, filter: input.filter || null,
    variables: Object.fromEntries(input.filterVariables.map((x) => [x.key, x.value])),
    skip: input.skip ?? null, take: input.take ?? null, sortOrder: input.sortOrder,
    sortField: input.sortField || null, queryData: input.queryData, outputType: input.outputType,
    expectedQuery: sent.split(/\s+/).filter(Boolean).join(" "),
    expectedField: sent.match(/\b((?:one|all)\w+)\(/)[1],
    response: JSON.stringify(await globalThis.gql(sent)),
    expectedResult: returned.result,
  });
}
fs.writeFileSync("data_api_request_cases.json", JSON.stringify(out, null, 2) + "\n");
console.log(out.length, "cases");
