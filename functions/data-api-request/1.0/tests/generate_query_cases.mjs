// Regenerates data_api_request_cases.json: the query the ORIGINAL ActionsJS step (1.7) sends to
// gql() for a set of inputs, which src/query.rs has to reproduce (ignoring whitespace).
//
// Set up a scratch folder with the original step and helper, with ES-module extensions:
//   functions/dataApiRequest/1.7/index.mjs   <- functions/dataApiRequest/1.7/index.js, import
//                                               of "../../utils/templayed" changed to ".mjs"
//   functions/utils/templayed.mjs            <- functions/utils/templayed.js
// from https://github.com/Betty-Blocks-Services/Data-API-request-step, then: node generate_query_cases.mjs
import fs from "node:fs";
import step from "./functions/dataApiRequest/1.7/index.mjs";

let sent;
globalThis.gql = async (query) => { sent = query; return { data: { oneWebuser: null, allWebuser: { results: [], totalCount: 0 } } }; };

const base = { type: "collection", queryData: "id name", filter: "", filterVariables: [], skip: undefined, take: undefined, sortOrder: "ASC", sortField: "", model: { name: "Webuser" }, outputType: "results" };
const variants = [
  {},
  { type: "record", filter: "id: { eq: {{id}} } ", filterVariables: [{ key: "id", value: "1" }] },
  { outputType: "totalCount" },
  { sortField: "name", sortOrder: "DESC" },
  { take: 50, skip: 10 },
  { take: 0 },
  { filter: "name: { eq: \"{{name}}\" }, id: { in: [{{ids}} ] } ", filterVariables: [{ key: "name", value: "Jan" }, { key: "ids", value: [1, 2, 3] }] },
  { type: "record", queryData: "id\n  name\n  company { id name }", model: { name: "Task" } },
];
const out = [];
for (const v of variants) {
  const input = { ...base, ...v };
  await step(input);
  out.push({
    model: input.model.name, queryType: input.type, filter: input.filter || null,
    variables: Object.fromEntries(input.filterVariables.map((x) => [x.key, x.value])),
    skip: input.skip ?? null, take: input.take ?? null, sortOrder: input.sortOrder,
    sortField: input.sortField || null, queryData: input.queryData, outputType: input.outputType,
    expectedQuery: sent.split(/\s+/).filter(Boolean).join(" "),
  });
}
fs.writeFileSync("data_api_request_cases.json", JSON.stringify(out, null, 2) + "\n");
console.log(out.length, "cases");
