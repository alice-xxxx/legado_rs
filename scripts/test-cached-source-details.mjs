import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import ts from "typescript";
import { computed, ref } from "vue";

const openedBook = ref(null);
const searchCalls = [], prepared = [], changes = [];
const candidates = ref([
  { id: "audio-a", searchType: "audio" }, { id: "audio-b", searchType: "audio" },
  { id: "text-a", searchType: "text" }, { id: "text-b", searchType: "text" },
  { id: "image-a", searchType: "image" }, { id: "video-a", searchType: "video" },
]);
const alternate = (sourceId, title = "Song", author = "Singer") => ({ sourceId, sourceName: sourceId, title, author });
const book = (id, contentType, sourceId, alternatives) => ({
  id, contentType, sourceId, alternatives, title: "Song", author: "Singer",
  mediaType: contentType === "audio" || contentType === "video" ? contentType : undefined,
  canChangeSource: true, chapters: [], chapterCount: 0,
});
const bindings = globalThis.__cachedSourceBindings = {
  computed, ref, openedBook, bookSourceCandidates: candidates, tasks: ref([]),
  bookContentType: book => book?.contentType,
  shelfBatchRecoveryRequired: ref(false), shelfBatchRecoveryTitle: ref(""), shelfBatchRecoveryMessage: ref(""),
  libraryType: () => undefined, replaceLibraryContent() {}, clearOpenedBook: () => { openedBook.value = null; },
  cancelTask: async () => {},
  prepareSearchResultBook: async () => { throw new Error("Cached alternatives must not use transient search result IDs"); },
  prepareAlternativeBook: async (...args) => { prepared.push(args); return { book: { id: "prepared" } }; },
  loadBookPreview: async () => ({ id: "prepared" }),
  changeBookSource: async (...args) => { changes.push(args); return { book: book("prepared", "audio", "audio-b", [alternate("audio-a")]) }; },
  switchOpenedBook: async value => { openedBook.value = value; return openedBook.value; },
  changeChapterSource: async () => {},
  searchBookSourceCandidates: async (...args) => {
    searchCalls.push(args);
    return { taskId: "task-one", task: { id: "task-one", kind: "bookSourceCandidates", status: "queued" } };
  },
  getSearchTaskResults: async () => ({ results: [{ resultId: "live", sourceId: "text-b", title: "Search response title" }],
    errors: [{ sourceId: "text-b", message: "test" }], complete: true }),
};
try {
  const source = await readFile(new URL("../src/features/sources/useBookSourceSwitch.ts", import.meta.url), "utf8");
  const { outputText } = ts.transpileModule(source, {
    compilerOptions: { module: ts.ModuleKind.ESNext, target: ts.ScriptTarget.ES2022 },
    transformers: { before: [context => root => ts.visitEachChild(root,
      node => ts.isImportDeclaration(node) ? undefined : node, context)] },
  });
  const prelude = `const { ${Object.keys(bindings).join(", ")} } = globalThis.__cachedSourceBindings;\n`;
  const { useBookSourceSwitch } = await import(`data:text/javascript;base64,${Buffer.from(prelude + outputText).toString("base64")}`);
  openedBook.value = book("audio-book", "audio", "audio-a", [alternate("audio-b"), alternate("audio-c", "Song", "")]);
  const state = useBookSourceSwitch({
    readingBook: ref(null), readerVisible: ref(false), readingChapterIndex: ref(0), readerPageIndex: ref(0),
    readingContent: ref(null), readerProgressDirty: ref(false), readerControlsOpen: ref(false),
    saveCurrentProgress: async () => {}, loadReaderChapter: async () => {},
    notify() {}, errorText: String, getRestoreRevision: () => 0,
  });
  assert.equal(state.bookSourceSwitchOpen.value, false);
  assert.equal(state.bookSourceSwitchBook.value.id, "audio-book");
  assert.deepEqual(state.bookSourceCandidateResults.value.map(candidate => candidate.sourceId), ["audio-b", "audio-c"],
    "The sources tab must show Rust's cached JSON without first opening the switch dialog");
  assert.equal(state.bookSourceCandidateResults.value[0].alternativeIndex, 0);
  assert.equal(state.bookSourceCandidateResults.value[1].requiresIdentityConfirmation, true);
  assert.deepEqual(searchCalls, [], "Reading cached alternatives must not start another search");
  assert.deepEqual(state.bookSourceEligibleSources.value.map(source => source.id), ["audio-b"]);

  openedBook.value = { ...openedBook.value, alternatives: [alternate("audio-b", "Updated cached title")] };
  assert.equal(state.bookSourceCandidateResults.value[0].title, "Updated cached title", "Rust JSON updates must refresh the source list");
  await state.openBookSourceSwitchForBook(openedBook.value);
  state.selectBookSourceCandidate(state.bookSourceCandidateResults.value[0]);
  state.bookSourceSwitchConfirmOpen.value = true;
  await state.confirmBookSourceChange();
  assert.deepEqual(prepared, [["audio-book", 0, "audio-b"]], "Cached switching must preserve the Rust alternative index and expected source ID");
  assert.deepEqual(changes, [["audio-book", "prepared", false]]);
  assert.equal(state.bookSourceSwitchOpen.value, false);
  assert.equal(state.bookSourceCandidateResults.value[0].sourceId, "audio-a", "The new JSON must replace the old source list after a switch");

  openedBook.value = book("text-book", "text", "text-a", [alternate("text-b", "Cached text title")]);
  assert.equal(state.bookSourceSwitchBook.value.id, "text-book");
  assert.equal(state.bookSourceCandidateResults.value[0].title, "Cached text title", "Another detail page must not inherit stale switch candidates");
  assert.deepEqual(state.bookSourceEligibleSources.value.map(source => source.id), ["text-b"]);
  await state.openBookSourceSwitchForBook(openedBook.value);
  await state.startBookSourceCandidateSearch();
  assert.deepEqual(searchCalls[0], ["text-book", ["text-b"], undefined, 1], "Searching more must send only other sources of the same type");
  await state.syncBookSourceCandidateTask({ id: "task-one", kind: "bookSourceCandidates", status: "completed" });
  assert.equal(state.bookSourceCandidateResults.value[0].title, "Search response title");
  state.closeBookSourceSwitch();
  assert.equal(state.bookSourceCandidateResults.value[0].title, "Cached text title",
    "The detail page must use cached book JSON, not retain a parallel search-response list");
  openedBook.value = book("other-book", "image", "image-a", []);
  assert.deepEqual(state.bookSourceCandidateResults.value, []);
  assert.deepEqual(state.bookSourceCandidateErrors.value, [], "Candidate errors must not leak into another book's details");
  console.log("PASS: cached JSON source display, refresh, alternative-index switching, same-type search, and stale-session isolation.");
} finally {
  delete globalThis.__cachedSourceBindings;
}
