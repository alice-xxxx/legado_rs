import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import ts from "typescript";
import { ref } from "vue";
import { parse } from "@vue/compiler-sfc";

async function load(source, bindings) {
  globalThis.__readerInteractionBindings = bindings;
  const { outputText } = ts.transpileModule(source, {
    compilerOptions: { module: ts.ModuleKind.ESNext, target: ts.ScriptTarget.ES2022 },
    transformers: { before: [context => root => ts.visitEachChild(root,
      node => ts.isImportDeclaration(node) ? undefined : node, context)] },
  });
  const prelude = `const { ${Object.keys(bindings).join(", ")} } = globalThis.__readerInteractionBindings;\n`;
  return import(`data:text/javascript;base64,${Buffer.from(prelude + outputText).toString("base64")}`);
}

try {
  const interactionSource = await readFile(new URL("../src/features/reader/useReaderPageInteraction.ts", import.meta.url), "utf8");
  const { useReaderPageInteraction } = await load(interactionSource, {
    openedBook: ref(null), settings: ref({ reader: {} }),
    window: { innerWidth: 600 }, Node: { ELEMENT_NODE: 1 },
  });
  const reader = {
    viewport: ref(null), chapterIndex: ref(0), pageCount: ref(1),
    visible: ref(true), busy: ref(false), mediaChapter: ref(true),
    imagePage: ref(false), pdfPage: ref(null), sourceRefreshBusy: ref(false),
    menuOpen: ref(true), controlsOpen: ref(true), directoryOpen: ref(true),
    findQuery: ref(""), findMatches: ref([]), findIndex: ref(-1),
    getClickSuppressedUntil: () => 0,
  };
  const turns = [];
  const interaction = useReaderPageInteraction({
    reader, actions: { turnPage: direction => turns.push(direction) },
  });
  function surface(x, control = false) {
    interaction.onReaderSurfaceClick({ clientX: x, target: {
      nodeType: 1, closest: selector => selector === ".reader-main" || control ? {} : null,
    } });
  }
  for (const menu of [true, false]) {
    reader.menuOpen.value = menu;
    for (const x of [50, 300, 550]) surface(x);
    assert.equal(reader.menuOpen.value, menu, "Media surface clicks must not toggle reading chrome");
    assert.equal(reader.controlsOpen.value, true, "Media surface clicks must not close playback settings");
    assert.equal(reader.directoryOpen.value, true, "Media surface clicks must not close episode selection");
    assert.deepEqual(turns, [], "Media surface clicks must not switch episodes");
  }
  reader.mediaChapter.value = false;
  surface(300);
  assert.equal(reader.menuOpen.value, true, "Text reader center taps still show the reading menu");
  surface(300, true);
  assert.equal(reader.menuOpen.value, true, "Control clicks must not toggle the reading menu");
  surface(50);
  assert.equal(reader.menuOpen.value, false, "Blank taps dismiss reading chrome before turning pages");
  assert.deepEqual(turns, [], "Dismissing reading chrome must not turn a page");
  surface(50);
  surface(550);
  assert.deepEqual(turns, [-1, 1], "Text reader edge taps still turn pages");
  surface(300);
  assert.equal(reader.menuOpen.value, true);
  surface(300);
  assert.equal(reader.menuOpen.value, false);
  assert.equal(reader.controlsOpen.value, false);
  assert.equal(reader.directoryOpen.value, false);

  const hostSource = await readFile(new URL("../src/features/reader/ReaderHost.vue", import.meta.url), "utf8");
  const script = parse(hostSource).descriptor.scriptSetup.content;
  const tree = ts.createSourceFile("ReaderHost.ts", script, ts.ScriptTarget.Latest, true);
  const keyHandler = tree.statements.find(node => ts.isFunctionDeclaration(node) && node.name?.text === "onGlobalKeydown");
  assert.ok(keyHandler, "Reader keyboard handler must exist");
  const keys = [];
  let ignored = false;
  let blocked = false;
  const readerFeatures = {
    readerVisible: ref(true), readingMediaChapter: ref(true),
    bookmarkEditorOpen: ref(false), dictionaryOpen: ref(false),
    readerDirectoryOpen: ref(false), readerControlsOpen: ref(false),
    closeBookmarkEditor() { this.bookmarkEditorOpen.value = false; keys.push("bookmark-close"); },
    closeReaderDictionary() { this.dictionaryOpen.value = false; keys.push("dictionary-close"); },
    shouldIgnoreReaderKeydown: () => ignored,
    isReaderMediaControlBlocked: () => blocked,
    toggleReaderMediaPlayback: () => keys.push("play"),
    skipReaderMedia: seconds => keys.push(seconds),
    turnPage: direction => keys.push(`page:${direction}`),
  };
  const bookSourceSwitch = {
    bookSourceSwitchOpen: false,
    closeBookSourceSwitch() { this.bookSourceSwitchOpen = false; },
  };
  const { onGlobalKeydown } = await load(keyHandler.getText(tree) + "\nexport { onGlobalKeydown };", {
    readerFeatures, bookSourceSwitch, document: {}, router: { push: () => {} }, routeNames: { search: "search" },
    closeReaderForNavigation: () => keys.push("close"),
  });
  function key(value) {
    onGlobalKeydown({ key: value, preventDefault() {} });
  }
  key(" "); key("ArrowLeft"); key("ArrowRight"); key("PageDown"); key("PageUp");
  assert.deepEqual(keys, ["play", -10, 10], "Media keyboard controls must play/seek, never change episodes");
  blocked = true;
  key(" "); key("ArrowRight");
  assert.deepEqual(keys, ["play", -10, 10], "Blocked media must not accept playback shortcuts");
  blocked = false;
  ignored = true;
  key(" "); key("ArrowRight");
  assert.deepEqual(keys, ["play", -10, 10], "Native player and input keyboard handling must be preserved");
  ignored = false;
  key("Escape");
  readerFeatures.readingMediaChapter.value = false;
  key(" "); key("ArrowLeft");
  assert.deepEqual(keys, ["play", -10, 10, "close", "page:1", "page:-1"]);
  keys.length = 0;
  ignored = true;
  readerFeatures.bookmarkEditorOpen.value = true;
  key("Escape");
  assert.equal(readerFeatures.bookmarkEditorOpen.value, false);
  assert.deepEqual(keys, ["bookmark-close"], "Escape in a bookmark textarea dismisses the dialog, keeping the reader open");
  readerFeatures.dictionaryOpen.value = true;
  key("Escape");
  assert.equal(readerFeatures.dictionaryOpen.value, false);
  assert.deepEqual(keys, ["bookmark-close", "dictionary-close"]);
  bookSourceSwitch.bookSourceSwitchOpen = true;
  key("Escape");
  assert.equal(bookSourceSwitch.bookSourceSwitchOpen, false, "Escape dismisses source selection, keeping the reader open");
  for (const panel of ["readerDirectoryOpen", "readerControlsOpen"]) {
    readerFeatures[panel].value = true;
    key("Escape");
    assert.equal(readerFeatures[panel].value, false, "Escape dismisses the active panel even if an input has focus");
  }
  assert.deepEqual(keys, ["bookmark-close", "dictionary-close"], "Panel dismissal must not exit the reader");
  console.log("PASS: media clicks do not toggle menus or episodes; playback shortcuts and text reader navigation remain correct.");
} finally {
  delete globalThis.__readerInteractionBindings;
}
