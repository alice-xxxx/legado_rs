import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import ts from "typescript";
import { parse } from "@vue/compiler-sfc";
import { computed, reactive, ref, watch, nextTick } from "vue";

const page = await readFile(new URL("../src/features/sources/SourceWorkspacePage.vue", import.meta.url), "utf8");
const source = parse(page).descriptor.scriptSetup.content;
const tree = ts.createSourceFile("source-navigation.ts", source, ts.ScriptTarget.Latest, true);
const names = ["sourceListLocation", "leaveEditorRoute", "closeSourceEditor", "saveSourceEditor",
  "requestEditorSwitch", "isEdited", "chooseRouteLeave", "guardEditorNavigation"];
const selected = tree.statements.filter(node => ts.isFunctionDeclaration(node) && names.includes(node.name?.text)
  || ts.isExpressionStatement(node) && ts.isCallExpression(node.expression)
    && node.expression.expression.getText(tree) === "watch" && node.expression.arguments[0]?.getText(tree) === "routeSourceEdit");
const routeNames = { sources: "sources", settings: "settings" };
const currentRoute = reactive({ name: "sources", query: {}, path: "/sources", fullPath: "/sources" });
const routeSourceEdit = computed(() => typeof currentRoute.query.sourceEdit === "string" ? currentRoute.query.sourceEdit : "");
const sources = ref([{ id: "a" }, { id: "b" }]);
const editingSource = ref(null), pendingDraft = ref(false), confirmRouteLeave = ref(false), pendingSourceSwitch = ref(null);
let opens = 0, saves = 0, pushes = 0, replaces = 0;
const sourceEditJson = ref(""), sourceEditOriginalJson = ref(""), sourceEditCreating = ref(false), sourceEditBusy = ref(false);
const sourceManagementActions = {
  sourceEditJson, sourceEditOriginalJson, sourceEditCreating, sourceEditBusy,
  sourceEditorOpen: computed(() => editingSource.value !== null || sourceEditCreating.value),
  async beginSourceEdit(source) {
    opens++; sourceEditBusy.value = true;
    await Promise.resolve();
    editingSource.value = source;
    sourceEditJson.value = sourceEditOriginalJson.value = JSON.stringify(source);
    sourceEditCreating.value = false; sourceEditBusy.value = false;
  },
  beginSourceCreate() { sourceEditJson.value = sourceEditOriginalJson.value = "{}"; sourceEditCreating.value = true; },
  closeSourceEdit() { if (sourceEditBusy.value) return; editingSource.value = null; sourceEditCreating.value = false; sourceEditJson.value = sourceEditOriginalJson.value = ""; },
  async saveSourceDefinitionJson() {
    saves++; sources.value = [...sources.value];
    await nextTick();
    this.closeSourceEdit();
  },
};
const location = to => ({ name: to.name, path: `/${to.name}`, query: to.query ?? {},
  fullPath: `/${to.name}` + (Object.keys(to.query ?? {}).length ? "?" + new URLSearchParams(to.query) : "") });
const entries = [location({ name: "settings" }), location({ name: "sources" })];
let index = 1;
function navigate(to) { Object.assign(currentRoute, to); router.options.history.state.back = entries[index - 1]?.fullPath ?? null; }
const router = {
  options: { history: { state: { back: "/settings" } } }, resolve: location,
  async push(to) { pushes++; entries.splice(index + 1); entries.push(location(to)); index++; navigate(entries[index]); },
  async replace(to) { replaces++; entries[index] = location(to); navigate(entries[index]); },
  back() { index--; navigate(entries[index]); },
};
globalThis.__sourceNavigation = { router, routeNames, currentRoute, routeSourceEdit, sources, editingSource,
  pendingDraft, confirmRouteLeave, pendingSourceSwitch, sourceManagementActions,
  recoveryRequired: ref(false), window: { innerWidth: 390 }, notify() {}, errorText: String, watch,
  initialDesktopSourceOpened: false, editorNavigationPending: false, suppressSourceImportQuerySync: false,
  pageDisposed: false, resolveNavigation: null };
const content = selected.map(node => node.getText(tree)).join("\n") + `\nexport { ${names.join(",")} };`;
const { outputText } = ts.transpileModule(content, { compilerOptions: { module: ts.ModuleKind.ESNext } });
const prelude = `let { ${Object.keys(globalThis.__sourceNavigation).join(",")} } = globalThis.__sourceNavigation;\n`;
const flush = async () => { for (let i = 0; i < 6; i++) { await nextTick(); await Promise.resolve(); } };
try {
  const runtime = await import(`data:text/javascript;base64,${Buffer.from(prelude + outputText + "\n//# sourceURL=source-navigation-test.js").toString("base64")}`);
  runtime.requestEditorSwitch(sources.value[0]);
  runtime.requestEditorSwitch(sources.value[0]);
  await flush();
  assert.equal(pushes, 1, "Repeated taps create one editor history entry");
  assert.equal(router.options.history.state.back, "/sources");
  assert.equal(editingSource.value.id, "a");
  const beforeSave = opens;
  await runtime.saveSourceEditor(); await flush();
  assert.equal(saves, 1); assert.equal(opens, beforeSave, "Refreshing the saved source list cannot reopen its editor");
  assert.equal(currentRoute.fullPath, "/sources", "Save returns to the source list, not settings");

  runtime.requestEditorSwitch(sources.value[0]); await flush();
  pendingDraft.value = true;
  const cancelled = runtime.guardEditorNavigation();
  assert.equal(confirmRouteLeave.value, true);
  runtime.chooseRouteLeave(false);
  assert.equal(await cancelled, false);
  assert.equal(editingSource.value.id, "a");
  assert.equal(currentRoute.query.sourceEdit, "a", "Cancelling a swipe preserves both the route and draft");
  runtime.closeSourceEditor(); await flush();
  assert.equal(currentRoute.fullPath, "/sources");

  runtime.requestEditorSwitch(sources.value[0]); await flush();
  runtime.requestEditorSwitch(sources.value[1]); await flush();
  assert.equal(editingSource.value.id, "b");
  assert.equal(currentRoute.query.sourceEdit, "b", "Switching an editor updates its URL");
  pendingDraft.value = true;
  runtime.requestEditorSwitch(sources.value[0]);
  runtime.chooseRouteLeave(true); await flush();
  assert.equal(editingSource.value.id, "a");
  assert.equal(router.options.history.state.back, "/sources", "Discard-and-switch replaces the editor, preserving the list beneath it");
  assert.ok(replaces >= 2);
  sourceEditBusy.value = true; editingSource.value = null;
  assert.equal(runtime.guardEditorNavigation(), false, "History cannot escape while an editor is loading");
  console.log("PASS: source list history, repeated taps, save refresh, cancelled swipe, editor switching, discard, and busy navigation.");
} finally { delete globalThis.__sourceNavigation; }
