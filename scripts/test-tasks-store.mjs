import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import ts from "typescript";
import { computed, ref, shallowRef, watch } from "vue";

// Execute the production modules with an in-memory native event/resource boundary.
const bindings = globalThis.__taskStoreRegressionBindings = {};
async function loadModule(path, name, imports) {
  bindings[name] = imports;
  const source = await readFile(new URL(path, import.meta.url), "utf8");
  const { outputText } = ts.transpileModule(source, {
    compilerOptions: { module: ts.ModuleKind.ESNext, target: ts.ScriptTarget.ES2020 },
    transformers: { before: [context => root => ts.visitEachChild(root,
      node => ts.isImportDeclaration(node) ? undefined : node, context)] },
  });
  const prelude = `const { ${Object.keys(imports).join(", ")} } = globalThis.__taskStoreRegressionBindings[${JSON.stringify(name)}];\n`;
  return import(`data:text/javascript;base64,${Buffer.from(prelude + outputText).toString("base64")}`);
}

const listeners = new Map();
const reads = [];
let readCount = 0;
let revision = 0;
let commandResult;
const resource = { resourceId: "tasks.json", src: "https://fixture/tasks.json", format: "json", contentType: "application/json" };
const task = (id, status = "running") => ({
  id, status, kind: "downloadChapters", page: 1, fromIndex: 0,
  completed: 0, total: 10, createdAtMs: 1, updatedAtMs: 2,
});
const document = (...tasks) => ({ schemaVersion: 1, tasks });
const deferred = () => {
  let resolve;
  const promise = new Promise(complete => { resolve = complete; });
  return { promise, resolve };
};
const flush = () => new Promise(resolve => setImmediate(resolve));
function emit(event, payload) {
  for (const handler of listeners.get(event) ?? []) handler({ payload });
}
const imports = {
  computed, ref, shallowRef,
  listen: async (event, handler) => {
    const handlers = listeners.get(event) ?? new Set();
    handlers.add(handler);
    listeners.set(event, handlers);
    return () => { handlers.delete(handler); if (!handlers.size) listeners.delete(event); };
  },
  command: async name => name === "tasks_resource" ? { resource } : commandResult,
  readResource: async () => {
    readCount++;
    assert.ok(reads.length, "Every resource read must be expected by the scenario");
    const next = reads.shift();
    if (next instanceof Error) throw next;
    return next;
  },
  getFrontendRestoreRevision: () => revision,
  errorText: error => error.message,
  notify: () => {},
};
const store = await loadModule("../src/features/tasks/tasksStore.ts", "store", imports);
const api = await loadModule("../src/api/tasks.ts", "api", { ...imports, ...store });
try {
  reads.push(document(task("one")));
  const initialized = store.initializeTaskStore();
  assert.equal(store.initializeTaskStore(), initialized);
  await initialized;
  assert.equal(listeners.size, 4);
  assert.ok([...listeners.values()].every(handlers => handlers.size === 1));
  await Promise.all(Array.from({ length: 5 }, () => store.ensureTasksLoaded()));
  assert.equal(readCount, 1, "Opening consumers must reuse the loaded document");

  const readsBeforeBurst = readCount;
  reads.push(document(task("one")));
  await Promise.all([store.refreshTasks(resource), store.refreshTasks(resource)]);
  assert.equal(readCount, readsBeforeBurst + 1, "Simultaneous notifications must share one read");

  commandResult = { taskId: "one", resource, reused: false };
  reads.push(document(task("one", "paused")));
  const response = await api.taskCommand("pause_task", { taskId: "one" });
  assert.equal(response.task, store.tasks.value[0]);
  assert.equal(response.task.status, "paused");
  assert.equal(readCount, readsBeforeBurst + 2, "Task commands must hydrate through the shared store once");

  const earlier = deferred();
  reads.push(earlier.promise);
  const earlierRead = store.refreshTasks(resource);
  await flush();
  reads.push(document(task("one", "running")));
  await store.refreshTasks(resource);
  earlier.resolve(document(task("one", "paused")));
  await earlierRead;
  assert.equal(store.tasks.value[0].status, "running", "An earlier read must not overwrite a newer response");

  reads.push(document(task("one", "completed")));
  emit("task-updated", { taskId: "one", resource });
  await flush();
  assert.equal(store.latestTaskUpdate.value.status, "completed");

  const observedTasks = [];
  const stopObserving = watch(store.latestTaskUpdate, task => { if (task) observedTasks.push(task.id); }, { flush: "sync" });
  reads.push(document(task("one", "completed"), task("two", "completed")));
  emit("task-updated", { taskId: "one", resource });
  emit("task-updated", { taskId: "two", resource });
  await flush();
  stopObserving();
  assert.deepEqual(new Set(observedTasks), new Set(["one", "two"]), "A notification burst must deliver every task to business observers");

  emit("task-recovery-required", { id: "one", status: "recoveryRequired", error: "rollback incomplete" });
  reads.push(document(task("one", "completed")));
  emit("resource-updated", { kind: "tasks", resource });
  await flush();
  assert.equal(store.tasks.value[0].status, "recoveryRequired", "A stale resource must retain recovery state");
  assert.equal(store.taskRecoveryRequired.value, true);
  emit("task-recovery-required", { id: "unknown", status: "recoveryRequired" });
  assert.ok(!store.tasks.value.some(task => task.id === "unknown"), "Incomplete recovery summaries must not become invalid task records");

  const beforeRestore = deferred();
  reads.push(beforeRestore.promise);
  emit("task-updated", { taskId: "one", resource });
  await flush();
  reads.push(document(task("restored")));
  revision++;
  emit("app-state-updated", {});
  await flush();
  beforeRestore.resolve(document(task("one", "failed")));
  await flush();
  assert.equal(store.tasks.value[0].id, "restored");
  assert.equal(store.latestTaskUpdate.value, null, "Late events must not notify observers after restore");
  assert.equal(store.taskRecoveryRequired.value, false);

  const lateCommand = deferred();
  commandResult = lateCommand.promise;
  const result = api.taskCommand("retry_task", { taskId: "one" });
  const rejected = assert.rejects(result, /失效/);
  reads.push(document(task("restored-again")));
  revision++;
  emit("app-state-updated", {});
  await flush();
  const readsAfterRestore = readCount;
  lateCommand.resolve({ taskId: "one", resource });
  await rejected;
  assert.equal(readCount, readsAfterRestore, "A late command must not read its old descriptor");

  reads.push(new Error("unavailable"));
  await assert.rejects(store.refreshTasks(resource), /unavailable/);
  assert.equal(store.tasksError.value, "unavailable");
  reads.push(document(task("healthy")));
  await store.refreshTasks(resource);
  assert.equal(store.tasksError.value, "");

  const afterDispose = deferred();
  reads.push(afterDispose.promise);
  const pending = store.refreshTasks(resource);
  await flush();
  store.disposeTaskStore();
  afterDispose.resolve(document(task("disposed")));
  await pending;
  assert.equal(listeners.size, 0);
  assert.equal(store.tasks.value[0].id, "healthy");
  console.log("Task store passed: one initialization/read owner, shared command results, recovery retention, restore races, error recovery and listener disposal.");
} finally {
  store.disposeTaskStore();
  delete globalThis.__taskStoreRegressionBindings;
}
