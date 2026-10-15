import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import ts from "typescript";

const source = await readFile(new URL("../src/api/resources.ts", import.meta.url), "utf8");
const { outputText } = ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.ESNext } });
const { readResource, readTextResource, ResourceHttpError, ResourceNetworkError } = await import(
  `data:text/javascript;base64,${Buffer.from(outputText + "\n//# sourceURL=resources-recovery-test.js").toString("base64")}`);
const originalFetch = globalThis.fetch;
const originalDocument = Object.getOwnPropertyDescriptor(globalThis, "document");
const document = new EventTarget();
document.visibilityState = "visible";
Object.defineProperty(globalThis, "document", { configurable: true, value: document });
const local = "http://127.0.0.1:1234/r/token/book.html";
let calls = 0;
try {
  globalThis.fetch = async () => {
    calls++;
    // A response can fail while reading its body, after fetch has succeeded.
    if (calls === 1) return { ok: true, json: async () => { throw new TypeError("Load failed"); } };
    return new Response('{"ok":true}');
  };
  assert.deepEqual(await readResource(local), { ok: true });
  assert.equal(calls, 2);

  calls = 0;
  globalThis.fetch = async () => { calls++; return new Response("invalid JSON"); };
  await assert.rejects(readResource(local), SyntaxError);
  assert.equal(calls, 1, "Parsing errors are not network failures");
  calls = 0;
  globalThis.fetch = async () => { calls++; return new Response("", { status: 404 }); };
  await assert.rejects(readResource(local), error => error instanceof ResourceHttpError && error.status === 404);
  assert.equal(calls, 1, "Missing resources must not be retried as a lifecycle problem");

  document.visibilityState = "hidden";
  calls = 0;
  globalThis.fetch = async () => {
    if (++calls === 1) throw new TypeError("Load failed");
    return new Response("chapter");
  };
  const suspended = readTextResource({ src: local });
  await new Promise(resolve => setTimeout(resolve, 300));
  assert.equal(calls, 1, "Retries wait for foreground instead of exhausting while suspended");
  document.visibilityState = "visible";
  document.dispatchEvent(new Event("visibilitychange"));
  assert.equal(await suspended, "chapter");

  document.visibilityState = "hidden";
  const abort = new AbortController();
  globalThis.fetch = async () => { throw new TypeError("Load failed"); };
  const cancelled = readResource(local, abort.signal);
  await Promise.resolve();
  abort.abort();
  await assert.rejects(cancelled, error => error.name === "AbortError");
  document.visibilityState = "visible";

  calls = 0;
  globalThis.fetch = async () => { calls++; throw new TypeError("Load failed"); };
  await assert.rejects(readResource("https://example.invalid/file"), ResourceNetworkError);
  assert.equal(calls, 1, "External requests do not inherit local-resource retries");
  await assert.rejects(readResource(local), ResourceNetworkError);
  assert.equal(calls, 5, "Local connection failures have a bounded retry budget");
  console.log("PASS: response-body reconnect, foreground retry, cancellation, bounded retries, and HTTP/parse error separation.");
} finally {
  globalThis.fetch = originalFetch;
  if (originalDocument) Object.defineProperty(globalThis, "document", originalDocument);
  else delete globalThis.document;
}
