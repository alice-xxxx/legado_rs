import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import vm from "node:vm";
import ts from "typescript";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const modules = new Map();
function load(name) {
  if (modules.has(name)) return modules.get(name);
  const filename = path.join(root, "src/features/sources", `${name}.ts`);
  const compiled = ts.transpileModule(fs.readFileSync(filename, "utf8"), {
    compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2020 },
  }).outputText;
  const exports = {};
  modules.set(name, exports);
  vm.runInNewContext(compiled, { exports, require: name => load(name.replace("./", "")), TextEncoder }, { filename });
  return exports;
}
const { bookSourceEditorTabs, rssSourceEditorTabs } = load("sourceEditorFields");
const { readSourceEditorDraft, serializeSourceEditorDraft, sourceEditorTabs } = load("sourceEditorDraft");
const fields = bookSourceEditorTabs.flatMap(tab => tab.fields);
function edit(json, changes, selectedFields = fields) {
  const baseline = readSourceEditorDraft(json, selectedFields, true);
  return serializeSourceEditorDraft(json, selectedFields, baseline, { ...baseline, ...changes });
}

const original = '{"bookSourceName":"Example","bookSourceUrl":"https://example.org","enabled":true,"lastUpdateTime":9223372036854775807,"ruleContent":{"content":"old","custom":9007199254740993}}';
assert.equal(edit(original, {}), original, "viewing fields must not populate defaults");
const result = edit(original, {
  "ruleSearch.name": "h2@text", "ruleSearch.author": ".author@text", "ruleContent.content": "new",
  loginUi: '[{"name":"password","type":"password"}]',
});
assert.deepEqual(JSON.parse(result).ruleSearch, { name: "h2@text", author: ".author@text" });
assert.equal(JSON.parse(result).ruleContent.content, "new");
assert.equal(typeof JSON.parse(result).loginUi, "string");
assert(!Object.hasOwn(JSON.parse(result), "header"));
assert(result.includes("9223372036854775807"));
assert(result.includes("9007199254740993"));

const stringRules = JSON.stringify({
  bookSourceName: "Example", bookSourceUrl: "https://example.org",
  ruleSearch: '{"name":"old","custom":9223372036854775807}', ruleToc: null,
});
const updatedRules = JSON.parse(edit(stringRules, { "ruleSearch.name": "new", "ruleToc.chapterList": ".chapters" }));
assert.equal(typeof updatedRules.ruleSearch, "string");
assert(updatedRules.ruleSearch.includes("9223372036854775807"));
assert.equal(JSON.parse(updatedRules.ruleSearch).name, "new");
assert.deepEqual(updatedRules.ruleToc, { chapterList: ".chapters" });

const template = "{}";
assert.deepEqual(JSON.parse(edit(template, {
  bookSourceName: "New", bookSourceUrl: "https://example.org", enabled: "false",
  "ruleToc.chapterList": ".chapter", "ruleToc.chapterName": "a@text",
})), {
  bookSourceName: "New", bookSourceUrl: "https://example.org", enabled: false,
  ruleToc: { chapterList: ".chapter", chapterName: "a@text" },
});
const baseline = readSourceEditorDraft(template, fields, true);
const reverted = { ...baseline, header: "temporary" };
reverted.header = baseline.header;
assert.equal(serializeSourceEditorDraft(template, fields, baseline, reverted), template);
assert.throws(() => edit(template, { weight: "1.5" }));
assert.throws(() => edit(template, { weight: "2147483648" }));
assert.throws(() => edit(template, { lastUpdateTime: "9223372036854775808" }));
assert.equal(JSON.parse(edit('{"bookSourceEnabled":true}', { enabled: "false" })).enabled, false);
assert.equal(JSON.parse(edit('{"bookSourceEnabled":true}', { enabled: "false" })).bookSourceEnabled, false);
assert.equal(JSON.parse(edit('{"loginUi":[]}', { loginUi: '[{"name":"username"}]' })).loginUi[0].name, "username");
assert.throws(() => edit('{"loginUi":[]}', { loginUi: "invalid array" }));
assert.equal(JSON.parse(edit('{"ruleContent":{"lrcRule":"old"}}', { "ruleContent.subContent": "new" })).ruleContent.lrcRule, "new");

const legacy = '{"sourceName":"RSS","sourceUrl":"https://example.org/feed","ruleContent":".article"}';
assert.equal(sourceEditorTabs(legacy), rssSourceEditorTabs);
const rssFields = rssSourceEditorTabs.flatMap(tab => tab.fields);
assert.equal(JSON.parse(edit(legacy, { ruleContent: "main" }, rssFields)).ruleContent, "main");

// Fail when a supported KMP property no longer has a form entry.
const models = { BookSource: "", SearchRule: "ruleSearch", ExploreRule: "ruleExplore", BookInfoRule: "ruleBookInfo",
  TocRule: "ruleToc", ContentRule: "ruleContent", ReviewRule: "ruleReview", OldRssSource: "RSS" };
for (const [model, parent] of Object.entries(models)) {
  const filename = path.join(root, "kotlin/kmp-engine/src/commonMain/kotlin/io/legado/app/data/entities",
    parent && parent !== "RSS" ? `rule/${model}.kt` : `${model}.kt`);
  const code = fs.readFileSync(filename, "utf8").split(") :")[0].split(") {")[0];
  for (const match of code.matchAll(/(?:override\s+)?var\s+(\w+)\s*:/g)) {
    const key = parent === "RSS" ? match[1] : parent ? `${parent}.${match[1]}` : match[1];
    assert((parent === "RSS" ? rssFields : fields).some(field => field.key === key || field.key.startsWith(`${key}.`)), `Missing ${model}.${key}`);
  }
}
console.log(`Source editor checks passed: ${fields.length} book fields, ${rssFields.length} RSS fields, schema coverage and save behavior.`);
