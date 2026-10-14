import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import ts from "typescript";
import { computed, nextTick, ref, shallowRef, watch } from "vue";

const requests = [], cancellations = [], releases = [];
const hooks = new Set();
let delayedStart = null;
const deferred = () => {
  let resolve;
  const promise = new Promise(done => { resolve = done; });
  return { promise, resolve };
};
const flush = () => new Promise(resolve => setImmediate(resolve));
const routeNames = { search: "search", bookDetail: "book-detail", shelf: "shelf", home: "home", sources: "sources" };
const candidates = ref([
  { id: "text-1", searchType: "text" },
  { id: "text-2", searchType: "text" },
  { id: "image-1", searchType: "image" },
  { id: "audio-1", searchType: "audio" },
  { id: "video-1", searchType: "video" },
]);
globalThis.__searchStateTestBindings = {
  computed, ref, shallowRef, watch, routeNames,
  router: { afterEach: hook => { hooks.add(hook); return () => hooks.delete(hook); } },
  listen: async () => () => {},
  startSearch: async (type, sourceIds, keyword, page, precision, onResults) => {
    const taskId = "task-" + (requests.length + 1);
    requests.push({ type, sourceIds, keyword, page, precision, onResults, taskId });
    const delayed = delayedStart;
    delayedStart = null;
    if (delayed) await delayed.promise;
    return { taskId };
  },
  stopSearch: async id => { cancellations.push(id); },
  releaseSearchResults: async id => { releases.push(id); },
  bookSourceCandidates: candidates,
  shelfBatchRecoveryRequired: ref(false),
  notify: message => { throw new Error(message); },
};
const source = await readFile(new URL("../src/features/discovery/searchState.ts", import.meta.url), "utf8");
const { outputText } = ts.transpileModule(source, {
  compilerOptions: { module: ts.ModuleKind.ESNext, target: ts.ScriptTarget.ES2020 },
  transformers: { before: [context => root => ts.visitEachChild(root,
    node => ts.isImportDeclaration(node) ? undefined : node, context)] },
});
const prelude = "const { " + Object.keys(globalThis.__searchStateTestBindings).join(", ") + " } = globalThis.__searchStateTestBindings;\n";
const state = await import("data:text/javascript;base64," + Buffer.from(prelude + outputText).toString("base64"));
const latest = () => requests.at(-1);
const book = (resultId, title) => ({ resultId, title, sourceId: "text-1", sourceName: "Source" });
const ids = () => state.searchResults.value.map(book => book.resultId);
function emit(request, results, resultOrder, complete = false, hasMore = true) {
  request.onResults({ taskId: request.taskId, results, resultOrder, incremental: true,
    errors: [], completedSources: complete ? 2 : 0, totalSources: 2, complete, hasMore });
}
function navigate(from, to, failure) {
  for (const hook of hooks) hook({ name: to }, { name: from }, failure);
}

