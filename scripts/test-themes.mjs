import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import ts from "typescript";
import { computed, ref, watch } from "vue";

const bindings = globalThis.__themeRegressionBindings = {};
async function load(path, name, imports) {
  bindings[name] = imports;
  const source = await readFile(new URL(path, import.meta.url), "utf8");
  const { outputText } = ts.transpileModule(source, {
    compilerOptions: { module: ts.ModuleKind.ESNext, target: ts.ScriptTarget.ES2020 },
    transformers: { before: [context => root => ts.visitEachChild(root,
      node => ts.isImportDeclaration(node) ? undefined : node, context)] },
  });
  const prelude = `const { ${Object.keys(imports).join(", ")} } = globalThis.__themeRegressionBindings[${JSON.stringify(name)}];\n`;
  return import(`data:text/javascript;base64,${Buffer.from(prelude + outputText).toString("base64")}`);
}

const { defaultSettings } = await load("../src/features/settings/settingsDefaults.ts", "defaults", {});
const settings = ref(structuredClone(defaultSettings));
const handlers = new Set();
const media = { matches: false, addEventListener: (_, handler) => handlers.add(handler), removeEventListener: (_, handler) => handlers.delete(handler) };
const window = { matchMedia: () => media };
const document = { documentElement: { dataset: {} } };
const theme = await load("../src/features/settings/themeState.ts", "theme", { computed, ref, watch, settings, window, document });
const dispose = theme.initializeTheme();
const display = () => ({ ...document.documentElement.dataset });
function system(dark) { media.matches = dark; for (const handler of handlers) handler({ matches: dark }); }
try {
  assert.deepEqual(display(), { theme: "light", readerTheme: "paper" });
  const preferences = JSON.stringify(settings.value);
  let saves = 0;
  const stopCounting = watch(settings, () => saves++, { deep: true, flush: "sync" });
  system(true);
  assert.deepEqual(display(), { theme: "dark", readerTheme: "dark" });
  system(false);
  assert.deepEqual(display(), { theme: "light", readerTheme: "paper" });
  assert.equal(saves, 0, "System changes must not save configuration or overwrite the selected mode");
  assert.equal(JSON.stringify(settings.value), preferences);
  theme.setReaderTheme("sepia");
  assert.equal(JSON.stringify(settings.value), preferences, "System mode must not edit reading profiles");
  settings.value.theme = "light";
  theme.setReaderTheme("sepia");
  assert.deepEqual(display(), { theme: "light", readerTheme: "sepia" });
  settings.value.theme = "dark";
  theme.setReaderTheme("paper");
  assert.deepEqual(display(), { theme: "dark", readerTheme: "paper" });
  system(false);
  assert.deepEqual(display(), { theme: "dark", readerTheme: "paper" }, "Explicit mode must ignore system changes");
  settings.value.theme = "light";
  assert.deepEqual(display(), { theme: "light", readerTheme: "sepia" });
  assert.deepEqual(settings.value.reader.themes, { light: "sepia", dark: "paper" });
  settings.value.theme = "system";
  system(true);
  assert.deepEqual(display(), { theme: "dark", readerTheme: "paper" });
  settings.value = structuredClone(defaultSettings);
  assert.deepEqual(display(), { theme: "dark", readerTheme: "dark" }, "Rust readback or restore must update both themes");
  stopCounting();
} finally {
  dispose();
  assert.equal(handlers.size, 0);
  delete globalThis.__themeRegressionBindings;
}
console.log("PASS: day/night profiles, system switching, global JSON replacement, and read-only system appearance.");
