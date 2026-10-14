import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import ts from "typescript";
import { ref } from "vue";
import { parse } from "@vue/compiler-sfc";

async function load(source, bindings) {
  globalThis.__detailNavigationBindings = bindings;
  const { outputText } = ts.transpileModule(source, {
    compilerOptions: { module: ts.ModuleKind.ESNext, target: ts.ScriptTarget.ES2022 },
    transformers: { before: [context => root => ts.visitEachChild(root,
      node => ts.isImportDeclaration(node) ? undefined : node, context)] },
  });
  const prelude = `const { ${Object.keys(bindings).join(", ")} } = globalThis.__detailNavigationBindings;\n`;
  return import(`data:text/javascript;base64,${Buffer.from(prelude + outputText).toString("base64")}`);
}
const routeNames = { bookDetail: "book-detail", search: "search", shelf: "shelf", home: "home" };
const resolve = path => ({ fullPath: path, name: path.split("?")[0].slice(1) || "home" });
let entries = [], index = 0, wrongPop = false;
const hooks = new Set(), moves = [];
const router = {
  currentRoute: ref(null), options: { history: { state: {} } }, resolve,
  afterEach(callback) { hooks.add(callback); return () => hooks.delete(callback); },
  go(delta) { moves.push(delta); index += delta; navigate(wrongPop ? "/book-detail" : entries[index]); },
  back() { this.go(-1); },
  replace(target) { const path = typeof target === "string" ? target : `/${target.name}`; entries[index] = path; navigate(path); },
};
function navigate(path) {
  const from = router.currentRoute.value;
  router.currentRoute.value = resolve(path);
  router.options.history.state = { position: index, back: entries[index - 1] ?? null, current: path };
  for (const hook of [...hooks]) hook(router.currentRoute.value, from);
}
function reset(paths) { entries = [...paths]; index = paths.length - 1; moves.length = 0; wrongPop = false; navigate(paths[index]); }
function push(path) { entries = entries.slice(0, index + 1); entries.push(path); index++; navigate(path); }

try {
  const source = await readFile(new URL("../src/features/books/bookDetailOpenRequest.ts", import.meta.url), "utf8");
  const requests = await load(source, { router, routeNames });
  for (const origin of ["/search", "/", "/shelf", "/settings/tasks", "/settings/history"]) {
    reset([origin]);
    requests.requestBookDetailOpen({ kind: "search-result", result: { resultId: "one" } });
    const request = requests.consumeBookDetailOpenRequest();
    assert.equal(request.returnPath, origin);
    push("/book-detail");
    requests.returnFromBookDetail(request);
    assert.equal(router.currentRoute.value.fullPath, origin, "Details must return to their own entry point");
    assert.deepEqual(moves, [-1]);
    assert.equal(hooks.size, 0);
  }

  reset(["/shelf", "/search?type=audio"]);
  requests.requestBookDetailOpen({ kind: "search-result", result: { resultId: "two" } });
  const request = requests.consumeBookDetailOpenRequest();
  push("/book-detail"); push("/book-detail"); push("/book-detail");
  requests.returnFromBookDetail(request);
  assert.equal(router.currentRoute.value.fullPath, "/search?type=audio");
  assert.deepEqual(moves, [-3], "Return must skip duplicate detail entries");

  push("/book-detail");
  wrongPop = true;
  requests.returnFromBookDetail(request);
  assert.equal(router.currentRoute.value.fullPath, request.returnPath, "A stale native history target must not leave the user in details");
  assert.equal(hooks.size, 0);

  reset(["/search"]);
  router.options.history.state.position = undefined;
  requests.requestBookDetailOpen({ kind: "search-result", result: { resultId: "three" } });
  const withoutPosition = requests.consumeBookDetailOpenRequest();
  push("/book-detail");
  requests.returnFromBookDetail(withoutPosition);
  assert.equal(router.currentRoute.value.fullPath, "/search", "Missing history positions must fall back to the captured route");
  assert.deepEqual(moves, []);
  reset(["/book-detail", "/book-detail"]);
  requests.returnFromBookDetail(null);
  assert.equal(router.currentRoute.value.name, routeNames.shelf, "A direct detail visit must not return to another detail entry");

  reset(["/search"]);
  const firstId = requests.requestBookDetailOpen({ kind: "search-result", result: { resultId: "old" } });
  requests.requestBookDetailOpen({ kind: "search-result", result: { resultId: "new" } });
  requests.discardBookDetailOpenRequest(firstId);
  assert.equal(requests.consumeBookDetailOpenRequest().result.resultId, "new");

  const pageSource = await readFile(new URL("../src/features/discovery/SearchPage.vue", import.meta.url), "utf8");
  const tree = ts.createSourceFile("SearchPage.ts", parse(pageSource).descriptor.scriptSetup.content, ts.ScriptTarget.Latest, true);
  const handler = tree.statements.find(node => ts.isFunctionDeclaration(node) && node.name?.text === "openSearchResultDetails");
  let completeNavigation;
  let pushes = 0;
  const openingResultDetails = ref(false);
  router.push = () => { pushes++; return new Promise(resolve => { completeNavigation = resolve; }); };
  const { openSearchResultDetails } = await load(handler.getText(tree) + "\nexport { openSearchResultDetails };", {
    ...requests, router, routeNames, openingResultDetails, shelfBatchRecoveryRequired: ref(false),
    rememberResultPosition() {}, forgetResultPosition() {}, notify() {}, errorText: String,
  });
  const opening = openSearchResultDetails({ resultId: "clicked" });
  await openSearchResultDetails({ resultId: "double-clicked" });
  assert.equal(pushes, 1, "Repeated taps must not publish another detail request or navigate twice");
  assert.equal(requests.consumeBookDetailOpenRequest().result.resultId, "clicked");
  completeNavigation(undefined);
  await opening;
  assert.equal(openingResultDetails.value, false);
  navigate("/book-detail");
  await openSearchResultDetails({ resultId: "inactive-search" });
  assert.equal(pushes, 1, "An inactive cached search page must not open details");
  console.log("PASS: origin routes, duplicate detail history, stale native history, missing positions, handoff ownership, and repeated search taps.");
} finally {
  delete globalThis.__detailNavigationBindings;
}