try {
  state.searchKeyword.value = "凡人";
  // Corrupt/stale selections must never send other types to Rust.
  state.selectedSourceIds.value = ["text-1", "text-2", "image-1", "audio-1"];
  await state.runSearch();
  const first = latest();
  assert.equal(first.precision, true);
  assert.deepEqual(first.sourceIds, ["text-1", "text-2"]);
  emit(first, [book("partial", "凡人修仙传"), book("exact", "凡人")], ["exact", "partial"]);
  assert.deepEqual(ids(), ["exact", "partial"]);
  emit(first, [], ["partial", "exact"], true);
  assert.deepEqual(ids(), ["partial", "exact"]);

  state.searchKeyword.value = "edited query";
  state.precisionSearch.value = false;
  const next = state.loadMoreSearchResults();
  const repeatedNext = state.loadMoreSearchResults();
  await Promise.all([next, repeatedNext]);
  assert.equal(requests.length, 2, "Repeated bottom gestures admit only one next-page request");
  assert.equal(latest().keyword, "凡人");
  assert.equal(latest().precision, true);
  assert.equal(latest().page, 2);
  emit(latest(), [book("new", "凡人续作")], ["exact", "partial", "new"], true, false);
  assert.deepEqual(ids(), ["exact", "partial", "new"]);
  await state.loadMoreSearchResults();
  assert.equal(requests.length, 2, "Exhausted searches never request another page");

  state.searchKeyword.value = "凡人";
  await state.runSearch();
  const broad = latest();
  assert.equal(broad.precision, false);
  emit(broad, [book("broad", "推荐书")], ["broad"], true);
  emit(first, [book("stale", "旧结果")], ["stale"], true);
  assert.deepEqual(ids(), ["broad"]);
  await state.loadMoreSearchResults();
  emit(latest(), [book("current", "当前结果")], ["current"], true);
  assert.deepEqual(ids(), ["current"], "Backend order drops retained IDs from a previous page");

  await state.runSearch();
  const detailSearch = latest();
  emit(detailSearch, [book("retained", "凡人")], ["retained"]);
  const stopsBeforeDetail = cancellations.length;
  navigate(routeNames.search, routeNames.bookDetail);
  navigate(routeNames.bookDetail, routeNames.bookDetail);
  navigate(routeNames.bookDetail, routeNames.search);
  assert.equal(cancellations.length, stopsBeforeDetail, "Opening details and returning retains live search");
  navigate(routeNames.search, routeNames.shelf, new Error("navigation prevented"));
  assert.equal(cancellations.length, stopsBeforeDetail, "Failed navigation must not stop search");
  navigate(routeNames.search, routeNames.shelf);
  await flush();
  assert.deepEqual(cancellations.slice(stopsBeforeDetail), [detailSearch.taskId]);
  assert.equal(state.searchBusy.value, false);
  emit(detailSearch, [book("late", "迟到的结果")], ["late"]);
  assert.deepEqual(ids(), ["retained"], "Stopped searches cannot add late results");
  await state.loadMoreSearchResults();
  assert.equal(latest(), detailSearch, "Leaving a search cannot silently resume it through paging");

  await state.runSearch();
  const awayFromDetail = latest();
  navigate(routeNames.search, routeNames.bookDetail);
  navigate(routeNames.bookDetail, routeNames.home);
  await flush();
  assert.equal(cancellations.at(-1), awayFromDetail.taskId, "Leaving result details ends the search flow");

  // The route can change before start_search has returned its task ID.
  const pendingStart = deferred();
  delayedStart = pendingStart;
  const pendingRun = state.runSearch();
  await flush();
  const lateStart = latest();
  navigate(routeNames.search, routeNames.shelf);
  pendingStart.resolve();
  await pendingRun;
  assert.equal(cancellations.at(-1), lateStart.taskId);
  assert.equal(state.searchBusy.value, false);

  // New requests must supersede queued commands, without late Rust admission.
  const slowStart = deferred();
  delayedStart = slowStart;
  const previousRun = state.runSearch();
  await flush();
  const old = latest();
  state.searchKeyword.value = "intermediate";
  const superseded = state.runSearch();
  state.searchKeyword.value = "latest";
  const newestRun = state.runSearch();
  const beforeResolve = requests.length;
  slowStart.resolve();
  await Promise.all([previousRun, superseded, newestRun]);
  assert.equal(requests.length, beforeResolve + 1);
  assert.equal(latest().keyword, "latest");
  assert.ok(releases.includes(old.taskId));
  emit(old, [book("stale", "旧结果")], ["stale"], true);
  assert.deepEqual(ids(), []);

  state.searchType.value = "image";
  await nextTick();
  assert.deepEqual(state.searchSources.value.map(source => source.id), ["image-1"]);
  assert.deepEqual(state.selectedSourceIds.value, ["image-1"]);
  await state.runSearch();
  assert.equal(latest().type, "image");
  assert.deepEqual(latest().sourceIds, ["image-1"]);
  state.resetSearch();
  emit(latest(), [book("stale-type", "错类型")], ["stale-type"], true);
  assert.deepEqual(ids(), []);
  console.log("Search state regression checks passed: ordering, paging, types, route exits, and start/stop races.");
} finally {
  state.disposeSearchState();
  assert.equal(hooks.size, 0);
  delete globalThis.__searchStateTestBindings;
}
