import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import ts from "typescript";
import { ref, watch, nextTick } from "vue";

const originals = Object.fromEntries(["navigator", "document", "getComputedStyle", "innerWidth", "innerHeight"]
  .map(key => [key, Object.getOwnPropertyDescriptor(globalThis, key)]));
let callback, guard, unregisters = 0, exits = 0, backs = 0, pushes = 0;
let layers = [], localButtons = [], pageButtons = [], top = null;
const route = (path, query = {}) => ({ path, query, hash: "", fullPath: path + (Object.keys(query).length ? "?" + new URLSearchParams(query) : "") });
const currentRoute = ref(route("/"));
const router = {
  currentRoute, options: { history: { state: { current: "/", back: null } } },
  beforeEach(fn) { guard = fn; return () => { guard = undefined; }; },
  async push(to) { pushes++; this.options.history.state.back = currentRoute.value.fullPath; currentRoute.value = route(to.path, to.query); this.options.history.state.current = currentRoute.value.fullPath; },
  back() { backs++; },
};
function button(click, disabled = false) {
  return { isConnected: true, disabled, getClientRects: () => [1], closest: () => null, click };
}
function layer(buttons, children = []) {
  const element = { isConnected: true, getClientRects: () => [1], closest: () => null,
    getBoundingClientRect: () => ({ left: 0, top: 0, width: 390, height: 844 }),
    contains: target => target === element || children.includes(target) || buttons.includes(target),
    querySelectorAll: () => buttons };
  return element;
}
function setPlatform(userAgent) {
  Object.defineProperty(globalThis, "navigator", { configurable: true, value: { userAgent, maxTouchPoints: 1 } });
}

try {
  Object.defineProperty(globalThis, "document", { configurable: true, value: {
    querySelectorAll: selector => selector.includes("data-back-layer") ? layers : selector.includes("data-back-local") ? localButtons : pageButtons,
    elementFromPoint: () => top,
  } });
  Object.defineProperty(globalThis, "getComputedStyle", { configurable: true, value: () => ({ visibility: "visible" }) });
  Object.defineProperty(globalThis, "innerWidth", { configurable: true, value: 390 });
  Object.defineProperty(globalThis, "innerHeight", { configurable: true, value: 844 });
  setPlatform("Android");
  globalThis.__nativeBackBindings = {
    router, watch, isTauri: () => true,
    onBackButtonPress: async fn => { callback = fn; return { unregister: async () => { unregisters++; } }; },
    exit: async code => { assert.equal(code, 0); exits++; },
  };
  const source = await readFile(new URL("../src/app/nativeBack.ts", import.meta.url), "utf8");
  const { outputText } = ts.transpileModule(source, {
    compilerOptions: { module: ts.ModuleKind.ESNext, target: ts.ScriptTarget.ES2022 },
    transformers: { before: [context => root => ts.visitEachChild(root,
      node => ts.isImportDeclaration(node) ? undefined : node, context)] },
  });
  const prelude = `const {${Object.keys(globalThis.__nativeBackBindings).join(",")}} = globalThis.__nativeBackBindings;\n`;
  const runtime = await import(`data:text/javascript;base64,${Buffer.from(prelude + outputText).toString("base64")}`);
  const dispose = runtime.installNativeBackNavigation();
  await nextTick();

  await callback({ canGoBack: false });
  assert.equal(exits, 1, "Back at the root closes the Android activity");
  let pageReturns = 0;
  pageButtons = [button(() => pageReturns++)];
  await callback({ canGoBack: false });
  assert.equal(pageReturns, 1, "A direct subpage still has a return action without history");
  router.options.history.state.back = "/settings";
  await callback({ canGoBack: true });
  assert.equal(backs, 1, "Normal Back traverses history rather than pushing the parent again");

  let closes = 0, saveSucceeds = false;
  const reader = ref(true);
  const disposeReader = runtime.installReaderNativeBack({ visible: reader, close: async () => { closes++; if (saveSucceeds) reader.value = false; } });
  let dismisses = 0;
  const close = button(() => dismisses++);
  layers = [layer([close])]; top = close;
  await callback({ canGoBack: true });
  assert.equal(dismisses, 1); assert.equal(closes, 0, "A modal consumes Back before the reader");
  close.disabled = true;
  await callback({ canGoBack: true });
  assert.equal(dismisses, 1); assert.equal(backs, 1, "Disabled modal actions cannot leak Back to the page");
  layers = []; top = null;
  await callback({ canGoBack: true });
  assert.equal(reader.value, true); assert.equal(backs, 1, "Failed saves keep reading and its route intact");
  router.options.history.state.current = "/settings";
  assert.equal(await guard(route("/settings"), route("/shelf")), false, "Native history navigation also waits for reader closing");
  saveSucceeds = true;
  await callback({ canGoBack: true });
  assert.equal(reader.value, false); assert.equal(backs, 1);

  let cancelEditor = 0;
  localButtons = [button(() => cancelEditor++)];
  await callback({ canGoBack: true });
  assert.equal(cancelEditor, 1, "In-page editor Back invokes its existing discard guard");
  localButtons = []; disposeReader(); dispose(); await nextTick();
  assert.equal(unregisters, 1); assert.equal(guard, undefined);
  await callback({ canGoBack: false }); assert.equal(exits, 1, "Disposed callbacks ignore late events");

  setPlatform("iPhone"); currentRoute.value = route("/shelf"); router.options.history.state.current = "/shelf";
  const iosReader = ref(false);
  const disposeIosReader = runtime.installReaderNativeBack({ visible: iosReader, close: async () => { iosReader.value = false; } });
  const disposeIos = runtime.installNativeBackNavigation();
  iosReader.value = true; await nextTick();
  assert.equal(pushes, 1); assert.equal(currentRoute.value.fullPath, "/shelf?_reader=1");
  assert.equal(router.options.history.state.back, "/shelf", "An iOS reader receives a real native history entry");
  const before = backs;
  router.options.history.state.current = "/shelf";
  assert.equal(await guard(route("/shelf"), route("/shelf", { _reader: "1" })), undefined);
  await nextTick(); assert.equal(backs, before, "Native pop closes the reader without requesting another pop");
  assert.deepEqual(await guard(route("/shelf", { _reader: "1" }), route("/shelf")),
    { path: "/shelf", query: {}, hash: "", replace: true }, "Forward cannot restore a stale reader marker");
  disposeIosReader(); disposeIos();
  console.log("PASS: native Back routing, overlay priority, disabled controls, progress failures, editor guards, iOS history and listener cleanup.");
} finally {
  delete globalThis.__nativeBackBindings;
  for (const [key, descriptor] of Object.entries(originals)) {
    if (descriptor) Object.defineProperty(globalThis, key, descriptor);
    else delete globalThis[key];
  }
}
