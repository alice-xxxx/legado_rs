import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import ts from "typescript";
import { computed, nextTick, ref, shallowRef, watch } from "vue";

const mounted = [], activated = [], deactivated = [], unmounted = [];
const listeners = new Map();
const frames = new Map();
let frameId = 0;
let listTop = 200;
const window = {
  innerWidth: 390, innerHeight: 640, scrollY: 0,
  addEventListener(name, listener) {
    if (!listeners.has(name)) listeners.set(name, new Set());
    listeners.get(name).add(listener);
  },
  removeEventListener(name, listener) { listeners.get(name)?.delete(listener); },
  scrollTo({ top }) { this.scrollY = top; },
};
let resize;
class ResizeObserver {
  constructor(callback) { resize = callback; }
  observe() {}
  disconnect() {}
}
globalThis.__searchWindowBindings = {
  computed, nextTick, ref, watch, window, ResizeObserver,
  document: { querySelector: () => ({ getBoundingClientRect: () => ({ height: 56 }) }) },
  onMounted: callback => mounted.push(callback), onActivated: callback => activated.push(callback),
  onDeactivated: callback => deactivated.push(callback), onBeforeUnmount: callback => unmounted.push(callback),
  requestAnimationFrame: callback => { const id = ++frameId; frames.set(id, callback); return id; },
  cancelAnimationFrame: id => frames.delete(id),
};
const source = await readFile(new URL("../src/features/discovery/useSearchResultWindow.ts", import.meta.url), "utf8");
const { outputText } = ts.transpileModule(source, {
  compilerOptions: { module: ts.ModuleKind.ESNext, target: ts.ScriptTarget.ES2022 },
  transformers: { before: [context => root => ts.visitEachChild(root,
    node => ts.isImportDeclaration(node) ? undefined : node, context)] },
});
const prelude = `const { ${Object.keys(globalThis.__searchWindowBindings).join(", ")} } = globalThis.__searchWindowBindings;\n`;
const { searchResultRange, SEARCH_RESULT_ROW_HEIGHT: rowHeight, useSearchResultWindow } =
  await import(`data:text/javascript;base64,${Buffer.from(prelude + outputText).toString("base64")}`);
function draw() {
  const pending = [...frames.values()];
  frames.clear();
  pending.forEach(callback => callback());
}
function scroll(top) {
  window.scrollY = top;
  listeners.get("scroll")?.forEach(callback => callback());
}

try {
  for (const count of [0, 1, 8, 1000, 50000]) {
    for (const offset of [-5000, -50, 0, 15, 10000, count * rowHeight - 70, count * rowHeight + 500]) {
      const range = searchResultRange(count, offset, 640);
      assert.ok(range.start >= 0 && range.end >= range.start && range.end <= count);
      assert.ok(range.end - range.start <= 21, "Rendered rows must be bounded by viewport size, not total result count");
      assert.equal(range.top + (range.end - range.start) * rowHeight + range.bottom, count * rowHeight,
        "Spacers must preserve the full list height and next-page marker position");
      const firstVisible = Math.max(0, Math.floor(offset / rowHeight));
      const lastVisible = Math.min(count, Math.ceil((offset + 640) / rowHeight));
      for (let index = firstVisible; index < lastVisible; index++) {
        assert.ok(index >= range.start && index < range.end, "Every visible row must be mounted");
      }
    }
  }
  const original = Array.from({ length: 50000 }, (_, index) => ({ resultId: `result-${index}` }));
  const results = shallowRef(original);
  const element = ref({ getBoundingClientRect: () => ({ top: listTop - window.scrollY }), closest: () => ({}) });
  const list = useSearchResultWindow(results, element);
  await mounted[0]();
  await activated[0]();
  assert.equal(listeners.get("scroll").size, 1, "KeepAlive activation must not duplicate listeners");
  assert.ok(list.renderedResults.value.length <= 21);

  for (let i = 0; i < 40; i++) scroll(listTop + 20000 * rowHeight + 17);
  assert.equal(frames.size, 1, "Scroll layout reads must be coalesced to one animation frame");
  draw();
  assert.equal(list.range.value.start, 19994);
  assert.equal(list.renderedResults.value[0].resultId, "result-19994");
  assert.equal(list.renderedResults.value[6].resultId, "result-20000");

  list.rememberPosition();
  deactivated[0]();
  assert.equal(listeners.get("scroll").size, 0);
  results.value = [...Array.from({ length: 7 }, (_, index) => ({ resultId: `new-${index}` })), ...original];
  await nextTick();
  assert.equal(frames.size, 0, "Live search batches must not measure an inactive detail-page search view");
  window.scrollY = 0;
  await activated[0]();
  assert.equal(window.scrollY, listTop + 20007 * rowHeight + 17,
    "Returning from details must restore the same result even when Rust reordered or inserted results");

  window.innerWidth = 320;
  window.innerHeight = 400;
  listTop = 450;
  resize();
  draw();
  assert.equal(list.viewportWidth.value, 320);
  assert.ok(list.renderedResults.value.length <= 18);

  results.value = original.slice(0, 3);
  list.scrollToStart();
  await nextTick();
  draw();
  assert.equal(window.scrollY, listTop - 56);
  assert.deepEqual(list.renderedResults.value.map(result => result.resultId), ["result-0", "result-1", "result-2"]);
  assert.equal(list.range.value.bottom, 0);
  results.value = [];
  await nextTick();
  draw();
  assert.deepEqual(list.renderedResults.value, []);
  unmounted[0]();
  assert.equal(frames.size, 0);
  assert.equal(listeners.get("scroll").size, 0);
  assert.equal(listeners.get("resize").size, 0);
  console.log("PASS: 50,000-result window stays bounded, preserves list height, coalesces scroll work, restores details, and handles resize/filter/reset/KeepAlive cleanup.");
} finally {
  delete globalThis.__searchWindowBindings;
}
