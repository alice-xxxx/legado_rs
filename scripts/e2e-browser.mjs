#!/usr/bin/env node

import { createServer } from "node:http";
import { spawn } from "node:child_process";
import { createHash } from "node:crypto";
import { access, mkdtemp, mkdir, readFile, readdir, rm, writeFile } from "node:fs/promises";
import { constants } from "node:fs";
import { join, resolve } from "node:path";
import { tmpdir } from "node:os";
import { dirname } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { setTimeout as delay } from "node:timers/promises";

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const chromiumPath = process.env.CHROMIUM_EXECUTABLE_PATH || "/usr/bin/chromium";
const playwrightPath = process.env.PLAYWRIGHT_CORE_ENTRY ||
  "/workspace/.setup/browser-testing/node_modules/playwright-core/index.mjs";
const runId = new Date().toISOString().replace(/[:.]/g, "-");
const outputDir = resolve(process.env.PLAYWRIGHT_OUTPUT_DIR || join("/tmp/legado-browser-e2e-results", runId));
const origin = "http://127.0.0.1:1420";
const builtFrontendDist = process.env.BROWSER_E2E_DIST ? resolve(process.env.BROWSER_E2E_DIST) : null;
const errors = [];
const browserConsoleErrors = [];
const sourceRequests = [];
const resourceRequests = [];
const browserRequestFailures = [];
const runMessages = [];
const layoutChecks = [];
const verificationEvidence = {};
const runStartedAt = new Date().toISOString();
let runStatus = "NOT_RUN";
let runFailure;
let fixtureServer;
let fixtureOrigin;
let rustProcess;
const rustHarnessProcesses = new Set();
let viteProcess;
let browser;
let browserHarnessUrl;
let resourceServerUrl;
let dataDir;
let txtFixturePath;
let page;
let releasePendingFixtureResponse;

function report(message) {
  runMessages.push(message);
  console.log(message);
}

function assert(condition, message) {
  if (!condition) throw new Error(message);
}

async function requireFile(path, description, mode = constants.F_OK) {
  try {
    await access(path, mode);
  } catch {
    throw new Error(`${description} is unavailable at ${path}`);
  }
}

async function waitForHttp(url, timeoutMs = 45_000) {
  const deadline = Date.now() + timeoutMs;
  let lastError;
  while (Date.now() < deadline) {
    try {
      const response = await fetch(url);
      if (response.ok) return response;
      lastError = new Error(`${url} returned HTTP ${response.status}`);
    } catch (error) {
      lastError = error;
    }
    await delay(150);
  }
  throw new Error(`Timed out waiting for ${url}: ${lastError}`);
}

async function startFixtureServer() {
  let chapterTwoAttempts = 0;
  let feedRequests = 0;
  let discoveryRequests = 0;
  let reverseCatalog = false;
  let replacementBookInfoMode = "initial";
  const replacementBookInfoRequests = [];
  const catalogResponseOrders = [];
  let releaseFirstChapterTwo;
  let firstChapterTwoReleased = false;
  const firstChapterTwoResponse = new Promise((resolvePromise) => {
    releaseFirstChapterTwo = resolvePromise;
  });
  releasePendingFixtureResponse = (status = 503) => {
    if (firstChapterTwoReleased) return;
    firstChapterTwoReleased = true;
    releaseFirstChapterTwo(status);
  };
  const paragraphs = Array.from({ length: 36 }, (_, index) => {
    const number = String(index + 1).padStart(2, "0");
    return `段落 ${number}：这是来自本地书源服务器的真实正文，用于逐页核对阅读器没有跳过内容。` +
      "故事沿着河岸慢慢展开，远处的灯火映在水面，每一次选择都会带来新的方向。";
  });
  const fullText = paragraphs.join("\n\n");

  fixtureServer = createServer(async (request, response) => {
    const chunks = [];
    for await (const chunk of request) chunks.push(chunk);
    const body = Buffer.concat(chunks).toString("utf8");
    const pathname = new URL(request.url || "/", fixtureOrigin || "http://127.0.0.1").pathname;
    const chapterTwoAttempt = pathname === "/chapter/2" ? ++chapterTwoAttempts : null;
    sourceRequests.push({ method: request.method, url: request.url, body, chapterTwoAttempt });
    const html = (value) => {
      response.writeHead(200, { "content-type": "text/html; charset=utf-8" });
      response.end(value);
    };
    const failChapter = (status, attempt) => {
      response.writeHead(status, { "content-type": "text/html; charset=utf-8" });
      response.end(`<div>Fixture chapter ${attempt} intentionally unavailable</div>`);
    };
    if (pathname === "/search") {
      html("<div class='item'><h3><a href='/book'>Browser E2E Novel</a></h3><span class='author'>Fixture Author</span></div>");
    } else if (pathname === "/replacement/search") {
      html("<div class='item'><h3><a href='/replacement/book'>Browser E2E Novel</a></h3></div>");
    } else if (pathname === "/replacement/book") {
      replacementBookInfoRequests.push({ mode: replacementBookInfoMode, method: request.method });
      if (replacementBookInfoMode === "failure") {
        request.socket.destroy();
        return;
      } else if (replacementBookInfoMode === "refreshed") {
        html(`<h1>Browser E2E Novel Updated</h1><span class='author'>Refreshed Fixture Author</span>
          <p class='intro'>Metadata refreshed from the real KMP book-info operation.</p>
          <span class='kind'>Historical Mystery</span><span class='word-count'>987654</span>
          <a class='toc' href='/replacement/toc'>目录</a>`);
      } else {
        html("<h1>Browser E2E Novel</h1><a class='toc' href='/replacement/toc'>目录</a>");
      }
    } else if (pathname === "/replacement/toc") {
      html(`<ul id='list'>
        <li><a href='/replacement/chapter/opening'>Replacement Opening</a></li>
        <li><a href='/replacement/chapter/two'>Fixture Chapter Two</a></li>
        <li><a href='/replacement/chapter/ending'>Replacement Ending</a></li>
      </ul>`);
    } else if (pathname === "/replacement/chapter/opening") {
      html("<div class='content'>Replacement source opening chapter.</div>");
    } else if (pathname === "/replacement/chapter/two") {
      html(`<div class='content'>Replacement-source chapter marker. ${fullText}</div>`);
    } else if (pathname === "/replacement/chapter/ending") {
      html("<div class='content'>Replacement source ending chapter.</div>");
    } else if (pathname === "/broken/search") {
      html("<div class='item'><h3><a href='/broken/book'>Browser E2E Novel</a></h3></div>");
    } else if (pathname === "/broken/book") {
      html("<h1>Browser E2E Novel</h1><a class='toc' href='/broken/toc'>目录</a>");
    } else if (pathname === "/broken/toc") {
      html("<ul id='list'></ul>");
    } else if (pathname === "/wrong-author/search") {
      html("<div class='item'><h3><a href='/wrong-author/book'>Browser E2E Novel</a></h3><span class='author'>Unrelated Author</span></div>");
    } else if (pathname === "/wrong-title/search") {
      html("<div class='item'><h3><a href='/wrong-title/book'>Browser E2E Novel: Alternate</a></h3><span class='author'>Fixture Author</span></div>");
    } else if (pathname === "/book") {
      html("<h1>Browser E2E Novel</h1><span class='author'>Fixture Author</span><a class='toc' href='/toc'>目录</a>");
    } else if (pathname === "/toc") {
      const order = reverseCatalog ? [2, 1] : [1, 2];
      catalogResponseOrders.push(order);
      html(`<ul id='list'>${order.map((number) =>
        `<li><a href='/chapter/${number}'>Fixture Chapter ${number === 1 ? "One" : "Two"}</a></li>`,
      ).join("")}</ul>`);
    } else if (pathname === "/chapter/1") {
      html(`<div class='content'>${fullText}</div>`);
    } else if (pathname === "/chapter/2") {
      if (chapterTwoAttempt === 1) {
        const status = await firstChapterTwoResponse;
        if (status !== 200) {
          failChapter(status, chapterTwoAttempt);
          return;
        }
      } else if (chapterTwoAttempt === 2) {
        await delay(180);
        failChapter(503, chapterTwoAttempt);
        return;
      }
      html(`<div class='content'>第二章标记：${fullText}</div>`);
    } else if (pathname === "/discover") {
      discoveryRequests += 1;
      html("<ul><li class='item'><h3><a href='/discovery/book'>首页精选小说</a></h3><span class='author'>首页作者</span></li></ul>");
    } else if (pathname === "/feed.atom") {
      feedRequests += 1;
      const atom = `<?xml version="1.0" encoding="utf-8"?>
        <feed xmlns="http://www.w3.org/2005/Atom">
          <id>${fixtureOrigin}/feed.atom</id><title>Browser E2E Atom Feed</title>
          <updated>2026-10-03T00:00:00Z</updated>
          <entry><id>browser-e2e-atom-entry-one</id><title>Atom Article One</title>
            <author><name>Feed Writer</name></author><updated>2026-10-02T00:00:00Z</updated>
            <summary type="html">&lt;p&gt;Atom fixture article body one.&lt;/p&gt;&lt;script&gt;window.__UNSAFE_FEED_SCRIPT__ = true&lt;/script&gt;</summary>
          </entry>
          <entry><id>browser-e2e-atom-entry-two</id><title>Atom Article Two</title>
            <author><name>Feed Writer</name></author><updated>2026-10-01T00:00:00Z</updated>
            <summary type="html">&lt;p&gt;Atom fixture article body two.&lt;/p&gt;</summary>
          </entry>
        </feed>`;
      response.writeHead(200, { "content-type": "application/atom+xml; charset=utf-8" });
      response.end(atom);
    } else {
      response.writeHead(404, { "content-type": "text/plain; charset=utf-8" });
      response.end("not found");
    }
  });

  await new Promise((resolvePromise, reject) => {
    fixtureServer.once("error", reject);
    fixtureServer.listen(0, "127.0.0.1", resolvePromise);
  });
  const address = fixtureServer.address();
  fixtureOrigin = `http://127.0.0.1:${address.port}`;
  return {
    paragraphs,
    fullText,
    catalogResponseOrders,
    feedRequestCount() { return feedRequests; },
    discoveryRequestCount() { return discoveryRequests; },
    replacementBookInfoRequests() { return [...replacementBookInfoRequests]; },
    setReplacementBookInfoMode(mode) { replacementBookInfoMode = mode; },
    reverseCatalog() { reverseCatalog = true; },
    async waitForChapterTwoAttempt(attempt, timeoutMs = 45_000) {
      const deadline = Date.now() + timeoutMs;
      while (chapterTwoAttempts < attempt && Date.now() < deadline) await delay(20);
      assert(chapterTwoAttempts >= attempt,
        `The fixture did not receive chapter two request ${attempt}; got ${chapterTwoAttempts}.`);
      return chapterTwoAttempts;
    },
    releaseFirstChapterTwo(status = 503) { releasePendingFixtureResponse(status); },
    chapterTwoAttemptCount() { return chapterTwoAttempts; },
  };
}

async function fetchJson(url, body) {
  const response = await fetch(url, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(body),
  });
  const value = await response.json();
  if (!response.ok) throw new Error(value.error || `${url} returned HTTP ${response.status}`);
  return value && value.ok === true && Object.hasOwn(value, "value") ? value.value : value;
}

async function readResourceJson(descriptor) {
  const src = typeof descriptor === "string" ? descriptor : descriptor?.src;
  assert(src, `Rust did not return a JSON resource URL: ${JSON.stringify(descriptor)}.`);
  const response = await fetch(src);
  assert(response.ok, `Resource ${src} returned HTTP ${response.status}.`);
  return response.json();
}

function startProcess(command, args, options = {}) {
  const child = spawn(command, args, {
    cwd: options.cwd || repoRoot,
    env: options.env || process.env,
    stdio: ["ignore", "pipe", "pipe"],
    detached: process.platform !== "win32",
  });
  child.tail = [];
  const capture = (stream, label) => stream.on("data", (chunk) => {
    const text = chunk.toString();
    child.tail.push(`${label}: ${text}`);
    if (child.tail.length > 80) child.tail.shift();
    if (options.onStdout && label === "stdout") options.onStdout(text);
  });
  capture(child.stdout, "stdout");
  capture(child.stderr, "stderr");
  child.on("error", (error) => errors.push(`${command}: ${error.message}`));
  return child;
}

function parseReadyOutput(child, marker, timeoutMs = 180_000) {
  return new Promise((resolvePromise, reject) => {
    let output = "";
    const timeout = setTimeout(() => reject(new Error(`Timed out waiting for ${marker}.\n${child.tail.join("")}`)), timeoutMs);
    child.stdout.on("data", (chunk) => {
      output += chunk.toString();
      const match = output.match(new RegExp(`${marker}=([^\\s]+)`));
      if (match) {
        clearTimeout(timeout);
        resolvePromise(match[1]);
      }
    });
    child.once("exit", (code) => {
      clearTimeout(timeout);
      reject(new Error(`${marker} process exited (${code}).\n${child.tail.join("")}`));
    });
  });
}

async function startRustHarness() {
  if (!dataDir) dataDir = await mkdtemp(join(tmpdir(), "legado-browser-harness-"));
  if (!txtFixturePath) {
    txtFixturePath = join(dataDir, "browser-e2e-txt-rules.txt");
    const body = (chapter) => Array.from({ length: 48 }, (_, index) =>
      `Fixture ${chapter} paragraph ${String(index + 1).padStart(2, "0")}: Rust applies the saved heading rule while importing this real local text file.`,
    ).join("\n");
    await writeFile(txtFixturePath, [
      "Part A: Part rule opening",
      body("part-a"),
      "Chapter 01: Chapter rule opening",
      body("chapter-one"),
      "Part B: Part rule ending",
      body("part-b"),
      "Chapter 02: Chapter rule ending",
      body("chapter-two"),
    ].join("\n\n"), "utf8");
  }
  const rustEnv = {
    ...process.env,
    LEGADO_BROWSER_HARNESS_DATA: dataDir,
    LEGADO_BROWSER_HARNESS_BOOK_FILE: txtFixturePath,
    ANDROID_USER_HOME: process.env.ANDROID_USER_HOME || "/workspace/.setup/android-user",
  };
  if (process.env.BROWSER_HARNESS_BINARY) {
    const binary = resolve(process.env.BROWSER_HARNESS_BINARY);
    await requireFile(binary, "Rust browser harness binary", constants.X_OK);
    rustProcess = startProcess(binary, [], { env: rustEnv });
  } else {
    rustProcess = startProcess(
      "cargo",
      ["run", "--manifest-path", "src-tauri/Cargo.toml", "--no-default-features", "--example", "browser_harness"],
      { env: rustEnv },
    );
  }
  rustHarnessProcesses.add(rustProcess);
  const ready = await parseReadyOutput(rustProcess, "BROWSER_HARNESS_READY");
  const resourceLine = rustProcess.tail.join("").match(/BROWSER_HARNESS_RESOURCE_URL=([^\s]+)/);
  browserHarnessUrl = ready;
  if (resourceLine) resourceServerUrl = resourceLine[1];
  else {
    const health = await (await fetch(`${browserHarnessUrl}/healthz`)).json();
    resourceServerUrl = health.resourceServerUrl;
  }
  await waitForHttp(`${browserHarnessUrl}/healthz`);
}

async function stopRustHarness() {
  const child = rustProcess;
  if (!child) return;
  await stopProcessGroup(child);
  rustHarnessProcesses.delete(child);
  if (rustProcess === child) rustProcess = null;
}

async function restartRustHarness() {
  await stopRustHarness();
  await startRustHarness();
}

async function verifyApplicationStartupProcessLock() {
  const binary = process.env.BROWSER_HARNESS_BINARY ? resolve(process.env.BROWSER_HARNESS_BINARY) : null;
  const command = binary || "cargo";
  const args = binary ? [] : ["run", "--manifest-path", "src-tauri/Cargo.toml", "--no-default-features", "--example", "browser_harness"];
  const contender = startProcess(command, args, {
    env: {
      ...process.env,
      LEGADO_BROWSER_HARNESS_DATA: dataDir,
      LEGADO_BROWSER_HARNESS_BOOK_FILE: txtFixturePath,
    },
  });
  rustHarnessProcesses.add(contender);
  const exited = await waitForChildExit(contender, 45_000);
  if (!exited) {
    signalProcessGroup(contender, "SIGTERM");
    await waitForChildExit(contender, 5_000);
  }
  rustHarnessProcesses.delete(contender);
  const tail = contender.tail.join("");
  assert(exited && contender.exitCode !== 0 &&
    tail.includes("App-data root is already in use by another process"),
  `A second real ApplicationService startup did not fail on the active data root: exit=${contender.exitCode}, signal=${contender.signalCode}, output=${tail}`);
  verificationEvidence.appDataStartupLock = {
    contenderExitCode: contender.exitCode,
    rejectedSameActiveDataRoot: true,
    error: "App-data root is already in use by another process",
  };
  report("PASS: a second real browser_harness ApplicationService startup was rejected while the first service held the app-data process lock.");
}

async function startVite() {
  const args = builtFrontendDist
    ? ["run", "preview", "--", "--outDir", builtFrontendDist, "--host", "127.0.0.1", "--port", "1420", "--strictPort"]
    : ["run", "dev", "--", "--host", "127.0.0.1", "--port", "1420", "--strictPort"];
  viteProcess = startProcess("npm", args);
  await waitForHttp(`${origin}/`, 60_000);
}

async function importFixtureSource() {
  const source = {
    bookSourceName: "Local Browser E2E",
    bookSourceGroup: "Local Test",
    bookSourceUrl: fixtureOrigin,
    bookSourceType: 0,
    enabled: true,
    searchUrl: `${fixtureOrigin}/search,${JSON.stringify({ method: "POST", body: "q={{key}}" })}`,
    ruleSearch: {
      bookList: "@css:.item",
      name: "@css:h3 a@text",
      author: "@css:.author@text",
      bookUrl: "@css:h3 a@href",
    },
    ruleBookInfo: {
      name: "@css:h1@text",
      author: "@css:.author@text",
      tocUrl: "@css:a.toc@href",
    },
    exploreUrl: JSON.stringify([{
      title: "首页精选分类",
      type: "text",
      url: `${fixtureOrigin}/discover`,
    }]),
    ruleExplore: {
      bookList: "@css:.item",
      name: "@css:h3 a@text",
      author: "@css:.author@text",
      bookUrl: "@css:h3 a@href",
    },
    ruleToc: {
      chapterList: "@css:#list li",
      chapterName: "@css:a@text",
      chapterUrl: "@css:a@href",
    },
    ruleContent: { content: "@css:.content@text" },
  };
  const result = await fetchJson(`${browserHarnessUrl}/setup/source`, {
    sourceJson: JSON.stringify(source),
  });
  assert(result.sources?.length === 1, "Rust did not persist the local test source.");
  const rssSource = {
    sourceName: "Browser E2E Atom Feed",
    sourceUrl: `${fixtureOrigin}/feed.atom`,
    enabled: true,
  };
  const combined = await fetchJson(`${browserHarnessUrl}/setup/source`, {
    sourceJson: JSON.stringify(rssSource),
  });
  const bookMetadata = combined.sources?.find((entry) => entry.name === source.bookSourceName);
  const rssMetadata = combined.sources?.find((entry) => entry.name === rssSource.sourceName);
  assert(bookMetadata && rssMetadata, "Rust did not persist both the book-source and standard-feed fixture sources.");
  return { book: bookMetadata, rss: rssMetadata };
}

async function importReplacementFixtureSources() {
  const sourceDefinition = (name, searchPath) => ({
    bookSourceName: name,
    bookSourceGroup: "Source Switch E2E",
    // The app identifies imported sources by bookSourceUrl. Keep these actual fixture
    // endpoints on the same local server while giving each source its own stable ID.
    bookSourceUrl: `${fixtureOrigin}/${searchPath.split("/")[1]}`,
    bookSourceType: 0,
    enabled: true,
    searchUrl: `${fixtureOrigin}${searchPath},${JSON.stringify({ method: "POST", body: "q={{key}}" })}`,
    ruleSearch: {
      bookList: "@css:.item",
      name: "@css:h3 a@text",
      author: "@css:.author@text",
      bookUrl: "@css:h3 a@href",
    },
    ruleBookInfo: {
      name: "@css:h1@text",
      author: "@css:.author@text",
      intro: "@css:.intro@text",
      kind: "@css:.kind@text",
      wordCount: "@css:.word-count@text",
      tocUrl: "@css:a.toc@href",
    },
    ruleToc: {
      chapterList: "@css:#list li",
      chapterName: "@css:a@text",
      chapterUrl: "@css:a@href",
    },
    ruleContent: { content: "@css:.content@text" },
  });
  const definitions = [
    sourceDefinition("Replacement without author", "/replacement/search"),
    sourceDefinition("Empty replacement catalog", "/broken/search"),
    sourceDefinition("Wrong author candidate", "/wrong-author/search"),
    sourceDefinition("Wrong title candidate", "/wrong-title/search"),
  ];
  const imported = await fetchJson(`${browserHarnessUrl}/setup/source`, {
    sourceJson: JSON.stringify(definitions),
  });
  const byName = new Map((imported.sources ?? []).map((source) => [source.name, source]));
  const sources = {
    replacement: byName.get("Replacement without author"),
    broken: byName.get("Empty replacement catalog"),
    wrongAuthor: byName.get("Wrong author candidate"),
    wrongTitle: byName.get("Wrong title candidate"),
  };
  assert(Object.values(sources).every(Boolean),
    `Rust did not import all source-switch fixture sources: ${JSON.stringify(imported.sources)}.`);
  return sources;
}

async function installInvokeBridge(context) {
  await context.route(`${origin}/__legado_browser_harness__/**`, async (route) => {
    const request = route.request();
    const url = new URL(request.url());
    const suffix = url.pathname.replace(/^\/__legado_browser_harness__/, "") + url.search;
    const headers = {};
    const requestHeaders = await request.allHeaders();
    for (const name of ["content-type", "accept"]) {
      if (requestHeaders[name]) headers[name] = requestHeaders[name];
    }
    headers.origin = origin;
    try {
      const upstream = await fetch(`${browserHarnessUrl}${suffix}`, {
        method: request.method(),
        headers,
        body: ["GET", "HEAD"].includes(request.method()) ? undefined : request.postDataBuffer() ?? undefined,
      });
      const responseHeaders = Object.fromEntries(upstream.headers);
      for (const header of ["content-length", "content-encoding", "transfer-encoding", "connection"]) {
        delete responseHeaders[header];
      }
      let responseBody = Buffer.from(await upstream.arrayBuffer());
      let browserStatus = upstream.status;
      let normalizedCommandError = null;
      if (suffix.startsWith("/invoke") && upstream.status >= 400) {
        const text = responseBody.toString("utf8");
        try { normalizedCommandError = JSON.parse(text).error || text; } catch { normalizedCommandError = text; }
        responseBody = Buffer.from(JSON.stringify({ ok: false, error: normalizedCommandError }), "utf8");
        browserStatus = 200;
      }
      responseHeaders["content-length"] = String(responseBody.byteLength);
      responseHeaders["content-type"] ||= "application/json; charset=utf-8";
      await route.fulfill({ status: browserStatus, headers: responseHeaders, body: responseBody });
    } catch (error) {
      await route.fulfill({ status: 502, contentType: "application/json", body: JSON.stringify({ error: String(error) }) });
    }
  });
  await context.addInitScript(() => {
    if (window.top !== window) return;
    const callbacks = new Map();
    const listeners = new Map();
    const calls = [];
    const events = [];
    let nextCallback = 1;
    let eventCursor = 0;
    let stop = false;

    const dispatch = (record) => {
      events.push({ ...record, receivedAt: performance.now() });
      for (const [listenerId, listener] of listeners) {
        if (listener.event !== record.event) continue;
        const callback = callbacks.get(String(listener.callbackId));
        if (callback) callback({ event: record.event, id: listenerId, payload: record.payload });
      }
    };

    window.__LEGADO_BROWSER_HARNESS__ = {
      calls,
      events,
      stop() { stop = true; },
      async invoke(command, args = {}) {
        const loggedArgs = { ...args };
        for (const key of ["password", "pdfPassword"]) {
          if (Object.hasOwn(loggedArgs, key)) loggedArgs[key] = "[redacted]";
        }
        if (loggedArgs.options && typeof loggedArgs.options === "object") {
          loggedArgs.options = { ...loggedArgs.options };
          if (Object.hasOwn(loggedArgs.options, "pdfPassword")) loggedArgs.options.pdfPassword = "[redacted]";
        }
        const call = { command, args: loggedArgs, startedAt: performance.now() };
        calls.push(call);
        try {
          const response = await fetch("/__legado_browser_harness__/invoke", {
            method: "POST",
            headers: { "content-type": "application/json" },
            body: JSON.stringify({ command, args }),
          });
          const result = await response.json();
          if (!response.ok || result.ok === false) throw new Error(result.error || `Rust command failed: ${command}`);
          call.result = result.value;
          if (command === "plugin:event|listen") {
            listeners.set(result.value, { event: args.event, callbackId: args.handler });
          } else if (command === "plugin:event|unlisten") {
            listeners.delete(args.eventId);
          }
          return result.value;
        } catch (error) {
          call.error = error instanceof Error ? error.message : String(error);
          throw error;
        } finally {
          call.finishedAt = performance.now();
        }
      },
    };

    // @tauri-apps/api's isTauri() checks this exact global before forwarding to invoke.
    window.isTauri = true;
    window.__TAURI_INTERNALS__ = {
      invoke: window.__LEGADO_BROWSER_HARNESS__.invoke,
      transformCallback(callback, once = false) {
        const id = nextCallback++;
        callbacks.set(String(id), (...args) => {
          callback(...args);
          if (once) callbacks.delete(String(id));
        });
        return id;
      },
      unregisterCallback(id) { callbacks.delete(String(id)); },
    };
    window.__TAURI_EVENT_PLUGIN_INTERNALS__ = {
      unregisterListener(_event, id) { listeners.delete(id); },
    };

    async function pollEvents() {
      while (!stop) {
        if (window.location.origin === "null" || window.location.origin === "about:blank") {
          await new Promise((resolve) => setTimeout(resolve, 100));
          continue;
        }
        try {
          const response = await fetch(`/__legado_browser_harness__/events?after=${eventCursor}`);
          const result = await response.json();
          for (const event of result.events || []) {
            eventCursor = Math.max(eventCursor, event.id);
            dispatch(event);
          }
        } catch {
          await new Promise((resolve) => setTimeout(resolve, 250));
        }
        await new Promise((resolve) => setTimeout(resolve, 80));
      }
    }
    void pollEvents();
  }, browserHarnessUrl);
}

async function setViewport(width, height = 900) {
  await page.setViewportSize({ width, height });
  await page.waitForTimeout(120);
  const dimensions = await page.evaluate(() => ({
    viewport: window.innerWidth,
    document: document.documentElement.scrollWidth,
    body: document.body.scrollWidth,
  }));
  assert(dimensions.document <= width + 1 && dimensions.body <= width + 1,
    `Horizontal overflow at ${width}px: ${JSON.stringify(dimensions)}`);
}

async function commandLog() {
  return page.evaluate(() => window.__LEGADO_BROWSER_HARNESS__.calls.map((call) => ({
    command: call.command,
    args: call.args,
    result: call.result,
    error: call.error,
    startedAt: call.startedAt,
    finishedAt: call.finishedAt,
  })));
}

async function harnessEventCount() {
  return page.evaluate(() => window.__LEGADO_BROWSER_HARNESS__.events.length);
}

async function waitForHarnessEvent(eventName, payloadKind, previousCount = 0, timeoutMs = 10_000) {
  await page.waitForFunction(({ expectedEvent, expectedKind, count }) =>
    window.__LEGADO_BROWSER_HARNESS__.events.slice(count).some((event) =>
      event.event === expectedEvent && (!expectedKind || event.payload?.kind === expectedKind)),
  { expectedEvent: eventName, expectedKind: payloadKind, count: previousCount }, { timeout: timeoutMs });
  return page.evaluate(({ expectedEvent, expectedKind, count }) =>
    window.__LEGADO_BROWSER_HARNESS__.events.slice(count).find((event) =>
      event.event === expectedEvent && (!expectedKind || event.payload?.kind === expectedKind)),
  { expectedEvent: eventName, expectedKind: payloadKind, count: previousCount });
}

async function completedCommandCount(command) {
  return (await commandLog()).filter((call) => call.command === command && call.finishedAt !== undefined).length;
}

async function waitForCompletedCommand(command, previousCount = 0, timeoutMs = 15_000) {
  await page.waitForFunction(({ expectedCommand, count }) =>
    window.__LEGADO_BROWSER_HARNESS__.calls.filter((call) =>
      call.command === expectedCommand && call.finishedAt !== undefined).length > count,
  { expectedCommand: command, count: previousCount }, { timeout: timeoutMs });
  const completed = (await commandLog()).filter((call) => call.command === command && call.finishedAt !== undefined);
  const result = completed[previousCount];
  assert(result, `Rust command '${command}' completion record disappeared after its wait predicate.`);
  if (result.error) throw new Error(`Rust command '${command}' failed: ${result.error}`);
  return result;
}

async function waitForCompletedCommandMatching(command, previousCount, predicate, timeoutMs = 30_000) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const completed = (await commandLog()).filter((call) =>
      call.command === command && call.finishedAt !== undefined);
    const result = completed.slice(previousCount).find(predicate);
    if (result) {
      if (result.error) throw new Error(`Rust command '${command}' failed: ${result.error}`);
      return result;
    }
    await delay(40);
  }
  throw new Error(`Timed out waiting for Rust command '${command}' result after call ${previousCount}.`);
}

async function waitForCommandCall(command, index, timeoutMs = 15_000) {
  await page.waitForFunction(({ expectedCommand, callIndex }) =>
    window.__LEGADO_BROWSER_HARNESS__.calls.filter((call) => call.command === expectedCommand).length > callIndex,
  { expectedCommand: command, callIndex: index }, { timeout: timeoutMs });
  const call = (await commandLog()).filter((entry) => entry.command === command)[index];
  assert(call, `Rust command '${command}' call ${index} disappeared after its wait predicate.`);
  return call;
}

async function waitForFinishedCommand(command, index, timeoutMs = 15_000) {
  await page.waitForFunction(({ expectedCommand, callIndex }) => {
    const call = window.__LEGADO_BROWSER_HARNESS__.calls
      .filter((entry) => entry.command === expectedCommand)[callIndex];
    return call && call.finishedAt !== undefined;
  }, { expectedCommand: command, callIndex: index }, { timeout: timeoutMs });
  const call = (await commandLog()).filter((entry) => entry.command === command)[index];
  assert(call?.finishedAt !== undefined, `Rust command '${command}' call ${index} did not finish.`);
  return call;
}

function readPageIndicator(value) {
  const match = value.match(/(\d+)\s*\/\s*(\d+)/);
  assert(Boolean(match), `Cannot parse reader page indicator: '${value}'`);
  return { current: Number(match[1]), total: Number(match[2]) };
}

async function waitForReaderChapter(title) {
  await page.getByTestId("reader-chapter-title").filter({ hasText: title }).waitFor();
}

async function visibleTextOnCurrentPage() {
  return page.frameLocator('[data-testid="reader-frame"]').locator("body").evaluate((body) => {
    const document = body.ownerDocument;
    const width = document.documentElement.clientWidth;
    const height = document.documentElement.clientHeight;
    const walker = document.createTreeWalker(body, NodeFilter.SHOW_TEXT);
    let output = "";
    while (walker.nextNode()) {
      const node = walker.currentNode;
      for (let index = 0; index < node.textContent.length; index += 1) {
        const range = document.createRange();
        range.setStart(node, index);
        range.setEnd(node, index + 1);
        const rect = range.getBoundingClientRect();
        if (rect.width > 0 && rect.height > 0 && rect.right > 0 && rect.left < width && rect.bottom > 0 && rect.top < height) {
          output += node.textContent[index];
        }
      }
    }
    return output;
  });
}

async function visibleReaderFontSize() {
  return page.frameLocator('[data-testid="reader-frame"]').locator("body").evaluate((body) => getComputedStyle(body).fontSize);
}

async function readerFrameText() {
  return page.frameLocator('[data-testid="reader-frame"]').locator("body").innerText();
}

async function measureLayout(selector, label) {
  const locator = page.locator(selector).first();
  await locator.waitFor();
  const measurement = await locator.evaluate((element) => {
    const rect = element.getBoundingClientRect();
    return {
      fontSizePx: Number.parseFloat(getComputedStyle(element).fontSize),
      widthPx: rect.width,
      heightPx: rect.height,
    };
  });
  const result = { label, selector, ...measurement };
  layoutChecks.push(result);
  return result;
}

async function assertNoHorizontalOverflow(selector, label) {
  const measurement = await page.locator(selector).evaluate((element) => {
    const viewportWidth = element.ownerDocument.documentElement.clientWidth;
    const rect = element.getBoundingClientRect();
    const children = [...element.querySelectorAll(":scope > *")].map((child) => {
      const childRect = child.getBoundingClientRect();
      return { leftPx: childRect.left, rightPx: childRect.right, widthPx: childRect.width };
    });
    return {
      viewportWidthPx: viewportWidth,
      leftPx: rect.left,
      rightPx: rect.right,
      children,
      overflows: rect.left < -1 || rect.right > viewportWidth + 1 ||
        children.some((child) => child.leftPx < -1 || child.rightPx > viewportWidth + 1),
    };
  });
  const result = { label, selector, ...measurement };
  layoutChecks.push(result);
  assert(!result.overflows, `Horizontal overflow in ${label}: ${JSON.stringify(result)}.`);
}

async function getBookDocument(bookId) {
  const descriptor = await fetchJson(`${browserHarnessUrl}/invoke`, { command: "get_book", args: { bookId } });
  const document = await (await fetch(descriptor.src)).json();
  return { descriptor, document };
}

async function waitForTaskTerminal(taskId, timeoutMs = 45_000) {
  const deadline = Date.now() + timeoutMs;
  let lastTask;
  while (Date.now() < deadline) {
    const taskList = await fetchJson(`${browserHarnessUrl}/invoke`, { command: "tasks_resource", args: {} });
    const response = await fetch(taskList.resource.src);
    assert(response.ok, `Tasks resource failed with HTTP ${response.status}.`);
    const document = await response.json();
    lastTask = document.tasks?.find((task) => task.id === taskId);
    if (lastTask && ["completed", "failed", "cancelled"].includes(lastTask.status)) return lastTask;
    await delay(100);
  }
  throw new Error(`Catalog task ${taskId} did not reach a terminal state: ${JSON.stringify(lastTask)}.`);
}

async function assertPrivateSourceIsNotPublic() {
  const privateData = await readFile(join(dataDir, "private-data", "sources.json"), "utf8");
  assert(privateData.includes("searchUrl") && privateData.includes("ruleSearch"), "Rust did not keep the original source rules in private application storage.");
  const privateUrl = new URL("private-data/sources.json", resourceServerUrl).href;
  const response = await fetch(privateUrl);
  assert(response.status === 404, `Private source data was served as a browser resource (HTTP ${response.status}).`);
  const frontendMarkup = await page.locator("body").innerText();
  assert(!frontendMarkup.includes("ruleSearch") && !frontendMarkup.includes("searchUrl"), "The frontend rendered source definitions or rules.");
}

async function testReaderFlow(sourceMetadata, contentFixture) {
  const context = await browser.newContext({ viewport: { width: 360, height: 900 }, deviceScaleFactor: 1 });
  let expected404Recovery = null;
  await installInvokeBridge(context);
  page = await context.newPage();
  page.on("pageerror", (error) => errors.push(`pageerror: ${error.message}`));
  page.on("console", (message) => {
    if (message.type() === "error") {
      const text = message.text();
      const at = Date.now();
      browserConsoleErrors.push({
        text,
        location: message.location(),
        at,
        expectedResourceUrl: expected404Recovery && at >= expected404Recovery.startedAt
          ? expected404Recovery.url
          : undefined,
      });
    }
  });
  page.on("request", (request) => {
    if (request.url().includes("/r/")) resourceRequests.push({ url: request.url(), method: request.method() });
  });
  page.on("requestfailed", (request) => {
    browserRequestFailures.push({ url: request.url(), error: request.failure()?.errorText });
  });
  page.on("response", (response) => {
    if (response.url().includes("/r/")) resourceRequests.push({ url: response.url(), status: response.status() });
  });

  await page.goto(origin, { waitUntil: "domcontentloaded" });
  await page.getByTestId("nav-search-mobile").waitFor();
  await page.waitForFunction(() => window.__LEGADO_BROWSER_HARNESS__.calls.some((call) => call.command === "app_bootstrap"), { timeout: 10_000 });
  assert((await commandLog()).some((call) => call.command === "app_bootstrap"), "Rust-backed UI bootstrap never completed.");
  await page.getByTestId("nav-settings-mobile").click();
  await page.getByTestId("source-manager-open").click();
  await page.getByText(sourceMetadata.name, { exact: true }).waitFor();
  await page.getByTestId("nav-search-mobile").click();
  await page.getByTestId("discover-mode-search").click();
  await page.getByTestId("search-keyword").waitFor();
  await setViewport(360);
  await page.screenshot({ path: join(outputDir, "mobile-search-360.png"), fullPage: true });

  await page.getByTestId("search-keyword").fill("Browser E2E Novel");
  await page.getByTestId("search-submit").click();
  const resultCard = page.locator('[data-testid^="search-result-"]').first();
  await resultCard.waitFor({ timeout: 45_000 });
  assert((await resultCard.innerText()).includes("Browser E2E Novel"), "Search results did not display the parsed fixture book.");
  const resultTestId = await resultCard.getAttribute("data-testid");
  const resultId = resultTestId.replace("search-result-", "");
  const addBooksBefore = await completedCommandCount("add_book");
  await page.getByTestId(`search-add-${resultId}`).click();
  const addCall = await waitForCompletedCommand("add_book", addBooksBefore);
  const bookDetails = page.locator(".book-detail-panel");
  await bookDetails.waitFor();
  await setViewport(360);
  const catalogHeading = await measureLayout(".catalog-heading h3", "mobile chapter catalog heading");
  assert(catalogHeading.fontSizePx >= 14, `Mobile chapter catalog heading is too small: ${catalogHeading.fontSizePx}px.`);
  const catalogRow = await measureLayout(".catalog-row", "mobile chapter catalog row");
  assert(catalogRow.heightPx >= 44, `Mobile chapter catalog row is below 44px: ${catalogRow.heightPx}px.`);
  const catalogRowTitle = await measureLayout(".catalog-row strong", "mobile chapter catalog title");
  assert(catalogRowTitle.fontSizePx >= 14, `Mobile chapter title is too small: ${catalogRowTitle.fontSizePx}px.`);
  const detailAction = await measureLayout(".detail-actions .button", "mobile book detail action");
  assert(detailAction.heightPx >= 44, `Mobile book detail action is below 44px: ${detailAction.heightPx}px.`);
  await page.screenshot({ path: join(outputDir, "book-detail-360.png"), fullPage: true });
  assert(addCall?.result?.book?.src, "Rust add_book did not return a book resource URL.");
  const addedBook = await (await fetch(addCall.result.book.src)).json();
  const bookId = addedBook.id;
  assert(bookId && addCall.result.book.resourceId.endsWith(`/books/${bookId}/book.json`),
    "Rust book resource id did not match the added book JSON.");
  const prepareCallsBeforeOpen = await completedCommandCount("prepare_chapters");
  await bookDetails.getByRole("button", { name: /开始阅读|继续阅读/ }).click();
  await page.getByTestId("reader-frame").waitFor({ timeout: 60_000 });
  await waitForReaderChapter("Fixture Chapter One");
  const initialPrepare = await waitForCompletedCommand("prepare_chapters", prepareCallsBeforeOpen, 45_000);
  assert(initialPrepare.args.fromIndex === 0 && initialPrepare.args.count === 1 && Number(initialPrepare.result?.prepared) === 1,
    `Opening a missing first chapter should ask Rust to prepare exactly that chapter: ${JSON.stringify(initialPrepare)}.`);
  const firstPrefetchIndex = prepareCallsBeforeOpen + 1;
  await contentFixture.waitForChapterTwoAttempt(1);
  const blockedPrefetch = await waitForCommandCall("prepare_chapters", firstPrefetchIndex);
  assert(blockedPrefetch.args.fromIndex === 1 && blockedPrefetch.args.count === 1 && blockedPrefetch.finishedAt === undefined,
    `The next chapter should be a separate one-chapter background request while chapter one stays visible: ${JSON.stringify(blockedPrefetch)}.`);
  assert((await readerFrameText()).includes("段落 01"), "The current chapter did not remain readable while next-chapter fetching was blocked.");

  const { document: firstBookDoc } = await getBookDocument(bookId);
  const firstChapter = firstBookDoc.chapters[0];
  const secondChapterBeforePrefetch = firstBookDoc.chapters[1];
  assert(firstChapter?.src, "Rust did not include a resource URL for the chapter being displayed.");
  assert(secondChapterBeforePrefetch && !secondChapterBeforePrefetch.src,
    "The book JSON must not claim the blocked next chapter is cached before Rust finishes preparing it.");
  const firstCacheResponse = await fetch(firstChapter.src);
  assert(firstCacheResponse.ok, `Prepared chapter resource failed with HTTP ${firstCacheResponse.status}.`);
  const firstCacheHtml = await firstCacheResponse.text();
  const firstHash = createHash("sha256").update(firstCacheHtml).digest("hex");
  assert(firstCacheHtml.includes("font-size:"), "Chapter HTML does not include default reading styles.");

  const startFontSize = Number.parseFloat(await visibleReaderFontSize());
  const settingsWritesBefore = await completedCommandCount("save_settings");
  await page.getByRole("button", { name: "阅读显示设置" }).click();
  await page.getByTestId("reader-font-increase").click();
  await page.waitForFunction((expected) => {
    const frame = document.querySelector('[data-testid="reader-frame"]');
    return frame?.contentDocument?.body && getComputedStyle(frame.contentDocument.body).fontSize !== expected;
  }, `${startFontSize}px`);
  const increasedFontSize = Number.parseFloat(await visibleReaderFontSize());
  assert(increasedFontSize > startFontSize, `Reader font size did not increase (${startFontSize} -> ${increasedFontSize}).`);
  const settingsSaveCall = await waitForCompletedCommand("save_settings", settingsWritesBefore, 15_000);
  assert(settingsSaveCall?.result?.src, "Reader setting change did not persist a settings resource.");
  const savedSettings = await (await fetch(settingsSaveCall.result.src)).json();
  assert(savedSettings.reader.fontSizePx === increasedFontSize,
    `Saved reader font size differs from the displayed setting: ${savedSettings.reader.fontSizePx} vs ${increasedFontSize}.`);
  const afterFontHtml = await (await fetch(firstChapter.src)).text();
  assert(createHash("sha256").update(afterFontHtml).digest("hex") === firstHash, "Changing reader font rewrote cached chapter HTML.");
  await page.getByRole("button", { name: "关闭阅读设置" }).click();
  await page.getByTestId("reader-settings-popover").waitFor({ state: "hidden" });

  await setViewport(360);
  await assertNoHorizontalOverflow(".reader-topbar", "mobile reader header");
  const readerBack = await measureLayout('[data-testid="reader-back"]', "mobile reader back label");
  assert(readerBack.fontSizePx >= 14 && readerBack.heightPx >= 44,
    `Mobile reader back target/label is too small: ${JSON.stringify(readerBack)}.`);
  const readerChapterTitle = await measureLayout('[data-testid="reader-chapter-title"]', "mobile reader chapter title");
  assert(readerChapterTitle.fontSizePx >= 14, `Mobile reader chapter title is too small: ${readerChapterTitle.fontSizePx}px.`);
  const readerPageStatus = await measureLayout(".reader-page-status", "mobile reader page status");
  assert(readerPageStatus.fontSizePx >= 14, `Mobile reader page status is too small: ${readerPageStatus.fontSizePx}px.`);
  for (const selector of [
    '[data-testid="reader-prev-chapter"]',
    '[data-testid="reader-prev"]',
    '[data-testid="reader-next"]',
    '[data-testid="reader-next-chapter"]',
    '[aria-label="阅读显示设置"]',
    '[data-testid="reader-add-bookmark"]',
  ]) {
    const target = await measureLayout(selector, `mobile reader control ${selector}`);
    assert(target.widthPx >= 44 && target.heightPx >= 44,
      `Mobile reader control is below a 44px touch target: ${JSON.stringify(target)}.`);
  }
  const initialPage = readPageIndicator(await page.getByTestId("reader-page-indicator").innerText());
  assert(initialPage.total >= 3, `The long fixture chapter should occupy at least three mobile pages, got ${initialPage.total}.`);
  const visiblePages = [];
  for (let pageIndex = initialPage.current; pageIndex <= initialPage.total; pageIndex += 1) {
    const indicator = readPageIndicator(await page.getByTestId("reader-page-indicator").innerText());
    assert(indicator.current === pageIndex, `Reader skipped or repeated a page: expected ${pageIndex}, received ${indicator.current}.`);
    visiblePages.push(await visibleTextOnCurrentPage());
    if (pageIndex < initialPage.total) {
      await page.getByTestId("reader-next").click();
      await page.waitForFunction((expected) => {
        const value = document.querySelector('[data-testid="reader-page-indicator"]')?.textContent || "";
        return new RegExp(`^${expected}\\s*/`).test(value.trim());
      }, pageIndex + 1);
    }
  }
  const allVisible = visiblePages.join("").replace(/\s+/g, "");
  for (let index = 0; index < contentFixture.paragraphs.length; index += 1) {
    const marker = `段落${String(index + 1).padStart(2, "0")}`;
    assert(allVisible.includes(marker), `Paged reader skipped fixture paragraph ${marker}.`);
  }
  await page.screenshot({ path: join(outputDir, "reader-360.png"), fullPage: true });
  const firstSavedPage = readPageIndicator(await page.getByTestId("reader-page-indicator").innerText());
  const expectedFirstOffset = firstSavedPage.current - 1;
  const progressWritesBeforeSlowPrefetch = await completedCommandCount("save_progress");
  await page.getByTestId("reader-back").click();
  const firstProgressSave = await waitForCompletedCommand("save_progress", progressWritesBeforeSlowPrefetch, 10_000);
  assert(firstProgressSave?.result?.src, "Leaving the current chapter could not save progress while next-chapter prefetch was blocked.");
  await page.waitForFunction(() => !document.querySelector(".reader-shell"));
  const prefetchWhileProgressSaved = await waitForCommandCall("prepare_chapters", firstPrefetchIndex);
  assert(prefetchWhileProgressSaved.finishedAt === undefined,
    "Background next-chapter preparation finished before the current chapter progress save was observed.");
  const progressWhilePrefetchPending = await getBookDocument(bookId);
  assert(progressWhilePrefetchPending.document.progress.chapterId === firstChapter.id &&
    progressWhilePrefetchPending.document.progress.chapterIndex === 0 &&
    progressWhilePrefetchPending.document.progress.offset === expectedFirstOffset,
  `Current chapter progress was not committed while next-chapter preparation remained pending: ${JSON.stringify(progressWhilePrefetchPending.document.progress)}.`);
  verificationEvidence.progressDuringBlockedPrefetch = {
    chapterId: progressWhilePrefetchPending.document.progress.chapterId,
    chapterIndex: progressWhilePrefetchPending.document.progress.chapterIndex,
    offset: progressWhilePrefetchPending.document.progress.offset,
    prefetchStillPending: true,
  };

  contentFixture.releaseFirstChapterTwo(503);
  const firstPrefetchFailure = await waitForFinishedCommand("prepare_chapters", firstPrefetchIndex, 45_000);
  assert(firstPrefetchFailure.error && firstPrefetchFailure.args.fromIndex === 1 && firstPrefetchFailure.args.count === 1,
    `The gated next-chapter prefetch should surface its real HTTP failure: ${JSON.stringify(firstPrefetchFailure)}.`);
  const afterFirstPrefetchFailure = await getBookDocument(bookId);
  assert(!afterFirstPrefetchFailure.document.chapters[1].src,
    "Failed background preparation must not publish a stale chapter resource URL.");
  verificationEvidence.chapterPreparation = [
    { fromIndex: initialPrepare.args.fromIndex, count: initialPrepare.args.count, prepared: initialPrepare.result?.prepared },
    { fromIndex: firstPrefetchFailure.args.fromIndex, count: firstPrefetchFailure.args.count, error: firstPrefetchFailure.error },
  ];

  const continueReading = page.locator(".book-detail-panel").getByRole("button", { name: "继续阅读" });
  await continueReading.click();
  await page.getByTestId("reader-frame").waitFor({ timeout: 60_000 });
  await waitForReaderChapter("Fixture Chapter One");
  const firstChapterRestoredPage = readPageIndicator(await page.getByTestId("reader-page-indicator").innerText());
  assert(firstChapterRestoredPage.current === expectedFirstOffset + 1,
    `Current chapter page did not restore after the prefetch error: expected ${expectedFirstOffset + 1}, got ${firstChapterRestoredPage.current}.`);
  const secondPrefetchIndex = firstPrefetchIndex + 1;
  await contentFixture.waitForChapterTwoAttempt(2);
  const secondPrefetch = await waitForCommandCall("prepare_chapters", secondPrefetchIndex);
  assert(secondPrefetch.args.fromIndex === 1 && secondPrefetch.args.count === 1,
    `Reopening the current chapter should retry the one-chapter background prefetch: ${JSON.stringify(secondPrefetch)}.`);
  const secondPrefetchFailure = await waitForFinishedCommand("prepare_chapters", secondPrefetchIndex, 45_000);
  assert(secondPrefetchFailure.error,
    `The second controlled background request should fail before the explicit chapter turn retry: ${JSON.stringify(secondPrefetchFailure)}.`);
  await page.waitForFunction(() => !document.querySelector(".preload-indicator"), null, { timeout: 10_000 });
  assert((await readerFrameText()).includes("段落 01"), "A failed next-chapter retry replaced the readable current chapter.");
  assert(!((await getBookDocument(bookId)).document.chapters[1].src),
    "A failed background retry published a chapter URL that is not cached.");

  const prepareCallsBeforeChapterRetry = await completedCommandCount("prepare_chapters");
  await page.getByTestId("reader-next-chapter").click();
  const chapterTurnPrepare = await waitForCompletedCommand("prepare_chapters", prepareCallsBeforeChapterRetry, 45_000);
  assert(chapterTurnPrepare.args.fromIndex === 1 && chapterTurnPrepare.args.count === 1 &&
    Number(chapterTurnPrepare.result?.prepared) === 1,
  `Choosing the next chapter should retry and cache exactly that chapter after background failure: ${JSON.stringify(chapterTurnPrepare)}.`);
  await contentFixture.waitForChapterTwoAttempt(3);
  await waitForReaderChapter("Fixture Chapter Two");
  const recoveredBook = await getBookDocument(bookId);
  const recoveredSecondChapter = recoveredBook.document.chapters[1];
  assert(recoveredSecondChapter?.src, "The explicit next-chapter retry did not publish a browser resource URL.");
  const recoveredChapterResponse = await fetch(recoveredSecondChapter.src);
  assert(recoveredChapterResponse.ok, `The retried next chapter resource failed with HTTP ${recoveredChapterResponse.status}.`);
  verificationEvidence.chapterPreparation.push(
    { fromIndex: secondPrefetchFailure.args.fromIndex, count: secondPrefetchFailure.args.count, error: secondPrefetchFailure.error },
    { fromIndex: chapterTurnPrepare.args.fromIndex, count: chapterTurnPrepare.args.count, prepared: chapterTurnPrepare.result?.prepared, recoveredAfterBackgroundErrors: true },
  );

  const callsBeforeCachedNavigation = await commandLog();
  const chapterFetchCommands = (calls) => calls.filter((call) =>
    call.command === "get_book" || /chapter|content|prepare/i.test(call.command));
  const chapterFetchCountBeforeCachedNavigation = chapterFetchCommands(callsBeforeCachedNavigation).length;
  await page.getByTestId("reader-prev-chapter").click();
  await waitForReaderChapter("Fixture Chapter One");
  await page.getByTestId("reader-next-chapter").click();
  await waitForReaderChapter("Fixture Chapter Two");
  await page.waitForTimeout(200);
  const callsAfterCachedNavigation = await commandLog();
  const chapterFetchCallsAfterCachedNavigation = chapterFetchCommands(callsAfterCachedNavigation);
  assert(chapterFetchCallsAfterCachedNavigation.length === chapterFetchCountBeforeCachedNavigation,
    `Switching between chapters whose resources are already cached invoked Rust retrieval: ${chapterFetchCallsAfterCachedNavigation.slice(chapterFetchCountBeforeCachedNavigation).map((call) => call.command).join(", ")}`);

  await setViewport(1440);
  await page.screenshot({ path: join(outputDir, "reader-1440.png"), fullPage: true });
  const desktopIndicator = readPageIndicator(await page.getByTestId("reader-page-indicator").innerText());
  assert(desktopIndicator.current <= desktopIndicator.total, "Reader resize left its page position outside the new page range.");
  assert((await readerFrameText()).includes("第二章标记"), "Resizing the reader lost its current cached chapter content.");
  assert((await visibleTextOnCurrentPage()).includes("第二章标记"), "Resizing the reader moved the visible page away from its current chapter content.");
  const { document: secondBookDoc } = await getBookDocument(bookId);
  const secondChapter = secondBookDoc.chapters[1];
  const secondChapterFile = join(dataDir, "books", bookId, "chapters", `${secondChapter.id}.html`);
  const secondCache = await readFile(secondChapterFile);
  const secondHash = createHash("sha256").update(secondCache).digest("hex");
  const beforeSecondPage = readPageIndicator(await page.getByTestId("reader-page-indicator").innerText());
  if (beforeSecondPage.total > 1) {
    await page.getByTestId("reader-next").click();
    await page.waitForFunction((expected) => {
      const value = document.querySelector('[data-testid="reader-page-indicator"]')?.textContent || "";
      return new RegExp(`^${expected}\\s*/`).test(value.trim());
    }, beforeSecondPage.current + 1);
  }
  const secondPageIndicator = readPageIndicator(await page.getByTestId("reader-page-indicator").innerText());
  if (secondPageIndicator.current < secondPageIndicator.total) {
    await page.getByTestId("reader-next").click();
    await page.waitForFunction((expected) => {
      const value = document.querySelector('[data-testid="reader-page-indicator"]')?.textContent || "";
      return new RegExp(`^${expected}\\s*/`).test(value.trim());
    }, secondPageIndicator.current + 1);
  }
  const savedPosition = readPageIndicator(await page.getByTestId("reader-page-indicator").innerText());
  const expectedOffset = savedPosition.current - 1;
  const progressWritesBeforeLeave = await completedCommandCount("save_progress");
  await page.getByTestId("reader-back").click();
  await waitForCompletedCommand("save_progress", progressWritesBeforeLeave, 15_000);
  await page.waitForFunction(() => !document.querySelector(".reader-shell"));
  const shelfCard = page.locator('[data-testid^="shelf-book-"]').first();
  await shelfCard.waitFor();
  const shelfTestId = await shelfCard.getAttribute("data-testid");
  assert(shelfTestId === `shelf-book-${bookId}`, `Shelf displayed an unexpected book after add: ${shelfTestId}.`);
  const persisted = await getBookDocument(bookId);
  assert(persisted.document.progress.chapterIndex === 1, `Saved progress has the wrong chapter: ${JSON.stringify(persisted.document.progress)}.`);
  assert(persisted.document.progress.offset === expectedOffset, `Saved progress has the wrong page offset: ${JSON.stringify(persisted.document.progress)}.`);

  expected404Recovery = { url: secondChapter.src, startedAt: Date.now() };
  await rm(secondChapterFile, { force: true });
  const prepareCountBefore404 = await completedCommandCount("prepare_chapters");
  await page.getByRole("button", { name: "关闭详情" }).click();
  await page.locator(`[data-testid="shelf-book-${bookId}"] button.cover-button`).click();
  await page.locator(".book-detail-panel").getByRole("button", { name: "继续阅读" }).click();
  await page.getByTestId("reader-frame").waitFor({ timeout: 60_000 });
  await waitForReaderChapter("Fixture Chapter Two");
  const recoveryPrepare = await waitForCompletedCommand("prepare_chapters", prepareCountBefore404, 45_000);
  assert(recoveryPrepare.args.fromIndex === 1 && Number(recoveryPrepare.result?.prepared) >= 1,
    `404 recovery did not re-prepare the missing chapter from index 1: ${JSON.stringify(recoveryPrepare)}.`);
  verificationEvidence.chapterPreparation.push({
    fromIndex: recoveryPrepare.args.fromIndex,
    count: recoveryPrepare.args.count,
    prepared: recoveryPrepare.result?.prepared,
    recoveredMissingSrc: true,
  });
  const restored = readPageIndicator(await page.getByTestId("reader-page-indicator").innerText());
  assert(restored.current === expectedOffset + 1,
    `Saved reader page was not restored after reopening and repairing the missing chapter: expected ${expectedOffset + 1}, got ${restored.current}.`);
  await access(secondChapterFile, constants.F_OK);
  const repairedHtml = await readFile(secondChapterFile, "utf8");
  assert(repairedHtml.includes("font-size:"), "404 recovery did not recreate a browser-ready HTML chapter resource.");
  assert(createHash("sha256").update(await readFile(join(dataDir, "books", bookId, "chapters", `${firstChapter.id}.html`))).digest("hex") === firstHash,
    "Repairing a second chapter modified the first chapter cache.");

  const progressWritesBeforeRefresh = await completedCommandCount("save_progress");
  await page.getByTestId("reader-back").click();
  await waitForCompletedCommand("save_progress", progressWritesBeforeRefresh, 15_000);
  await page.locator(".book-detail-panel").waitFor();
  const beforeCatalogRefresh = await getBookDocument(bookId);
  const chapterTwoBeforeRefresh = beforeCatalogRefresh.document.chapters.find((chapter) => chapter.title === "Fixture Chapter Two");
  const chapterOneBeforeRefresh = beforeCatalogRefresh.document.chapters.find((chapter) => chapter.title === "Fixture Chapter One");
  assert(chapterTwoBeforeRefresh?.id === secondChapter.id && chapterOneBeforeRefresh,
    "The active chapter IDs were not available before refreshing the catalog.");
  assert(beforeCatalogRefresh.document.progress.chapterId === chapterTwoBeforeRefresh.id &&
    beforeCatalogRefresh.document.progress.chapterIndex === 1 &&
    beforeCatalogRefresh.document.progress.offset === expectedOffset,
  `The pre-refresh progress is not anchored to the chapter being read: ${JSON.stringify(beforeCatalogRefresh.document.progress)}.`);

  contentFixture.reverseCatalog();
  const refreshCallsBefore = await completedCommandCount("refresh_chapters");
  await page.getByTestId("book-refresh-catalog").click();
  const refreshInvocation = await waitForCompletedCommand("refresh_chapters", refreshCallsBefore, 15_000);
  const refreshTaskId = refreshInvocation.result?.taskId;
  assert(typeof refreshTaskId === "string", `Refreshing the catalog did not return its Rust task ID: ${JSON.stringify(refreshInvocation)}.`);
  const refreshTask = await waitForTaskTerminal(refreshTaskId);
  assert(refreshTask.status === "completed" && refreshTask.result?.committed === true,
    `The fixture catalog refresh task did not commit successfully: ${JSON.stringify(refreshTask)}.`);
  assert(contentFixture.catalogResponseOrders.at(-1)?.join(",") === "2,1",
    `The real KMP catalog refresh did not receive the reordered fixture chapters: ${JSON.stringify(contentFixture.catalogResponseOrders)}.`);
  await page.waitForFunction(() => [...document.querySelectorAll(".catalog-row strong")]
    .map((element) => element.textContent?.trim()).join("|") === "Fixture Chapter Two|Fixture Chapter One", null, { timeout: 15_000 });

  const afterCatalogRefresh = await getBookDocument(bookId);
  const chapterTwoAfterRefresh = afterCatalogRefresh.document.chapters[0];
  const chapterOneAfterRefresh = afterCatalogRefresh.document.chapters[1];
  assert(chapterTwoAfterRefresh.title === "Fixture Chapter Two" && chapterOneAfterRefresh.title === "Fixture Chapter One",
    `The refreshed book resource did not follow the fixture chapter order: ${JSON.stringify(afterCatalogRefresh.document.chapters)}.`);
  assert(chapterTwoAfterRefresh.id === chapterTwoBeforeRefresh.id && chapterOneAfterRefresh.id === chapterOneBeforeRefresh.id,
    "Catalog reordering changed stable chapter IDs instead of matching chapters by their source identity.");
  assert(afterCatalogRefresh.document.progress.chapterId === chapterTwoBeforeRefresh.id &&
    afterCatalogRefresh.document.progress.chapterIndex === 0 &&
    afterCatalogRefresh.document.progress.offset === expectedOffset,
  `Catalog refresh moved progress to the wrong chapter or page: ${JSON.stringify(afterCatalogRefresh.document.progress)}.`);
  assert(await page.locator(".catalog-row.current strong").innerText() === "Fixture Chapter Two",
    "The refreshed catalog highlight does not follow the chapter identified by saved progress.");
  verificationEvidence.catalogRefresh = {
    status: refreshTask.status,
    committed: refreshTask.result?.committed,
    sourceResponseOrder: contentFixture.catalogResponseOrders.at(-1),
    chapterIdsStable: true,
    beforeProgress: beforeCatalogRefresh.document.progress,
    afterProgress: afterCatalogRefresh.document.progress,
  };

  const preparesBeforeReopenAfterRefresh = await completedCommandCount("prepare_chapters");
  await page.locator(".book-detail-panel").getByRole("button", { name: "继续阅读" }).click();
  await page.getByTestId("reader-frame").waitFor({ timeout: 60_000 });
  await waitForReaderChapter("Fixture Chapter Two");
  const pageAfterCatalogRefresh = readPageIndicator(await page.getByTestId("reader-page-indicator").innerText());
  assert(pageAfterCatalogRefresh.current === expectedOffset + 1,
    `Reopening after reordering did not restore the same chapter page: expected ${expectedOffset + 1}, got ${pageAfterCatalogRefresh.current}.`);
  assert((await readerFrameText()).includes("第二章标记"), "Reordering the catalog opened chapter one content at chapter two progress.");
  assert(await completedCommandCount("prepare_chapters") === preparesBeforeReopenAfterRefresh,
    "Opening an already cached chapter after catalog reordering unnecessarily called Rust to fetch chapter content.");
  const resumedAfterRefresh = await getBookDocument(bookId);
  assert(resumedAfterRefresh.document.progress.chapterId === chapterTwoBeforeRefresh.id &&
    resumedAfterRefresh.document.progress.chapterIndex === 0 &&
    resumedAfterRefresh.document.progress.offset === expectedOffset,
  `Progress changed after reopening the reordered book: ${JSON.stringify(resumedAfterRefresh.document.progress)}.`);

  await assertPrivateSourceIsNotPublic();
  const allCalls = await commandLog();
  verificationEvidence.rustCommands = allCalls.map((call) => call.command);
  verificationEvidence.sourceHttpPaths = [...new Set(sourceRequests.map((request) => new URL(request.url, fixtureOrigin).pathname))].sort();
  verificationEvidence.resourceHttp = resourceRequests
    .filter((request) => request.status !== undefined)
    .map((request) => ({ path: new URL(request.url).pathname.split("/").slice(-2).join("/"), status: request.status }));
  const requiredCommands = ["app_bootstrap", "list_sources", "start_search", "add_book", "get_book", "prepare_chapters", "save_settings", "save_progress", "refresh_chapters", "tasks_resource"];
  for (const command of requiredCommands) assert(allCalls.some((call) => call.command === command), `Browser flow did not exercise real Rust command '${command}'.`);
  assert(sourceRequests.some((request) => request.url.startsWith("/search") && request.method === "POST"), "The local book-source engine did not execute a real fixture HTTP search.");
  assert(sourceRequests.some((request) => request.url.startsWith("/book")) && sourceRequests.some((request) => request.url.startsWith("/toc")), "The source engine did not fetch book details and chapter listing from the fixture server.");
  assert(sourceRequests.some((request) => request.url.startsWith("/chapter/1")) && sourceRequests.some((request) => request.url.startsWith("/chapter/2")), "The source engine did not fetch both chapter bodies from the fixture server.");
  assert(contentFixture.chapterTwoAttemptCount() >= 4,
    `The source engine did not exercise the blocked/failing/retried chapter path: received ${contentFixture.chapterTwoAttemptCount()} chapter-two requests.`);
  assert(resourceRequests.some((request) => request.url.includes("/r/") && request.url.endsWith("/shelf.json") && request.status === 200), "Browser did not fetch the shelf JSON resource.");
  assert(resourceRequests.some((request) => request.url.includes("/chapters/") && request.status === 200), "Browser did not fetch a processed chapter HTML resource.");
  const failedChapterRequestIndex = resourceRequests.findIndex((request) =>
    request.url === expected404Recovery.url && request.status === 404);
  assert(failedChapterRequestIndex >= 0, `Browser did not receive the expected 404 for ${expected404Recovery.url}.`);
  const repairedChapterResponse = resourceRequests.slice(failedChapterRequestIndex + 1).find((request) =>
    request.url === expected404Recovery.url && request.status === 200);
  assert(repairedChapterResponse, `The exact missing chapter URL was not served successfully after recovery: ${expected404Recovery.url}.`);

  const expected404Message = "Failed to load resource: the server responded with a status of 404 (Not Found)";
  const expected404ConsoleErrors = browserConsoleErrors.filter((entry) =>
    entry.text === expected404Message && entry.expectedResourceUrl === expected404Recovery.url);
  assert(expected404ConsoleErrors.length <= 1,
    `The deliberate chapter 404 produced repeated console errors for ${expected404Recovery.url}.`);
  const unexpectedConsoleErrors = browserConsoleErrors.filter((entry) => !expected404ConsoleErrors.includes(entry));
  const unexpectedRequestFailures = browserRequestFailures.filter((failure) =>
    failure.url !== expected404Recovery.url || failure.error !== "net::ERR_ABORTED");
  const unexpectedErrors = [
    ...errors,
    ...unexpectedConsoleErrors.map((entry) => `console: ${entry.text}`),
    ...unexpectedRequestFailures.map((failure) => `request failed: ${failure.url} (${failure.error})`),
  ];
  assert(unexpectedErrors.length === 0, `Browser reported unexpected errors:\n${unexpectedErrors.join("\n")}`);

  report(`PASS: real Rust service + KMP source search/read on Chromium (${allCalls.length} IPC calls).`);
  report(`PASS: ${contentFixture.paragraphs.length} mobile-page paragraphs were all visible; font ${startFontSize}px -> ${increasedFontSize}px.`);
  report("PASS: current chapter remained readable and its progress saved while the separate one-chapter background prefetch was blocked; the next chapter recovered after background HTTP failures.");
  report("PASS: cached chapter flips invoked no Rust commands; missing-resource recovery and chapter/page progress restoration worked.");
  report("PASS: real KMP catalog refresh reordered chapters without changing chapter identity or moving saved progress to the wrong chapter/page.");
  report("PASS: cached HTML hash stayed stable after display-only font adjustment; missing chapter URL recovered through Rust.");
  report(`PASS: exact chapter URL returned 404 then 200 after repair${expected404ConsoleErrors.length ? " (one expected browser 404 notice)" : ""}.`);
  report("PASS: private source rules stayed off the browser resource server; real JSON and HTML resources were fetched.");
  report(`Screenshots: ${outputDir}`);
  await context.close();
  return { bookId };
}

async function testHomeRssFlow(sources, contentFixture) {
  const context = await browser.newContext({ viewport: { width: 360, height: 900 }, deviceScaleFactor: 1 });
  await installInvokeBridge(context);
  page = await context.newPage();
  const errorStart = errors.length;
  const consoleErrorStart = browserConsoleErrors.length;
  const requestFailureStart = browserRequestFailures.length;
  page.on("pageerror", (error) => errors.push(`pageerror: ${error.message}`));
  page.on("console", (message) => {
    if (message.type() === "error") browserConsoleErrors.push({ text: message.text(), location: message.location() });
  });
  page.on("request", (request) => {
    if (request.url().includes("/r/")) resourceRequests.push({ url: request.url(), method: request.method() });
  });
  page.on("requestfailed", (request) => browserRequestFailures.push({ url: request.url(), error: request.failure()?.errorText }));
  page.on("response", (response) => {
    if (response.url().includes("/r/")) resourceRequests.push({ url: response.url(), status: response.status() });
  });

  const readJson = async (descriptor) => {
    const src = typeof descriptor === "string" ? descriptor : descriptor?.src;
    assert(src, `A Rust command did not return a resource URL: ${JSON.stringify(descriptor)}.`);
    const response = await fetch(src);
    assert(response.ok, `Resource ${src} returned HTTP ${response.status}.`);
    return response.json();
  };

  await page.goto(origin, { waitUntil: "domcontentloaded" });
  await page.getByTestId("home-section-empty").waitFor({ timeout: 15_000 });
  const initialHomeCall = await waitForCompletedCommand("get_home_config", 0);
  const initialHome = await readJson(initialHomeCall.result);
  assert(initialHome.tabs?.length === 1 && initialHome.tabs[0].sections.length === 0,
    `A fresh app should load its empty home configuration from Rust: ${JSON.stringify(initialHome)}.`);

  let discoveryCallsBeforeLoad = await completedCommandCount("list_discovery_categories");
  await page.getByTestId("nav-search-mobile").click();
  await page.getByTestId("discover-mode-discover").click();
  await page.getByTestId("discovery-source").waitFor();
  let selectedDiscoverySource = await page.getByTestId("discovery-source").inputValue();
  if (selectedDiscoverySource !== sources.book.id) {
    discoveryCallsBeforeLoad = await completedCommandCount("list_discovery_categories");
    await page.getByTestId("discovery-source").selectOption(sources.book.id);
  }
  const categoriesCall = await waitForCompletedCommand("list_discovery_categories", discoveryCallsBeforeLoad);
  const categories = await readJson(categoriesCall.result.resource);
  assert(categories.categories?.length === 1, `KMP did not produce the fixture discovery category: ${JSON.stringify(categories)}.`);
  const category = categories.categories[0];
  assert(category.categoryId && category.title === "首页精选分类",
    `Discovery returned an unexpected processed category: ${JSON.stringify(category)}.`);
  assert(!JSON.stringify(categories).includes(fixtureOrigin) && !JSON.stringify(categories).includes("ruleExplore"),
    "The public discovery category JSON exposed a source URL or source rules.");

  const favoriteEventCount = await harnessEventCount();
  const favoriteCallsBefore = await completedCommandCount("set_discovery_favorite");
  await page.getByTestId(`discovery-category-favorite-${category.categoryId}`).click();
  const favoriteCall = await waitForCompletedCommand("set_discovery_favorite", favoriteCallsBefore);
  const favorites = await readJson(favoriteCall.result);
  assert(favorites.favorites?.length === 1 && favorites.favorites[0].categoryId === category.categoryId,
    `Rust did not persist the selected discovery favorite: ${JSON.stringify(favorites)}.`);
  await waitForHarnessEvent("resource-updated", "discoveryFavorites", favoriteEventCount);

  const homeSaveCallsBefore = await completedCommandCount("save_home_config");
  const homeSaveEventCount = await harnessEventCount();
  await page.getByTestId(`discovery-category-home-${category.categoryId}`).click();
  const homeSaveCall = await waitForCompletedCommand("save_home_config", homeSaveCallsBefore);
  const savedHome = await readJson(homeSaveCall.result);
  const section = savedHome.tabs?.flatMap((tab) => tab.sections ?? [])[0];
  assert(section?.sourceId === sources.book.id && section.categoryId === category.categoryId && section.title === category.title,
    `Rust did not save the discovery category to the home resource: ${JSON.stringify(savedHome)}.`);
  assert(!JSON.stringify(savedHome).includes(fixtureOrigin) && !JSON.stringify(savedHome).includes("ruleExplore"),
    "The saved public home configuration contains a source URL or source rules.");
  await waitForHarnessEvent("resource-updated", "homeConfig", homeSaveEventCount);
  await page.screenshot({ path: join(outputDir, "home-category-configured.png"), fullPage: true });

  await page.getByTestId("nav-home-mobile").click();
  await page.getByTestId(`home-section-${section.id}`).waitFor();
  const discoveryBookCallsBefore = await completedCommandCount("list_discovery_books");
  await page.getByTestId(`home-section-open-${section.id}`).click();
  const discoveryBooksCall = await waitForCompletedCommand("list_discovery_books", discoveryBookCallsBefore);
  const homeBookResults = await readJson(discoveryBooksCall.result.resource);
  assert(homeBookResults.results?.length === 1 && homeBookResults.results[0].title === "首页精选小说",
    `Home did not consume the Rust-processed discovery card: ${JSON.stringify(homeBookResults)}.`);
  const processedCard = homeBookResults.results[0];
  assert(processedCard.resultId && processedCard.sourceId === sources.book.id,
    `The processed discovery card is missing its opaque ID or display fields: ${JSON.stringify(processedCard)}.`);
  assert(!JSON.stringify(homeBookResults).includes("ruleExplore") && !JSON.stringify(homeBookResults).includes("ruleContent"),
    "The public discovery result JSON exposed source rules.");
  await page.getByTestId(`home-section-result-${processedCard.resultId}`).click();
  const processedCardPanel = page.locator(".result-detail-panel");
  await processedCardPanel.waitFor();
  assert((await processedCardPanel.innerText()).includes("首页精选小说"),
    "Opening the home card did not display its processed book metadata.");
  await page.screenshot({ path: join(outputDir, "home-processed-book-card.png"), fullPage: true });
  await processedCardPanel.locator(".detail-close").click();

  await page.getByTestId("nav-search-mobile").click();
  const rssCategoryCallsBeforeMode = await completedCommandCount("list_rss_categories");
  await page.getByTestId("discover-mode-rss").click();
  await page.getByTestId("discovery-source").waitFor();
  assert(await page.locator('[data-testid="discovery-source"]').locator(`option[value="${sources.rss.id}"]`).count() === 1,
    "The standard Atom fixture was not projected as an RSS source in the native source list.");
  let rssCategoryCallsBeforeLoad = rssCategoryCallsBeforeMode;
  const selectedRssSource = await page.getByTestId("discovery-source").inputValue();
  if (selectedRssSource !== sources.rss.id) {
    rssCategoryCallsBeforeLoad = await completedCommandCount("list_rss_categories");
    await page.getByTestId("discovery-source").selectOption(sources.rss.id);
  }
  const rssCategoriesCall = await waitForCompletedCommand("list_rss_categories", rssCategoryCallsBeforeLoad);
  const rssCategories = await readJson(rssCategoriesCall.result.resource);
  assert(rssCategories.categories?.length === 1 && rssCategories.categories[0].title === "最新文章",
    `The Rust feed parser did not publish its processed Atom category: ${JSON.stringify(rssCategories)}.`);
  const rssStateAfterCategory = await readJson(rssCategoriesCall.result.rssState);
  assert(rssStateAfterCategory.subscriptions.some((entry) => entry.sourceId === sources.rss.id && entry.filter === "all"),
    `Opening an RSS category did not create and persist the default all filter: ${JSON.stringify(rssStateAfterCategory)}.`);
  const rssArticleCallsBefore = await completedCommandCount("list_rss_articles");
  const rssArticlesCall = await waitForCompletedCommand("list_rss_articles", rssArticleCallsBefore, 30_000);
  const rssArticles = await readJson(rssArticlesCall.result.resource);
  assert(rssArticles.results?.length === 2, `The standard Atom feed should produce two article cards: ${JSON.stringify(rssArticles)}.`);
  const firstArticle = rssArticles.results.find((article) => article.title === "Atom Article One");
  const secondArticle = rssArticles.results.find((article) => article.title === "Atom Article Two");
  assert(firstArticle?.articleId && firstArticle.resultId === firstArticle.articleId && firstArticle.contentSrc,
    `The RSS article card is missing its Rust-issued article ID or processed content reference: ${JSON.stringify(firstArticle)}.`);
  assert(secondArticle?.articleId && secondArticle.contentSrc,
    `The second Atom article card was not processed: ${JSON.stringify(secondArticle)}.`);
  const rssPublicText = JSON.stringify(rssArticles);
  assert(!rssPublicText.includes(fixtureOrigin) && !rssPublicText.includes("Atom fixture article body") && !rssPublicText.includes("<script"),
    "RSS card JSON exposed the original feed URL or article HTML body.");
  assert(firstArticle.contentSrc.startsWith(resourceServerUrl),
    `The RSS content reference was not materialized through the Rust resource server: ${firstArticle.contentSrc}.`);
  const firstArticleResource = await fetch(firstArticle.contentSrc);
  assert(firstArticleResource.ok, `The Rust-served RSS chapter HTML failed with HTTP ${firstArticleResource.status}.`);
  const firstArticleHtml = await firstArticleResource.text();
  assert(firstArticleHtml.includes("Atom fixture article body one") && !firstArticleHtml.includes("<script") &&
    !firstArticleHtml.includes("__UNSAFE_FEED_SCRIPT__"),
  "The browser resource for the Atom article did not contain sanitized processed HTML.");
  await page.getByTestId(`rss-open-${firstArticle.resultId}`).waitFor();

  const rssStateCallsBeforeOpen = await completedCommandCount("get_rss_state");
  const openArticleCallsBefore = await completedCommandCount("open_rss_article");
  await page.getByTestId(`rss-open-${firstArticle.resultId}`).click();
  const openArticleCall = await waitForCompletedCommand("open_rss_article", openArticleCallsBefore);
  const openedArticleHtml = await (await fetch(openArticleCall.result.resource.src)).text();
  assert(openedArticleHtml.includes("Atom fixture article body one") && !openedArticleHtml.includes("<script"),
    "open_rss_article did not return the sanitized HTML resource.");
  const articleFrame = page.getByTestId("rss-article-frame");
  await articleFrame.waitFor();
  const displayedArticleText = await page.frameLocator('[data-testid="rss-article-frame"]').locator("body").innerText();
  assert(displayedArticleText.includes("Atom fixture article body one"),
    "The WebView did not display the processed Atom article HTML.");
  await page.getByRole("button", { name: "关闭文章" }).click();
  await articleFrame.waitFor({ state: "detached" });
  const stateAfterReadCall = await waitForCompletedCommand("get_rss_state", rssStateCallsBeforeOpen);
  const stateAfterRead = await readJson(stateAfterReadCall.result);
  assert(stateAfterRead.articles.some((entry) => entry.sourceId === sources.rss.id &&
    entry.articleId === firstArticle.articleId && entry.isRead && !entry.isFavorite),
  `Opening the article did not persist its read state: ${JSON.stringify(stateAfterRead)}.`);

  const articleStateCallsBefore = await completedCommandCount("set_rss_article_state");
  const favoriteArticleEventCount = await harnessEventCount();
  await page.getByTestId(`rss-favorite-${firstArticle.resultId}`).click();
  const articleStateCall = await waitForCompletedCommand("set_rss_article_state", articleStateCallsBefore);
  assert(articleStateCall.args.sourceId === sources.rss.id && articleStateCall.args.articleId === firstArticle.articleId &&
    articleStateCall.args.isFavorite === true,
  `Browser command arguments did not match the Tauri article-state contract: ${JSON.stringify(articleStateCall)}.`);
  await waitForHarnessEvent("resource-updated", "rssState", favoriteArticleEventCount);

  const selectRssFilter = async (filter, expectedTitle) => {
    const filterCallsBefore = await completedCommandCount("set_rss_filter");
    const articleCallsBefore = await completedCommandCount("list_rss_articles");
    await page.getByTestId("rss-filter").selectOption(filter);
    const filterCall = await waitForCompletedCommand("set_rss_filter", filterCallsBefore);
    assert(filterCall.args.sourceId === sources.rss.id && filterCall.args.filter === filter,
      `Browser command arguments did not match the Tauri RSS filter contract: ${JSON.stringify(filterCall)}.`);
    const articleCall = await waitForCompletedCommandMatching(
      "list_rss_articles",
      articleCallsBefore,
      (call) => call.args.sourceId === sources.rss.id && call.result?.filter === filter,
    );
    assert(articleCall.result.bookCount === 1,
      `Rust returned an unexpected count for the '${filter}' filter: ${JSON.stringify(articleCall.result)}.`);
    const document = await readJson(articleCall.result.resource);
    assert(document.results?.length === 1 && document.results[0].title === expectedTitle,
      `The '${filter}' filter should show '${expectedTitle}' only: ${JSON.stringify(document.results)}.`);
    await page.waitForFunction((title) =>
      [...document.querySelectorAll(".search-results-grid .result-title")]
        .map((element) => element.textContent?.trim()).join("|") === title,
    expectedTitle, { timeout: 10_000 });
    return { filterCall, articleCall, document };
  };

  const readFiltered = await selectRssFilter("read", "Atom Article One");
  assert(readFiltered.document.results[0].isRead && readFiltered.document.results[0].isFavorite,
    "The read filter did not expose the article's persisted favorite/read flags.");
  const favoritesFiltered = await selectRssFilter("favorites", "Atom Article One");
  assert(favoritesFiltered.document.results[0].isFavorite,
    "The favorites filter did not select the favorited Atom article.");
  const unreadFiltered = await selectRssFilter("unread", "Atom Article Two");
  assert(!unreadFiltered.document.results[0].isRead,
    "The unread filter returned an already read Atom article.");
  await selectRssFilter("favorites", "Atom Article One");
  await page.screenshot({ path: join(outputDir, "rss-favorites-filter.png"), fullPage: true });

  const stateBeforeUnsubscribeCall = await page.evaluate(async () =>
    window.__LEGADO_BROWSER_HARNESS__.invoke("get_rss_state"));
  const stateBeforeUnsubscribe = await readJson(stateBeforeUnsubscribeCall);
  assert(stateBeforeUnsubscribe.subscriptions.some((entry) => entry.sourceId === sources.rss.id && entry.filter === "favorites") &&
    stateBeforeUnsubscribe.articles.some((entry) => entry.sourceId === sources.rss.id && entry.articleId === firstArticle.articleId && entry.isRead && entry.isFavorite),
  `RSS filter and article state were not persisted before unsubscribe: ${JSON.stringify(stateBeforeUnsubscribe)}.`);

  const unsubscribeCallsBefore = await completedCommandCount("unsubscribe_rss");
  const unsubscribeEventCount = await harnessEventCount();
  await page.getByTestId("rss-unsubscribe").click();
  await page.getByTestId("rss-unsubscribe-confirm").click();
  const unsubscribeCall = await waitForCompletedCommand("unsubscribe_rss", unsubscribeCallsBefore);
  assert(!unsubscribeCall.result.sources.some((source) => source.id === sources.rss.id),
    "unsubscribe_rss left the standard feed in the source list.");
  const stateAfterUnsubscribe = await readJson(unsubscribeCall.result.resource);
  assert(!stateAfterUnsubscribe.subscriptions.some((entry) => entry.sourceId === sources.rss.id) &&
    !stateAfterUnsubscribe.articles.some((entry) => entry.sourceId === sources.rss.id),
  `unsubscribe_rss left per-feed state behind: ${JSON.stringify(stateAfterUnsubscribe)}.`);
  await waitForHarnessEvent("resource-updated", "rssState", unsubscribeEventCount);
  await waitForHarnessEvent("sources-updated", null, unsubscribeEventCount);
  const oldContentResponse = await fetch(firstArticle.contentSrc);
  assert(oldContentResponse.status === 404,
    `Unsubscribe did not remove the cached article HTML (HTTP ${oldContentResponse.status}).`);
  let cachedArticleHtmlExists = true;
  try {
    await access(join(dataDir, "books", `rss-${sources.rss.id}`, "chapters", `${firstArticle.articleId}.html`));
  } catch (error) {
    if (error?.code !== "ENOENT") throw error;
    cachedArticleHtmlExists = false;
  }
  assert(!cachedArticleHtmlExists, "Unsubscribe left the cached article HTML on disk.");

  const savedHomeBeforeRestart = await page.evaluate(async () =>
    window.__LEGADO_BROWSER_HARNESS__.invoke("get_home_config"));
  assert((await readJson(savedHomeBeforeRestart)).tabs[0].sections.some((entry) => entry.id === section.id),
    "The configured home section disappeared before restart.");
  verificationEvidence.homeRssCommands = (await commandLog()).map((call) => call.command);
  const requiredHomeRssCommands = [
    "get_home_config", "save_home_config", "list_discovery_categories", "list_discovery_favorites",
    "set_discovery_favorite", "list_discovery_books", "get_rss_state", "list_rss_categories",
    "list_rss_articles", "open_rss_article", "set_rss_article_state", "set_rss_filter", "unsubscribe_rss",
  ];
  for (const command of requiredHomeRssCommands) {
    assert(verificationEvidence.homeRssCommands.includes(command),
      `Browser flow did not exercise the matching Tauri/Rust command '${command}'.`);
  }
  const persistedHomeArgument = (await commandLog()).find((call) => call.command === "save_home_config")?.args.config;
  assert(persistedHomeArgument && !JSON.stringify(persistedHomeArgument).includes(fixtureOrigin) &&
    !JSON.stringify(persistedHomeArgument).includes("ruleExplore"),
  "The save_home_config command contract carried a source URL or unprocessed source rules to JS.");

  await page.goto("about:blank");
  await restartRustHarness();
  await page.goto(origin, { waitUntil: "domcontentloaded" });
  await page.getByTestId(`home-section-${section.id}`).waitFor({ timeout: 15_000 });
  const restartedBootstrap = await waitForCompletedCommand("app_bootstrap", 0);
  assert(!restartedBootstrap.result.sources.some((source) => source.id === sources.rss.id),
    "The unsubscribed feed reappeared in app_bootstrap after restarting the real Rust service.");
  const restartedHomeCall = await waitForCompletedCommand("get_home_config", 0);
  const restartedHome = await readJson(restartedHomeCall.result);
  assert(restartedHome.tabs[0].sections.some((entry) => entry.id === section.id),
    "The persisted home section did not survive restarting the real Rust service.");
  const restartedRssCall = await waitForCompletedCommand("get_rss_state", 0);
  const restartedRss = await readJson(restartedRssCall.result);
  assert(!restartedRss.subscriptions.some((entry) => entry.sourceId === sources.rss.id) &&
    !restartedRss.articles.some((entry) => entry.sourceId === sources.rss.id),
  `The RSS filter/article state returned after restart: ${JSON.stringify(restartedRss)}.`);
  const removedArticleUrl = `${resourceServerUrl}books/rss-${sources.rss.id}/chapters/${firstArticle.articleId}.html`;
  const removedArticleAfterRestart = await fetch(removedArticleUrl);
  assert(removedArticleAfterRestart.status === 404,
    `The removed RSS chapter resource returned HTTP ${removedArticleAfterRestart.status} after restart.`);
  const privateSources = JSON.parse(await readFile(join(dataDir, "private-data", "sources.json"), "utf8"));
  assert(!privateSources.some((source) => source.id === sources.rss.id),
    "The unsubscribed standard feed remained in private source storage after restart.");

  await page.getByTestId("nav-search-mobile").click();
  await page.getByTestId("discover-mode-rss").click();
  const sourceOptions = await page.locator('[data-testid="discovery-source"] option').evaluateAll((options) =>
    options.map((option) => ({ value: option.value, label: option.textContent?.trim() })));
  assert(!sourceOptions.some((option) => option.value === sources.rss.id || option.label === "Browser E2E Atom Feed"),
    `The removed Atom feed appeared in the RSS picker after application restart: ${JSON.stringify(sourceOptions)}.`);

  const directArticleRetry = await fetch(`${browserHarnessUrl}/invoke`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ command: "open_rss_article", args: { sourceId: sources.rss.id, articleId: firstArticle.articleId } }),
  });
  assert(directArticleRetry.status >= 400,
    `open_rss_article unexpectedly succeeded for an unsubscribed source after restart (HTTP ${directArticleRetry.status}).`);

  const newErrors = [
    ...errors.slice(errorStart),
    ...browserConsoleErrors.slice(consoleErrorStart).map((entry) => `console: ${entry.text}`),
    ...browserRequestFailures.slice(requestFailureStart).map((failure) => `request failed: ${failure.url} (${failure.error})`),
  ];
  assert(newErrors.length === 0, `Home/RSS browser flow reported unexpected errors:\n${newErrors.join("\n")}`);
  assert(contentFixture.discoveryRequestCount() >= 1 && contentFixture.feedRequestCount() >= 4,
    `The real KMP/Atom fixture endpoints were not exercised enough: ${contentFixture.discoveryRequestCount()} discovery requests, ${contentFixture.feedRequestCount()} feed requests.`);
  verificationEvidence.homeRss = {
    homeCategoryId: category.categoryId,
    savedHomeSectionId: section.id,
    openedProcessedCard: processedCard.title,
    rssSourceId: sources.rss.id,
    articleTitles: [firstArticle.title, secondArticle.title],
    articleSanitized: true,
    filters: [
      { filter: "read", titles: readFiltered.document.results.map((item) => item.title) },
      { filter: "favorites", titles: favoritesFiltered.document.results.map((item) => item.title) },
      { filter: "unread", titles: unreadFiltered.document.results.map((item) => item.title) },
    ],
    unsubscribedAndAbsentAfterRustRestart: true,
    discoveryRequests: contentFixture.discoveryRequestCount(),
    atomFeedRequests: contentFixture.feedRequestCount(),
  };
  report("PASS: real KMP discovery category was favorited, saved to Home, and opened from a Rust-processed card.");
  report("PASS: real feed-rs Atom resources displayed sanitized article HTML; read/favorite state and read/unread/favorite filters matched.");
  report("PASS: unsubscribe removed source, article state, and cached HTML, and stayed removed after restarting the Rust service.");
  await context.close();
}

function observeFeaturePage(flowPage) {
  const starts = {
    errors: errors.length,
    console: browserConsoleErrors.length,
    requests: browserRequestFailures.length,
  };
  flowPage.on("pageerror", (error) => errors.push(`pageerror: ${error.message}`));
  flowPage.on("console", (message) => {
    if (message.type() === "error") browserConsoleErrors.push({ text: message.text(), location: message.location() });
  });
  flowPage.on("requestfailed", (request) => browserRequestFailures.push({
    url: request.url(),
    error: request.failure()?.errorText,
  }));
  flowPage.on("request", (request) => {
    if (request.url().includes("/r/")) resourceRequests.push({ url: request.url(), method: request.method() });
  });
  flowPage.on("response", (response) => {
    if (response.url().includes("/r/")) resourceRequests.push({ url: response.url(), status: response.status() });
  });
  return {
    assertClean(label) {
      const newErrors = [
        ...errors.slice(starts.errors),
        ...browserConsoleErrors.slice(starts.console).map((entry) => `console: ${entry.text}`),
        ...browserRequestFailures.slice(starts.requests).map((failure) =>
          `request failed: ${failure.url} (${failure.error})`),
      ];
      assert(newErrors.length === 0, `${label} reported unexpected browser errors:\n${newErrors.join("\n")}`);
    },
  };
}

async function openFeatureFlowPage(viewport = { width: 390, height: 900 }) {
  const context = await browser.newContext({ viewport, deviceScaleFactor: 1 });
  await installInvokeBridge(context);
  page = await context.newPage();
  await page.emulateMedia({ reducedMotion: "reduce" });
  const diagnostics = observeFeaturePage(page);
  await page.goto(origin, { waitUntil: "domcontentloaded" });
  await page.addStyleTag({ content: "*,*::before,*::after{animation-duration:0s!important;animation-delay:0s!important;transition-duration:0s!important;transition-delay:0s!important;scroll-behavior:auto!important}" });
  await page.getByTestId("nav-search-mobile").waitFor({ timeout: 15_000 });
  await waitForCompletedCommand("app_bootstrap", 0);
  return { context, diagnostics };
}

async function sectionIdsByTitle() {
  return page.locator('input[data-testid^="home-config-section-title-"]').evaluateAll((inputs) =>
    inputs.map((input) => ({
      id: input.getAttribute("data-testid").replace("home-config-section-title-", ""),
      title: input.value,
    })),
  );
}

async function addHomeConfigSection(title) {
  const categorySelect = page.getByTestId("home-config-new-category");
  await page.waitForFunction(() => {
    const select = document.querySelector('[data-testid="home-config-new-category"]');
    return select && [...select.options].some((option) => !option.disabled && option.value);
  }, undefined, { timeout: 20_000 });
  const categoryId = await categorySelect.locator("option").evaluateAll((options) =>
    options.find((option) => !option.disabled && option.value)?.value,
  );
  assert(categoryId, `Rust did not load a selectable discovery category for home section '${title}'.`);
  await categorySelect.selectOption(categoryId);
  await page.getByTestId("home-config-new-title").fill(title);
  await page.getByTestId("home-config-add-section").click();
  await page.waitForFunction((expected) =>
    [...document.querySelectorAll('input[data-testid^="home-config-section-title-"]')]
      .some((input) => input.value === expected), title, { timeout: 10_000 });
  const row = (await sectionIdsByTitle()).find((entry) => entry.title === title);
  assert(row, `The Home editor did not add section '${title}'.`);
  return row.id;
}

async function testHomeConfigEditorFlow() {
  const { context, diagnostics } = await openFeatureFlowPage();
  await page.getByTestId("nav-settings-mobile").click();
  await page.getByTestId("home-config-open-settings").click();
  await page.getByTestId("home-config-editor").waitFor();

  const homeCall = await waitForCompletedCommand("get_home_config", 0);
  const original = await readResourceJson(homeCall.result);
  assert(original.tabs?.length === 1 && original.tabs[0].sections.length === 1,
    `The Home editor should open the category section saved by the prior Home flow: ${JSON.stringify(original)}.`);
  const mainTabId = original.tabs[0].id;
  const originalSectionId = original.tabs[0].sections[0].id;
  await page.getByTestId(`home-config-tab-name-${mainTabId}`).fill("Editor Main");

  const tabsBeforeAdd = await page.getByRole("tab").count();
  await page.getByTestId("home-config-add-tab").click();
  await page.waitForFunction((count) => document.querySelectorAll('[role="tab"]').length > count,
    tabsBeforeAdd, { timeout: 10_000 });
  const addedTabButton = page.locator('[role="tab"][aria-selected="true"]');
  const addedTabId = (await addedTabButton.getAttribute("data-testid")).replace("home-config-tab-", "");
  await page.getByTestId(`home-config-tab-name-${addedTabId}`).fill("Editor Extra");
  await page.getByTestId(`home-config-tab-up-${addedTabId}`).click();
  const extraSectionId = await addHomeConfigSection("Extra tab category");
  await page.getByTestId(`home-config-section-style-${extraSectionId}`).selectOption("2");

  await page.getByRole("tab", { name: "Editor Main", exact: true }).click();
  const secondarySectionId = await addHomeConfigSection("Editor Secondary");
  const tertiarySectionId = await addHomeConfigSection("Editor Tertiary");
  await page.getByTestId(`home-config-section-title-${secondarySectionId}`).fill("Editor Secondary Edited");
  await page.getByTestId(`home-config-section-style-${secondarySectionId}`).selectOption("3");
  await page.getByTestId(`home-config-section-style-${tertiarySectionId}`).selectOption("1");
  await page.getByTestId(`home-config-section-up-${tertiarySectionId}`).click();

  const editedSectionRows = await sectionIdsByTitle();
  assert(editedSectionRows.map((entry) => entry.id).join(",") ===
    [originalSectionId, tertiarySectionId, secondarySectionId].join(","),
  `The Home editor did not move the section into the requested order: ${JSON.stringify(editedSectionRows)}.`);
  await page.screenshot({ path: join(outputDir, "home-config-editor-crud.png"), fullPage: true });

  const firstSaveCount = await completedCommandCount("save_home_config");
  await page.getByTestId("home-config-save").click();
  const firstSave = await waitForCompletedCommand("save_home_config", firstSaveCount);
  await page.getByTestId("home-config-overlay").waitFor({ state: "detached", timeout: 15_000 });
  const saved = await readResourceJson(firstSave.result);
  assert(saved.tabs.length === 2 && saved.tabs[0].id === addedTabId && saved.tabs[0].title === "Editor Extra" &&
    saved.tabs[0].sections.length === 1 && saved.tabs[0].sections[0].id === extraSectionId && saved.tabs[0].sections[0].style === 2,
  `Home tab creation, ordering, title, or style did not persist through the Rust save command: ${JSON.stringify(saved)}.`);
  const savedMain = saved.tabs.find((tab) => tab.id === mainTabId);
  assert(savedMain?.title === "Editor Main" && savedMain.sections.map((section) => section.id).join(",") ===
    [originalSectionId, tertiarySectionId, secondarySectionId].join(",") &&
    savedMain.sections[1].style === 1 && savedMain.sections[2].style === 3,
  `Home section creation, edit, order, or styles did not persist: ${JSON.stringify(savedMain)}.`);

  await page.goto("about:blank");
  await restartRustHarness();
  await page.goto(origin, { waitUntil: "domcontentloaded" });
  await waitForCompletedCommand("app_bootstrap", 0);
  await page.getByTestId("nav-settings-mobile").click();
  await page.getByTestId("home-config-open-settings").click();
  await page.getByTestId("home-config-editor").waitFor();
  await page.getByRole("tab", { name: "Editor Extra", exact: true }).waitFor();
  await page.getByRole("tab", { name: "Editor Main", exact: true }).click();
  await page.getByTestId(`home-config-section-remove-${secondarySectionId}`).click();
  await page.getByRole("tab", { name: "Editor Extra", exact: true }).click();
  await page.getByTestId("home-config-delete-tab").click();
  const secondSaveCount = await completedCommandCount("save_home_config");
  await page.getByTestId("home-config-save").click();
  const secondSave = await waitForCompletedCommand("save_home_config", secondSaveCount);
  await page.getByTestId("home-config-overlay").waitFor({ state: "detached", timeout: 15_000 });
  const afterDeletion = await readResourceJson(secondSave.result);
  assert(afterDeletion.tabs.length === 1 && afterDeletion.tabs[0].id === mainTabId &&
    afterDeletion.tabs[0].sections.length === 2 &&
    afterDeletion.tabs[0].sections.every((section) => section.id !== secondarySectionId),
  `Home tab/section deletion did not persist: ${JSON.stringify(afterDeletion)}.`);

  await page.goto("about:blank");
  await restartRustHarness();
  await page.goto(origin, { waitUntil: "domcontentloaded" });
  await waitForCompletedCommand("app_bootstrap", 0);
  const restartedHomeCall = await waitForCompletedCommand("get_home_config", 0);
  const restartedHome = await readResourceJson(restartedHomeCall.result);
  assert(restartedHome.tabs.length === 1 && restartedHome.tabs[0].id === mainTabId &&
    restartedHome.tabs[0].title === "Editor Main" &&
    restartedHome.tabs[0].sections.length === 2 &&
    !restartedHome.tabs[0].sections.some((section) => section.id === secondarySectionId),
  `The Home tab and section deletion did not survive restarting the Rust service: ${JSON.stringify(restartedHome)}.`);

  verificationEvidence.homeConfigEditor = {
    tabCreatedRenamedAndReordered: true,
    tabDeleted: true,
    sectionCreatedRenamedAndDeleted: true,
    sectionOrder: savedMain.sections.map((section) => section.title),
    styles: savedMain.sections.map((section) => section.style),
    rustRestartRestoredSavedConfiguration: true,
    finalTabCount: restartedHome.tabs.length,
    finalSectionCount: restartedHome.tabs[0].sections.length,
  };
  report("PASS: HomeConfigEditor tab and section create/edit/delete/order/style flows used Rust saves and survived Rust service restarts.");
  diagnostics.assertClean("Home configuration editor");
  await context.close();
}

async function txtRuleIdForName(name) {
  const row = page.locator(".txt-toc-editor__rule").filter({ hasText: name });
  await row.waitFor({ timeout: 10_000 });
  const testId = await row.getAttribute("data-testid");
  assert(testId?.startsWith("txt-toc-rule-"), `Cannot locate TXT rule ID for '${name}'.`);
  return testId.slice("txt-toc-rule-".length);
}

async function txtRuleOrder() {
  return page.locator(".txt-toc-editor__rule").evaluateAll((rows) => rows.map((row) => ({
    id: row.getAttribute("data-testid").replace("txt-toc-rule-", ""),
    name: row.querySelector(".txt-toc-editor__rule-title strong")?.textContent?.trim() ?? "",
    state: row.querySelector(".txt-toc-editor__state")?.textContent?.trim() ?? "",
  })));
}

async function testTxtTocRulesAndImportFlow() {
  const { context, diagnostics } = await openFeatureFlowPage();
  await page.getByTestId("nav-settings-mobile").click();
  await page.getByTestId("txt-toc-rules-open").click();
  await page.getByTestId("txt-toc-rules-editor").waitFor();
  await page.getByTestId("txt-toc-rules-empty").waitFor();

  const invalidCount = await completedCommandCount("upsert_txt_toc_rule");
  await page.getByTestId("txt-toc-rule-name").fill("Invalid Rust regular expression");
  await page.getByTestId("txt-toc-rule-pattern").fill("[");
  await page.getByTestId("txt-toc-rule-save").click();
  await page.getByTestId("txt-toc-error").waitFor({ timeout: 10_000 });
  await page.waitForFunction((count) => {
    const calls = window.__LEGADO_BROWSER_HARNESS__.calls.filter((call) => call.command === "upsert_txt_toc_rule");
    return calls[count]?.finishedAt !== undefined;
  }, invalidCount, { timeout: 10_000 });
  const invalidCall = (await commandLog()).filter((call) => call.command === "upsert_txt_toc_rule")[invalidCount];
  assert(invalidCall.error?.includes("Invalid TXT TOC expression"),
    `The invalid expression was not rejected by Rust regex validation: ${JSON.stringify(invalidCall)}.`);
  assert((await page.getByTestId("txt-toc-error").innerText()).includes("Invalid TXT TOC expression"),
    "The TXT rule editor did not show the Rust regex validation error beside the form.");
  assert((await page.locator(".txt-toc-editor__rule").count()) === 0,
    "An invalid TXT rule appeared in the persisted rules list.");

  const asyncAddRule = async ({ name, pattern, example }) => {
    const before = await completedCommandCount("upsert_txt_toc_rule");
    await page.getByTestId("txt-toc-rule-name").fill(name);
    await page.getByTestId("txt-toc-rule-pattern").fill(pattern);
    if (example) await page.getByTestId("txt-toc-rule-example").fill(example);
    await page.getByTestId("txt-toc-rule-save").click();
    const call = await waitForCompletedCommand("upsert_txt_toc_rule", before);
    await page.getByText(name, { exact: true }).waitFor({ timeout: 10_000 });
    return call;
  };
  await asyncAddRule({
    name: "Chapter headings",
    pattern: "^Chapter [0-9]{2}: .+$",
    example: "Chapter 01: Opening",
  });
  const chapterRuleId = await txtRuleIdForName("Chapter headings");
  await asyncAddRule({
    name: "Part headings",
    pattern: "^Part [A-Z]: .+$",
    example: "Part A: Opening",
  });
  const partRuleId = await txtRuleIdForName("Part headings");
  await asyncAddRule({
    name: "Temporary heading",
    pattern: "^Never matches$",
  });
  const transientRuleId = await txtRuleIdForName("Temporary heading");

  await page.getByTestId(`txt-toc-rule-edit-${chapterRuleId}`).click();
  await page.getByTestId("txt-toc-rule-example").fill("Chapter 01: Edited opening");
  const editCount = await completedCommandCount("upsert_txt_toc_rule");
  await page.getByTestId("txt-toc-rule-save").click();
  const editedRule = await waitForCompletedCommand("upsert_txt_toc_rule", editCount);
  assert(editedRule.args.rule.id === chapterRuleId && editedRule.args.rule.example === "Chapter 01: Edited opening",
    "Editing a TXT rule did not update the existing Rust rule record.");

  await page.getByTestId(`txt-toc-rule-delete-${transientRuleId}`).click();
  await page.getByTestId("txt-toc-delete-dialog").waitFor();
  const deleteCount = await completedCommandCount("delete_txt_toc_rule");
  await page.getByTestId("txt-toc-delete-confirm").click();
  await waitForCompletedCommand("delete_txt_toc_rule", deleteCount);
  await page.getByTestId(`txt-toc-rule-${transientRuleId}`).waitFor({ state: "detached", timeout: 10_000 });

  await page.getByTestId(`txt-toc-rule-up-${partRuleId}`).click();
  await page.waitForFunction((id) => document.querySelectorAll(".txt-toc-editor__rule")[0]
    ?.getAttribute("data-testid") === `txt-toc-rule-${id}`, partRuleId, { timeout: 10_000 });
  const ruleToggle = page.getByTestId(`txt-toc-rule-toggle-${chapterRuleId}`);
  if (await ruleToggle.isChecked()) await ruleToggle.uncheck();
  await page.waitForFunction(() => document.querySelector('[data-testid="txt-toc-success"]')
    ?.textContent?.includes("规则已停用"), undefined, { timeout: 10_000 });

  const savedRuleOrder = await txtRuleOrder();
  assert(savedRuleOrder.map((rule) => rule.id).join(",") === `${partRuleId},${chapterRuleId}` &&
    savedRuleOrder[0].state === "已启用" && savedRuleOrder[1].state === "已停用",
  `TXT rule order and enable state were not reflected in the editor: ${JSON.stringify(savedRuleOrder)}.`);
  await page.screenshot({ path: join(outputDir, "txt-toc-rules-editor.png"), fullPage: true });

  const rulesResourceCall = (await commandLog()).filter((call) => call.command === "get_txt_toc_rules").at(-1);
  assert(rulesResourceCall?.result?.src, "The TXT rule editor did not load its rules from a Rust JSON resource.");
  const persistedRules = await readResourceJson(rulesResourceCall.result);
  const persistedRuleOrder = [...persistedRules.rules].sort((left, right) => left.serialNumber - right.serialNumber);
  assert(persistedRuleOrder.map((rule) => rule.id).join(",") === `${partRuleId},${chapterRuleId}` &&
    persistedRuleOrder[0].enable === true && persistedRuleOrder[1].enable === false,
  `Rust did not persist TXT rule order and enable state: ${JSON.stringify(persistedRules)}.`);

  await page.getByTestId("txt-toc-close").click();
  await page.getByTestId("nav-shelf-mobile").click();
  const importCount = await completedCommandCount("import_book_from_picker");
  await page.getByTestId("local-book-import").click();
  const importCall = await waitForCompletedCommand("import_book_from_picker", importCount, 30_000);
  const importedBook = await readResourceJson(importCall.result.book);
  const importedTitles = importedBook.chapters.map((chapter) => chapter.title);
  assert(importedBook.chapters.length === 2 && importedTitles[0] === "Part A: Part rule opening" &&
    importedTitles[1] === "Part B: Part rule ending",
  `The real TXT importer did not apply the ordered enabled Rust rule to produce exactly two chapters: ${JSON.stringify(importedBook.chapters)}.`);
  assert(importedBook.chapters.every((chapter) => chapter.src),
    "The imported TXT chapters were not persisted as browser-consumable HTML resources.");
  for (const [index, marker] of ["Fixture part-a paragraph 01", "Fixture part-b paragraph 01"].entries()) {
    const response = await fetch(importedBook.chapters[index].src);
    assert(response.ok && (await response.text()).includes(marker),
      `TXT chapter ${index + 1} HTML did not contain the corresponding imported body marker.`);
  }
  await page.locator(".book-detail-panel").waitFor({ timeout: 10_000 });
  assert(await page.locator(".catalog-row").count() === 2,
    "The local TXT book detail UI did not display the two chapters returned by the Rust import.");

  await page.goto("about:blank");
  await restartRustHarness();
  await page.goto(origin, { waitUntil: "domcontentloaded" });
  const restartedBootstrap = await waitForCompletedCommand("app_bootstrap", 0);
  const restartedShelf = await readResourceJson(restartedBootstrap.result.shelf);
  assert(restartedShelf.books.some((book) => book.id === importedBook.id),
    "The TXT book was not available on the shelf after restarting the Rust service.");
  await page.getByTestId("nav-settings-mobile").click();
  await page.getByTestId("txt-toc-rules-open").click();
  await page.getByTestId(`txt-toc-rule-${partRuleId}`).waitFor();
  const restartedRuleOrder = await txtRuleOrder();
  assert(restartedRuleOrder.map((rule) => rule.id).join(",") === `${partRuleId},${chapterRuleId}` &&
    restartedRuleOrder[0].state === "已启用" && restartedRuleOrder[1].state === "已停用",
  `TXT rule order and enable state did not survive the Rust restart: ${JSON.stringify(restartedRuleOrder)}.`);
  const reopenedBookDescriptor = await fetchJson(`${browserHarnessUrl}/invoke`, {
    command: "get_book",
    args: { bookId: importedBook.id },
  });
  const reopenedBook = await readResourceJson(reopenedBookDescriptor);
  assert(reopenedBook.chapters.length === 2 && reopenedBook.chapters.map((chapter) => chapter.title).join("|") === importedTitles.join("|"),
    `The TXT chapters did not survive the Rust restart: ${JSON.stringify(reopenedBook.chapters)}.`);

  verificationEvidence.txtTocRulesAndImport = {
    invalidRegexRejectedByRust: invalidCall.error,
    createdEditedDeletedRules: true,
    order: persistedRuleOrder.map((rule) => rule.name),
    enabled: persistedRuleOrder.map((rule) => rule.enable),
    importedBookId: importedBook.id,
    importedChapterCount: importedBook.chapters.length,
    importedChapterTitles: importedTitles,
    htmlBodiesServed: true,
    rulesAndChaptersSurvivedRustRestart: true,
  };
  report("PASS: TXT rule create/edit/delete/order/enable UI persisted through Rust, and Rust regex errors appeared beside the form.");
  report("PASS: the real local TXT importer applied the saved enabled rule, produced exactly two chapter HTML resources, and restored them with the rules after Rust restart.");
  diagnostics.assertClean("TXT rules and local import");
  await context.close();
}

async function testSearchHistoryFlow() {
  const { context, diagnostics } = await openFeatureFlowPage();
  const searchAttempts = [];
  const openSearch = async () => {
    await page.getByTestId("nav-search-mobile").click();
    await page.getByTestId("discover-mode-search").click();
  };
  await openSearch();

  const searchAndWait = async (query) => {
    const before = await completedCommandCount("start_search");
    const eventStart = await harnessEventCount();
    await page.getByTestId("search-submit").waitFor({ timeout: 10_000 });
    assert(await page.getByTestId("search-submit").isEnabled(),
      `The search UI was still busy before starting '${query}'.`);
    await page.getByTestId("search-keyword").fill(query);
    await page.getByTestId("search-submit").click();
    const call = await waitForCompletedCommand("start_search", before, 20_000);
    assert(call.result?.taskId && call.args.keyword === query,
      `The UI did not send the requested search query to Rust: ${JSON.stringify(call)}.`);
    const terminal = await waitForTaskTerminal(call.result.taskId);
    assert(terminal.status === "completed", `KMP search '${query}' did not complete: ${JSON.stringify(terminal)}.`);
    await page.locator('[data-testid^="search-result-"]').first().waitFor({ timeout: 15_000 });
    let recovered = false;
    try {
      await page.waitForFunction(() => {
        const submit = document.querySelector('[data-testid="search-submit"]');
        return submit && !submit.disabled;
      }, undefined, { timeout: 5_000 });
      recovered = true;
    } catch {
      recovered = false;
    }
    const trace = await page.evaluate(({ eventOffset, taskId }) => {
      const harness = window.__LEGADO_BROWSER_HARNESS__;
      const button = document.querySelector('[data-testid="search-submit"]');
      const taskEvents = harness.events.slice(eventOffset).map((entry) => {
        const payload = entry.payload ?? {};
        const task = payload.task && typeof payload.task === "object" ? payload.task : payload;
        return {
          id: entry.id,
          event: entry.event,
          receivedAt: entry.receivedAt,
          taskId: task.id ?? payload.taskId ?? null,
          status: task.status ?? null,
          kind: payload.kind ?? null,
          keyword: payload.keyword ?? null,
        };
      }).filter((entry) => ["task-updated", "search-started", "search-complete", "search-progress", "resource-updated"].includes(entry.event));
      return {
        taskId,
        buttonDisabled: button instanceof HTMLButtonElement ? button.disabled : null,
        buttonText: button?.textContent?.trim() ?? null,
        events: taskEvents,
      };
    }, { eventOffset: eventStart, taskId: call.result.taskId });
    const attempt = {
      query,
      taskId: call.result.taskId,
      terminalStatus: terminal.status,
      commandStartedAt: call.startedAt,
      commandFinishedAt: call.finishedAt,
      recoveredOnSameScreen: recovered,
      ...trace,
    };
    searchAttempts.push(attempt);
    verificationEvidence.searchHistoryAttempts = [...searchAttempts];
    assert(recovered,
      `Search '${query}' reached Rust task terminal state '${terminal.status}' but the same-screen submit button did not recover: ${JSON.stringify(attempt)}.`);
    return call;
  };

  for (const query of ["History Alpha", "History Beta", "History Alpha", "History Gamma"]) {
    await searchAndWait(query);
  }

  await page.getByTestId("nav-settings-mobile").click();
  await page.getByTestId("search-history").waitFor();
  await page.waitForFunction(() => {
    const rows = [...document.querySelectorAll('[data-testid^="search-history-entry-"]')];
    return rows[0]?.getAttribute("data-query") === "History Gamma" &&
      rows.find((row) => row.getAttribute("data-query") === "History Alpha")?.innerText.includes("2 次搜索");
  }, undefined, { timeout: 10_000 });
  const recentRows = await page.locator('[data-testid^="search-history-entry-"]').evaluateAll((rows) =>
    rows.map((row) => ({ query: row.getAttribute("data-query"), text: row.innerText })),
  );
  assert(recentRows[0]?.query === "History Gamma" && recentRows.find((row) => row.query === "History Alpha")?.text.includes("2 次搜索"),
    `SearchHistory did not show recent order and deduplicated usage counts: ${JSON.stringify(recentRows)}.`);

  await page.getByTestId("search-history-sort-popular").click();
  const popularRows = await page.locator('[data-testid^="search-history-entry-"]').evaluateAll((rows) =>
    rows.map((row) => ({ query: row.getAttribute("data-query"), text: row.innerText })),
  );
  assert(popularRows[0]?.query === "History Alpha" && popularRows[0].text.includes("2 次搜索"),
    `The popular-search view did not rank the most used query first: ${JSON.stringify(popularRows)}.`);
  await page.screenshot({ path: join(outputDir, "search-history-popular.png"), fullPage: true });

  const betaRow = page.locator('[data-testid^="search-history-entry-"][data-query="History Beta"]');
  const reuseCount = await completedCommandCount("start_search");
  const reuseEventStart = await harnessEventCount();
  await betaRow.locator('[data-testid^="search-history-use-"]').click();
  const reuseCall = await waitForCompletedCommand("start_search", reuseCount, 20_000);
  assert(reuseCall.args.keyword === "History Beta", `Selecting a recent search did not reuse its query: ${JSON.stringify(reuseCall)}.`);
  const reusedTask = await waitForTaskTerminal(reuseCall.result.taskId);
  assert(reusedTask.status === "completed", `Reused KMP search did not complete: ${JSON.stringify(reusedTask)}.`);
  assert(await page.getByTestId("search-keyword").inputValue() === "History Beta",
    "The search form did not receive the selected history query.");
  const reuseRecovered = await page.waitForFunction(() => {
    const submit = document.querySelector('[data-testid="search-submit"]');
    return submit && !submit.disabled;
  }, undefined, { timeout: 5_000 }).then(() => true).catch(() => false);
  const reuseTrace = await page.evaluate(({ taskId, eventOffset }) => {
    const harness = window.__LEGADO_BROWSER_HARNESS__;
    const button = document.querySelector('[data-testid="search-submit"]');
    const events = harness.events.slice(eventOffset).filter((entry) => {
      const payload = entry.payload ?? {};
      const task = payload.task && typeof payload.task === "object" ? payload.task : payload;
      return ["task-updated", "search-started", "search-complete", "search-progress", "resource-updated"].includes(entry.event)
        && (task.id === taskId || payload.taskId === taskId || entry.event === "resource-updated" && payload.kind === "searchHistory");
    }).map((entry) => ({
      id: entry.id,
      event: entry.event,
      receivedAt: entry.receivedAt,
      status: entry.payload?.task?.status ?? entry.payload?.status ?? null,
      kind: entry.payload?.kind ?? null,
    }));
    return {
      taskId,
      buttonDisabled: button instanceof HTMLButtonElement ? button.disabled : null,
      buttonText: button?.textContent?.trim() ?? null,
      events,
    };
  }, { taskId: reuseCall.result.taskId, eventOffset: reuseEventStart });
  const reuseAttempt = {
    query: reuseCall.args.keyword,
    taskId: reuseCall.result.taskId,
    terminalStatus: reusedTask.status,
    commandStartedAt: reuseCall.startedAt,
    commandFinishedAt: reuseCall.finishedAt,
    recoveredOnSameScreen: reuseRecovered,
    ...reuseTrace,
  };
  searchAttempts.push(reuseAttempt);
  verificationEvidence.searchHistoryAttempts = [...searchAttempts];
  assert(reuseRecovered,
    `Reused query task '${reuseAttempt.taskId}' completed but same-screen submit did not recover: ${JSON.stringify(reuseAttempt)}.`);

  await page.getByTestId("nav-settings-mobile").click();
  await page.getByTestId("search-history").waitFor();
  await page.getByTestId("search-history-sort-recent").click();
  await page.locator('[data-testid^="search-history-entry-"][data-query="History Beta"]').waitFor();
  assert((await page.locator('[data-testid^="search-history-entry-"]').first().getAttribute("data-query")) === "History Beta",
    "Reusing a search did not update its recent position.");

  const deleteCount = await completedCommandCount("delete_search_history");
  await page.locator('[data-testid^="search-history-entry-"][data-query="History Gamma"] [data-testid^="search-history-delete-"]').click();
  await waitForCompletedCommand("delete_search_history", deleteCount);
  await page.waitForFunction(() => ![...document.querySelectorAll('[data-testid^="search-history-entry-"]')]
    .some((row) => row.getAttribute("data-query") === "History Gamma"), undefined, { timeout: 10_000 });

  await page.goto("about:blank");
  await restartRustHarness();
  await page.goto(origin, { waitUntil: "domcontentloaded" });
  const afterDeleteBootstrap = await waitForCompletedCommand("app_bootstrap", 0);
  const historyAfterDeleteRestart = await readResourceJson(afterDeleteBootstrap.result.searchHistory);
  assert(!historyAfterDeleteRestart.entries.some((entry) => entry.query === "History Gamma") &&
    historyAfterDeleteRestart.entries.some((entry) => entry.query === "History Alpha") &&
    historyAfterDeleteRestart.entries.some((entry) => entry.query === "History Beta"),
  `Search history deletion or retained records did not survive restart: ${JSON.stringify(historyAfterDeleteRestart)}.`);

  await page.getByTestId("nav-settings-mobile").click();
  const clearCount = await completedCommandCount("clear_search_history");
  await page.getByTestId("search-history-clear").click();
  await page.getByTestId("search-history-clear-confirm").click();
  await waitForCompletedCommand("clear_search_history", clearCount);
  await page.getByTestId("search-history-empty").waitFor({ timeout: 10_000 });
  await page.screenshot({ path: join(outputDir, "search-history-cleared.png"), fullPage: true });

  await page.goto("about:blank");
  await restartRustHarness();
  await page.goto(origin, { waitUntil: "domcontentloaded" });
  const afterClearBootstrap = await waitForCompletedCommand("app_bootstrap", 0);
  const historyAfterClearRestart = await readResourceJson(afterClearBootstrap.result.searchHistory);
  assert(historyAfterClearRestart.entries.length === 0,
    `Clearing SearchHistory did not survive Rust restart: ${JSON.stringify(historyAfterClearRestart)}.`);
  await page.getByTestId("nav-settings-mobile").click();
  await page.getByTestId("search-history-empty").waitFor();
  await page.screenshot({ path: join(outputDir, "search-history-empty-after-restart.png"), fullPage: true });

  verificationEvidence.searchHistory = {
    recentOrder: recentRows.slice(0, 3).map((row) => row.query),
    popularOrder: popularRows.slice(0, 3).map((row) => row.query),
    reuseQuery: reuseCall.args.keyword,
    sameScreenSearchRecoveredAfterTask: searchAttempts.map((attempt) => attempt.recoveredOnSameScreen),
    individualDeletePersistedThroughRestart: true,
    clearPersistedThroughRestart: true,
  };
  report("PASS: SearchHistory displayed recent/popular ordering, reused a query through a real KMP/Rust search, and every same-screen search submit recovered after Rust task completion.");
  report("PASS: individual deletion and clear-all remained persisted after restarting the real Rust service.");
  diagnostics.assertClean("Search history");
  await context.close();
}

async function testBookSourceChangeFlow(bookId, sources, contentFixture) {
  const context = await browser.newContext({ viewport: { width: 390, height: 900 }, deviceScaleFactor: 1 });
  await installInvokeBridge(context);
  page = await context.newPage();
  const errorStart = errors.length;
  const consoleErrorStart = browserConsoleErrors.length;
  const requestFailureStart = browserRequestFailures.length;
  const bookJsonSnapshots = [];
  const pendingBookJsonSnapshots = [];
  page.on("pageerror", (error) => errors.push(`pageerror: ${error.message}`));
  page.on("console", (message) => {
    if (message.type() === "error") browserConsoleErrors.push({ text: message.text(), location: message.location() });
  });
  page.on("request", (request) => {
    if (request.url().includes("/r/")) resourceRequests.push({ url: request.url(), method: request.method() });
  });
  page.on("requestfailed", (request) => browserRequestFailures.push({ url: request.url(), error: request.failure()?.errorText }));
  page.on("response", (response) => {
    if (response.url().includes("/r/")) {
      resourceRequests.push({ url: response.url(), status: response.status() });
      if (response.url().includes(`/books/${bookId}/book.json`)) {
        pendingBookJsonSnapshots.push(response.json().then((document) => bookJsonSnapshots.push(document)).catch(() => {}));
      }
    }
  });

  const readJson = async (descriptor) => {
    const src = typeof descriptor === "string" ? descriptor : descriptor?.src;
    assert(src, `A Rust command did not return a resource URL: ${JSON.stringify(descriptor)}.`);
    const response = await fetch(src);
    assert(response.ok, `Resource ${src} returned HTTP ${response.status}.`);
    return response.json();
  };
  const invokeFromPage = async (command, args = {}) => page.evaluate(({ commandName, commandArgs }) =>
    window.__LEGADO_BROWSER_HARNESS__.invoke(commandName, commandArgs), { commandName: command, commandArgs: args });
  const saveBookmark = async (note) => {
    const before = await completedCommandCount("upsert_bookmark");
    await page.getByTestId("reader-add-bookmark").click();
    await page.locator(".bookmark-modal textarea").fill(note);
    await page.locator(".bookmark-modal").getByRole("button", { name: "保存书签" }).click();
    const call = await waitForCompletedCommand("upsert_bookmark", before);
    const document = await readJson(call.result);
    const bookmark = document.bookmarks?.find((entry) => entry.note === note && entry.bookId === bookId);
    assert(bookmark, `The UI did not persist the '${note}' bookmark: ${JSON.stringify(document)}.`);
    return bookmark;
  };

  await page.goto(origin, { waitUntil: "domcontentloaded" });
  await page.getByTestId("nav-shelf-mobile").waitFor();
  const bootstrapCall = await waitForCompletedCommand("app_bootstrap", 0);
  assert(bootstrapCall.result.shelf?.src, "Source-switch flow did not load its shelf through Rust resources.");
  await page.getByTestId("nav-shelf-mobile").click();
  const shelfCard = page.getByTestId(`shelf-book-${bookId}`);
  await shelfCard.waitFor({ timeout: 15_000 });
  await shelfCard.locator("button.cover-button").click();
  const bookDetail = page.locator(".book-detail-panel");
  await bookDetail.waitFor();
  const beforeBookmarks = (await getBookDocument(bookId)).document;
  assert(beforeBookmarks.id === bookId && beforeBookmarks.chapters.map((chapter) => chapter.title).join("|") ===
    "Fixture Chapter Two|Fixture Chapter One",
  `The source-switch flow did not start from the persisted reordered catalog: ${JSON.stringify(beforeBookmarks.chapters)}.`);
  const oldProgressChapterId = beforeBookmarks.progress.chapterId;
  assert(oldProgressChapterId === beforeBookmarks.chapters[0].id && beforeBookmarks.progress.chapterIndex === 0,
    `The starting progress is not anchored to Fixture Chapter Two: ${JSON.stringify(beforeBookmarks.progress)}.`);
  await bookDetail.getByRole("button", { name: "继续阅读" }).click();
  await page.getByTestId("reader-frame").waitFor({ timeout: 45_000 });
  await waitForReaderChapter("Fixture Chapter Two");
  await saveBookmark("migrate to the same chapter title");
  await page.getByTestId("reader-next-chapter").click();
  await waitForReaderChapter("Fixture Chapter One");
  const orphanBookmark = await saveBookmark("keep the unmatched chapter bookmark");
  await page.getByTestId("reader-prev-chapter").click();
  await waitForReaderChapter("Fixture Chapter Two");
  const originalPage = readPageIndicator(await page.getByTestId("reader-page-indicator").innerText());
  const expectedOffset = originalPage.current - 1;
  assert(expectedOffset > 0 && originalPage.current === beforeBookmarks.progress.offset + 1,
    `The original chapter/page progress did not reopen before source change: ${JSON.stringify({ originalPage, progress: beforeBookmarks.progress })}.`);
  assert((await readerFrameText()).includes("第二章标记"), "The original source chapter was not displayed before source change.");

  await page.getByRole("button", { name: "阅读显示设置" }).click();
  await page.getByTestId("reader-change-source").waitFor();
  const privateBookPath = join(dataDir, "private-data", "books", `${bookId}.json`);
  const bookDiskPath = join(dataDir, "books", bookId, "book.json");
  const originalPrivateBook = JSON.parse(await readFile(privateBookPath, "utf8"));
  const originalBookDocument = beforeBookmarks;
  for (const chapter of originalBookDocument.chapters) {
    const cacheResponse = await fetch(chapter.src);
    assert(cacheResponse.ok, `Original source cache returned HTTP ${cacheResponse.status} before source change.`);
  }

  const sourceSwitchOpenCalls = await completedCommandCount("search_book_source_candidates");
  await page.getByTestId("reader-change-source").click();
  await page.getByTestId("source-switch-results").waitFor({ timeout: 45_000 });
  const searchCall = await waitForCompletedCommand("search_book_source_candidates", sourceSwitchOpenCalls, 15_000);
  assert(searchCall.args.bookId === bookId && searchCall.args.sourceIds.includes(sources.replacement.id) &&
    searchCall.args.sourceIds.includes(sources.broken.id),
  `Candidate search did not use the actual book and imported target sources: ${JSON.stringify(searchCall.args)}.`);
  assert(!JSON.stringify(searchCall.args).includes(fixtureOrigin) && !JSON.stringify(searchCall.args).match(/ruleSearch|searchUrl/),
    "Candidate search IPC exposed a source URL or source rule to the WebView.");
  const searchTask = await waitForTaskTerminal(searchCall.result.taskId);
  assert(searchTask.status === "completed", `Source candidate search did not complete: ${JSON.stringify(searchTask)}.`);
  const candidateDocument = await readJson(searchCall.result.resource);
  assert(candidateDocument.complete === true && candidateDocument.errors.length === 0,
    `Source candidate search did not return a complete result: ${JSON.stringify(candidateDocument)}.`);
  assert(candidateDocument.results.length === 2,
    `Candidate identity filtering should leave the two same-title sources with unknown authors only: ${JSON.stringify(candidateDocument.results)}.`);
  assert(candidateDocument.results.every((result) => result.requiresIdentityConfirmation === true),
    `Unknown authors must require explicit identity confirmation: ${JSON.stringify(candidateDocument.results)}.`);
  assert(candidateDocument.results.some((result) => result.sourceId === sources.replacement.id) &&
    candidateDocument.results.some((result) => result.sourceId === sources.broken.id),
  `The valid and failed-catalog candidates were not both returned: ${JSON.stringify(candidateDocument.results)}.`);
  assert(!candidateDocument.results.some((result) => [sources.wrongAuthor.id, sources.wrongTitle.id].includes(result.sourceId)),
    `Identity filtering kept the wrong-author or wrong-title candidate: ${JSON.stringify(candidateDocument.results)}.`);
  // Search results may carry the engine-processed book URL used by Rust to resolve
  // the opaque result ID. They must never carry the source definition or rule set.
  assert(!JSON.stringify(candidateDocument).match(/bookSourceUrl|ruleSearch|searchUrl/),
    "The processed candidate resource exposed an unprocessed source definition.");
  const browserCandidateRows = await page.locator('[data-testid^="source-switch-candidate-"]').evaluateAll((rows) =>
    rows.map((row) => ({ id: row.getAttribute("data-testid"), text: row.innerText })));
  assert(browserCandidateRows.length === 2 && browserCandidateRows.some((row) => row.text.includes("Replacement without author")) &&
    browserCandidateRows.some((row) => row.text.includes("Empty replacement catalog")),
  `The UI did not display exactly the processed identity-matching candidates: ${JSON.stringify(browserCandidateRows)}.`);
  await page.screenshot({ path: join(outputDir, "source-switch-candidates.png"), fullPage: true, animations: "disabled" });

  const candidatesBySource = new Map(candidateDocument.results.map((result) => [result.sourceId, result]));
  const brokenCandidate = candidatesBySource.get(sources.broken.id);
  const replacementCandidate = candidatesBySource.get(sources.replacement.id);
  const baselineBeforeFailure = (await getBookDocument(bookId)).document;
  const baselinePrivateBeforeFailure = JSON.parse(await readFile(privateBookPath, "utf8"));
  const baselineBookDiskBeforeFailure = await readFile(bookDiskPath, "utf8");
  const baselineCacheHashes = new Map();
  for (const chapter of baselineBeforeFailure.chapters) {
    baselineCacheHashes.set(chapter.id, createHash("sha256").update(
      await readFile(join(dataDir, "books", bookId, "chapters", `${chapter.id}.html`)),
    ).digest("hex"));
  }
  const bookSourceEventsBeforeFailure = (await page.evaluate(() => window.__LEGADO_BROWSER_HARNESS__.events))
    .filter((event) => event.event === "book-source-changed").length;

  await page.getByTestId(`source-switch-select-${brokenCandidate.resultId}`).click();
  await page.getByTestId("source-switch-confirm-open").click();
  await page.locator(".source-identity-confirmation").filter({ hasText: "缺少可核实的作者" }).waitFor();
  assert((await page.getByTestId("source-switch-confirm").innerText()).includes("确认书名正确并更换"),
    "A missing-author candidate did not make the user explicitly confirm identity in the dialog.");
  const changeCallsBeforeRejectedAuthor = await completedCommandCount("change_book_source");
  await page.getByTestId("source-switch-confirm").click();
  const brokenCatalogCall = await waitForFinishedCommand("change_book_source", changeCallsBeforeRejectedAuthor, 30_000);
  assert(brokenCatalogCall.args.confirmMissingAuthor === true &&
    /chapter_list_empty|no chapters|invalid chapter catalog/i.test(brokenCatalogCall.error ?? ""),
  `The confirmed empty replacement catalog should fail before committing: ${JSON.stringify(brokenCatalogCall)}.`);
  const afterBrokenCatalog = (await getBookDocument(bookId)).document;
  const privateAfterBrokenCatalog = JSON.parse(await readFile(privateBookPath, "utf8"));
  assert(JSON.stringify(afterBrokenCatalog) === JSON.stringify(baselineBeforeFailure),
    "A failed replacement catalog changed the public book JSON or progress.");
  assert(JSON.stringify(privateAfterBrokenCatalog) === JSON.stringify(baselinePrivateBeforeFailure) &&
    privateAfterBrokenCatalog.sourceId === originalPrivateBook.sourceId,
  "A failed replacement catalog changed the private source binding or engine book data.");
  assert(await readFile(bookDiskPath, "utf8") === baselineBookDiskBeforeFailure,
    "A failed replacement catalog changed the public book JSON file on disk.");
  for (const chapter of baselineBeforeFailure.chapters) {
    const cachePath = join(dataDir, "books", bookId, "chapters", `${chapter.id}.html`);
    assert(createHash("sha256").update(await readFile(cachePath)).digest("hex") === baselineCacheHashes.get(chapter.id),
      `A failed catalog replacement changed cached chapter '${chapter.title}'.`);
    assert((await fetch(chapter.src)).ok, `A failed catalog replacement removed original chapter '${chapter.title}'.`);
  }
  const bookSourceEventsAfterFailure = (await page.evaluate(() => window.__LEGADO_BROWSER_HARNESS__.events))
    .filter((event) => event.event === "book-source-changed").length;
  assert(bookSourceEventsAfterFailure === bookSourceEventsBeforeFailure,
    "A failed source change emitted the success event book-source-changed.");
  await page.getByRole("button", { name: "返回候选列表" }).click();

  await page.getByTestId(`source-switch-select-${replacementCandidate.resultId}`).click();
  await page.getByTestId("source-switch-confirm-open").click();
  await page.locator(".source-identity-confirmation").filter({ hasText: "缺少可核实的作者" }).waitFor();
  assert((await page.getByTestId("source-switch-confirm").innerText()).includes("确认书名正确并更换"),
    "The valid missing-author candidate did not ask for explicit identity confirmation.");
  const confirmEventCount = await harnessEventCount();
  const preMutationBookSnapshots = bookJsonSnapshots.length;
  const successfulChangeIndex = await completedCommandCount("change_book_source");
  await page.getByTestId("source-switch-confirm").click();
  const successfulChangeCall = await waitForCompletedCommand("change_book_source", successfulChangeIndex, 45_000);
  assert(successfulChangeCall.args.bookId === bookId && successfulChangeCall.args.resultId === replacementCandidate.resultId &&
    successfulChangeCall.args.confirmMissingAuthor === true,
  `The UI did not send the selected opaque result ID and positive author confirmation: ${JSON.stringify(successfulChangeCall.args)}.`);
  assert(!JSON.stringify(successfulChangeCall.args).includes(fixtureOrigin) &&
    !JSON.stringify(successfulChangeCall.args).match(/ruleSearch|searchUrl/),
  "change_book_source IPC exposed a source URL or source rules.");
  assert(successfulChangeCall.result?.book?.src && successfulChangeCall.result?.progress?.chapterIndex === 1 &&
    successfulChangeCall.result?.progress?.offset === expectedOffset && successfulChangeCall.result?.movedProgress === true,
  `Source change did not remap unique-title progress while preserving page offset: ${JSON.stringify(successfulChangeCall.result)}.`);
  await waitForHarnessEvent("book-source-changed", null, confirmEventCount);
  await waitForHarnessEvent("progress-saved", null, confirmEventCount);
  await waitForHarnessEvent("resource-updated", "bookmarks", confirmEventCount);
  await waitForReaderChapter("Fixture Chapter Two");
  await page.getByTestId("reader-frame").waitFor({ timeout: 45_000 });
  await page.waitForFunction(() => document.querySelector('[data-testid="reader-frame"]')?.contentDocument?.body?.innerText.includes("Replacement-source chapter marker"), null, { timeout: 15_000 });
  await page.screenshot({ path: join(outputDir, "source-switch-reader.png"), fullPage: true, animations: "disabled" });
  await Promise.all(pendingBookJsonSnapshots);

  const immediateSnapshots = bookJsonSnapshots.slice(preMutationBookSnapshots);
  const oldChapterIds = new Set(originalBookDocument.chapters.map((chapter) => chapter.id));
  assert(immediateSnapshots.some((document) => document.chapters?.[1]?.title === "Fixture Chapter Two" &&
    document.chapters[1].src == null && !oldChapterIds.has(document.chapters[1].id)),
    "The resource fetched immediately after source commit did not show the replacement chapter as uncached before JS asked Rust to prepare it.");
  const prepareCalls = (await commandLog()).filter((call) => call.command === "prepare_chapters" && call.finishedAt !== undefined);
  const replacementPrepare = prepareCalls.find((call) => call.args.bookId === bookId && call.args.fromIndex === 1 && call.args.count === 1 &&
    call.result?.prepared === 1 && call.startedAt >= successfulChangeCall.finishedAt);
  assert(replacementPrepare, "The UI did not request a one-chapter Rust cache for the uncached replacement chapter.");
  const changedBook = (await getBookDocument(bookId)).document;
  assert(changedBook.id === bookId && changedBook.chapters.length === 3 &&
    changedBook.chapters.map((chapter) => chapter.title).join("|") ===
      "Replacement Opening|Fixture Chapter Two|Replacement Ending",
  `Replacement source directory did not replace the old chapter list: ${JSON.stringify(changedBook.chapters)}.`);
  const changedChapter = changedBook.chapters[1];
  assert(changedChapter.id !== oldProgressChapterId && changedChapter.src &&
    changedBook.progress.chapterId === changedChapter.id && changedBook.progress.chapterIndex === 1 &&
    changedBook.progress.offset === expectedOffset,
  `The title-matched replacement chapter did not receive new chapter identity and mapped progress: ${JSON.stringify({ changedChapter, progress: changedBook.progress })}.`);
  const replacementHtmlResponse = await fetch(changedChapter.src);
  assert(replacementHtmlResponse.ok, `Replacement chapter HTML resource returned HTTP ${replacementHtmlResponse.status}.`);
  const replacementHtml = await replacementHtmlResponse.text();
  assert(replacementHtml.includes("Replacement-source chapter marker") && replacementHtml.includes("font-size:"),
    "The replacement source chapter was not cached as browser-ready HTML with default reading styles.");
  assert(!replacementHtml.includes("ruleSearch") && !replacementHtml.includes("searchUrl") &&
    !replacementHtml.includes(fixtureOrigin),
  "Cached replacement chapter HTML exposed source rules or the private source origin.");
  for (const chapter of originalBookDocument.chapters) {
    const oldCachePath = join(dataDir, "books", bookId, "chapters", `${chapter.id}.html`);
    let oldCacheExists = true;
    try { await access(oldCachePath); } catch (error) {
      if (error?.code !== "ENOENT") throw error;
      oldCacheExists = false;
    }
    assert(!oldCacheExists, `Source change left old-source chapter '${chapter.title}' cache on disk.`);
    assert((await fetch(chapter.src)).status === 404,
      `Source change left the old chapter resource URL readable for '${chapter.title}'.`);
  }
  const changedBookmarks = await readJson(successfulChangeCall.result.bookmarks.resource);
  const migratedBookmark = changedBookmarks.bookmarks.find((entry) => entry.note === "migrate to the same chapter title");
  const orphaned = changedBookmarks.bookmarks.find((entry) => entry.id === orphanBookmark.id);
  assert(migratedBookmark && migratedBookmark.chapterIndex === 1 && migratedBookmark.orphaned !== true,
    `The bookmark for the unique matching title did not migrate with its chapter: ${JSON.stringify(migratedBookmark)}.`);
  assert(orphaned?.orphaned === true && orphaned.chapterTitle === "Fixture Chapter One",
    `The unmatched original bookmark was not retained and marked orphaned: ${JSON.stringify(orphaned)}.`);

  const newPrivateBook = JSON.parse(await readFile(privateBookPath, "utf8"));
  assert(newPrivateBook.sourceId === sources.replacement.id,
    `The private book source binding did not move to the replacement source: ${newPrivateBook.sourceId}.`);
  const changedBookDisk = await readFile(bookDiskPath, "utf8");
  assert(!changedBookDisk.includes(new URL(resourceServerUrl).origin) && !changedBookDisk.includes(fixtureOrigin) &&
    !changedBookDisk.match(/ruleSearch|searchUrl/),
  "Persistent public book JSON contains an HTTP port, private source URL, or source rules.");
  assert(!JSON.stringify(successfulChangeCall.result).match(/ruleSearch|searchUrl|bookSourceUrl/),
    "The source-change command response exposed source definitions or rules.");

  const progressCallsBeforeLeave = await completedCommandCount("save_progress");
  await page.getByTestId("reader-back").click();
  await waitForCompletedCommand("save_progress", progressCallsBeforeLeave, 15_000);
  const bookAfterLeave = (await getBookDocument(bookId)).document;
  assert(bookAfterLeave.progress.chapterId === changedChapter.id && bookAfterLeave.progress.chapterIndex === 1 &&
    bookAfterLeave.progress.offset === expectedOffset,
  `Leaving the replacement reader did not persist the mapped chapter and page: ${JSON.stringify(bookAfterLeave.progress)}.`);

  const flowCallsBeforeRestart = await commandLog();
  const resourceOriginBeforeRestart = new URL(resourceServerUrl).origin;
  await page.goto("about:blank");
  await restartRustHarness();
  const resourceOriginAfterRestart = new URL(resourceServerUrl).origin;
  await page.goto(origin, { waitUntil: "domcontentloaded" });
  await page.getByTestId("nav-shelf-mobile").waitFor();
  const restartedBootstrap = await waitForCompletedCommand("app_bootstrap", 0);
  assert(restartedBootstrap.result.shelf?.src &&
    restartedBootstrap.result.sources.some((source) => source.id === sources.replacement.id),
  "The replacement source or shelf resource disappeared after restarting the Rust service.");
  await page.getByTestId("nav-shelf-mobile").click();
  await page.getByTestId(`shelf-book-${bookId}`).waitFor({ timeout: 15_000 });
  await page.getByTestId(`shelf-book-${bookId}`).locator("button.cover-button").click();
  await page.locator(".book-detail-panel").waitFor();
  const reopenedBook = (await getBookDocument(bookId)).document;
  assert(reopenedBook.id === bookId && reopenedBook.chapters[1].id === changedChapter.id &&
    reopenedBook.chapters[1].src && reopenedBook.progress.chapterId === changedChapter.id &&
    reopenedBook.progress.chapterIndex === 1 && reopenedBook.progress.offset === expectedOffset,
  `The new source, cached HTML URL, or mapped progress did not survive reopening the Rust service: ${JSON.stringify(reopenedBook)}.`);
  assert(!reopenedBook.chapters[1].src.includes(resourceOriginBeforeRestart) ||
    resourceOriginBeforeRestart === resourceOriginAfterRestart,
  "The book JSON persisted the previous process's temporary resource-server origin.");
  const reopenedHtmlResponse = await fetch(reopenedBook.chapters[1].src);
  assert(reopenedHtmlResponse.ok && (await reopenedHtmlResponse.text()).includes("Replacement-source chapter marker"),
    "The replacement chapter HTML resource was not served after restarting Rust.");
  const reopenedPrivateBook = JSON.parse(await readFile(privateBookPath, "utf8"));
  assert(reopenedPrivateBook.sourceId === sources.replacement.id,
    "The private replacement source binding did not survive the Rust service restart.");

  const bookmarksCallsBefore = await completedCommandCount("list_bookmarks");
  await invokeFromPage("list_bookmarks");
  const bookmarksCall = await waitForCompletedCommand("list_bookmarks", bookmarksCallsBefore);
  const reopenedBookmarks = await readJson(bookmarksCall.result);
  const reopenedOrphan = reopenedBookmarks.bookmarks.find((entry) => entry.id === orphanBookmark.id);
  assert(reopenedOrphan?.orphaned === true && reopenedOrphan.chapterTitle === "Fixture Chapter One",
    `The orphaned bookmark was not persisted through restart: ${JSON.stringify(reopenedOrphan)}.`);
  await page.locator(".book-detail-panel").getByRole("button", { name: "关闭详情" }).click();
  await page.getByTestId("nav-settings-mobile").click();
  const orphanButton = page.getByTestId(`bookmark-open-${orphanBookmark.id}`);
  await orphanButton.waitFor({ timeout: 15_000 });
  assert(await orphanButton.isDisabled(), "The restored unmatched bookmark can still be opened against the replacement catalog.");
  assert((await orphanButton.locator("xpath=..").innerText()).includes("无法匹配"),
    "The settings UI did not tell the user that the retained bookmark no longer matches a chapter.");

  const privateSources = JSON.parse(await readFile(join(dataDir, "private-data", "sources.json"), "utf8"));
  assert(privateSources.some((source) => source.id === sources.replacement.id && source.source?.searchUrl && source.source?.ruleSearch) &&
    privateSources.some((source) => source.id === originalPrivateBook.sourceId && source.source?.searchUrl && source.source?.ruleSearch),
  "The Rust service did not retain current and replacement source definitions privately.");
  const appBootstrapJson = JSON.stringify(restartedBootstrap.result);
  assert(!appBootstrapJson.includes(fixtureOrigin) && !appBootstrapJson.match(/ruleSearch|searchUrl|bookSourceUrl/),
    "The public app bootstrap exposed source definitions or source URLs.");

  const flowCalls = [...flowCallsBeforeRestart, ...(await commandLog())];
  verificationEvidence.sourceSwitchCommands = flowCalls.map((call) => call.command);
  for (const required of ["search_book_source_candidates", "change_book_source", "prepare_chapters", "upsert_bookmark", "save_progress", "list_bookmarks"]) {
    assert(verificationEvidence.sourceSwitchCommands.includes(required),
      `The source-change browser flow did not call the Rust command '${required}'.`);
  }
  assert(sourceRequests.some((request) => request.url.startsWith("/replacement/search")) &&
    sourceRequests.some((request) => request.url.startsWith("/replacement/book")) &&
    sourceRequests.some((request) => request.url.startsWith("/replacement/toc")) &&
    sourceRequests.some((request) => request.url.startsWith("/replacement/chapter/two")) &&
    sourceRequests.some((request) => request.url.startsWith("/broken/toc")) &&
    sourceRequests.some((request) => request.url.startsWith("/wrong-author/search")) &&
    sourceRequests.some((request) => request.url.startsWith("/wrong-title/search")),
  "The KMP/JNI source engine did not execute all actual replacement, failed-catalog, wrong-author, and wrong-title fixture operations.");

  const newErrors = [
    ...errors.slice(errorStart),
    ...browserConsoleErrors.slice(consoleErrorStart).map((entry) => `console: ${entry.text}`),
    ...browserRequestFailures.slice(requestFailureStart).map((failure) => `request failed: ${failure.url} (${failure.error})`),
  ];
  assert(newErrors.length === 0, `Book source change browser flow reported unexpected errors:\n${newErrors.join("\n")}`);
  verificationEvidence.bookSourceChange = {
    bookId,
    candidateSources: candidateDocument.results.map((result) => result.sourceId),
    excludedWrongAuthorAndWrongTitle: true,
    explicitMissingAuthorConfirmation: true,
    failedCatalogPreservedOldBookAndCaches: true,
    oldCatalog: originalBookDocument.chapters.map((chapter) => chapter.title),
    newCatalog: changedBook.chapters.map((chapter) => chapter.title),
    progressBefore: baselineBeforeFailure.progress,
    progressAfter: bookAfterLeave.progress,
    progressMovedToTitleMatch: true,
    offsetPreserved: expectedOffset,
    migratedBookmarkId: migratedBookmark.id,
    orphanedBookmarkId: orphanBookmark.id,
    newChapterId: changedChapter.id,
    oldCacheResourcesRemoved: true,
    replacementHtmlServedAfterRestart: true,
    resourceOriginBeforeRestart,
    resourceOriginAfterRestart,
  };
  report("PASS: real KMP/JNI candidate search filtered wrong-author/title sources and exposed only processed candidate metadata.");
  report("PASS: missing-author replacement required explicit confirmation; an empty new catalog failed without changing the original book or chapter caches.");
  report("PASS: replacement reloaded processed HTML through Rust, mapped chapter/page progress by unique title, migrated a matching bookmark, and retained the unmatched bookmark as disabled/orphaned.");
  report("PASS: replacement source binding, cached HTML, mapped progress, and orphan bookmark survived Rust service restart without persisting a resource-server port in book JSON.");
  await context.close();
}

async function testBookMetadataRefreshFlow(bookId, contentFixture) {
  const { context, diagnostics } = await openFeatureFlowPage();
  await page.getByTestId("nav-shelf-mobile").click();
  const shelfCard = page.getByTestId(`shelf-book-${bookId}`);
  await shelfCard.waitFor({ timeout: 15_000 });
  await shelfCard.locator("button.cover-button").click();
  const detail = page.locator(".book-detail-panel");
  await detail.waitFor();
  const refreshButton = page.getByTestId("book-refresh-info");
  await refreshButton.waitFor({ timeout: 10_000 });

  const before = (await getBookDocument(bookId)).document;
  assert(before.id === bookId && before.sourceId && before.canChangeSource === true,
    `The online book was not refreshable before metadata refresh: ${JSON.stringify(before)}.`);
  assert(before.chapters.length > 0 && before.progress,
    `The metadata refresh fixture needs a saved chapter catalog and progress: ${JSON.stringify(before)}.`);
  const beforePrivatePath = join(dataDir, "private-data", "books", `${bookId}.json`);
  const bookDiskPath = join(dataDir, "books", bookId, "book.json");
  const privateBefore = JSON.parse(await readFile(beforePrivatePath, "utf8"));
  const cacheBefore = [];
  for (const chapter of before.chapters.filter((entry) => entry.src)) {
    const cachePath = join(dataDir, "books", bookId, "chapters", `${chapter.id}.html`);
    const cacheBytes = await readFile(cachePath);
    const response = await fetch(chapter.src);
    assert(response.ok, `Cached chapter '${chapter.title}' returned HTTP ${response.status} before metadata refresh.`);
    const resourceBytes = Buffer.from(await response.arrayBuffer());
    assert(createHash("sha256").update(resourceBytes).digest("hex") === createHash("sha256").update(cacheBytes).digest("hex"),
      `Cached chapter '${chapter.title}' resource did not match its persisted HTML before metadata refresh.`);
    cacheBefore.push({
      id: chapter.id,
      title: chapter.title,
      cachePath,
      sha256: createHash("sha256").update(cacheBytes).digest("hex"),
    });
  }
  assert(cacheBefore.length > 0, "The metadata refresh test requires at least one already cached chapter.");
  const bookJsonBefore = await readFile(bookDiskPath, "utf8");
  const progressBefore = JSON.stringify(before.progress);
  const chaptersBefore = JSON.stringify(before.chapters);

  contentFixture.setReplacementBookInfoMode("refreshed");
  const successCount = await completedCommandCount("refresh_book_info");
  const successEventStart = await harnessEventCount();
  await refreshButton.click();
  const successCall = await waitForCompletedCommand("refresh_book_info", successCount, 30_000);
  assert(successCall.args.bookId === bookId && successCall.result.book?.src && successCall.result.shelf?.src,
    `The UI did not invoke Rust refresh_book_info or receive both processed resources: ${JSON.stringify(successCall)}.`);
  const [refreshed, refreshedShelf] = await Promise.all([
    readResourceJson(successCall.result.book),
    readResourceJson(successCall.result.shelf),
  ]);
  const bookUpdatedEvent = await waitForHarnessEvent("resource-updated", "book", successEventStart, 10_000);
  const shelfUpdatedEvent = await waitForHarnessEvent("shelf-updated", null, successEventStart, 10_000);
  assert(bookUpdatedEvent.payload?.resource?.resourceId === successCall.result.book.resourceId &&
    shelfUpdatedEvent.payload?.resourceId === successCall.result.shelf.resourceId,
  "The refresh command did not emit its processed book and shelf resource updates.");
  assert(refreshed.title === "Browser E2E Novel" && refreshed.author === "Refreshed Fixture Author" &&
    refreshed.intro === "Metadata refreshed from the real KMP book-info operation." &&
    refreshed.kind === "Historical Mystery" && refreshed.wordCount === "98.8万字",
  `Rust did not project refreshed KMP details while preserving the source's rename policy: ${JSON.stringify(refreshed)}.`);
  const shelfBook = refreshedShelf.books?.find((entry) => entry.id === bookId);
  assert(shelfBook?.title === refreshed.title && shelfBook.author === refreshed.author,
    `The shelf resource did not receive its refreshed title and author projection: ${JSON.stringify(shelfBook)}.`);
  assert(JSON.stringify(refreshed.progress) === progressBefore && JSON.stringify(refreshed.chapters) === chaptersBefore,
    `Refreshing book details changed catalog or progress: before progress=${progressBefore}, after=${JSON.stringify(refreshed.progress)}.`);
  assert(await detail.locator("h2").innerText() === refreshed.title &&
    await detail.locator(".detail-author").innerText() === refreshed.author &&
    await detail.locator(".book-detail-intro").innerText() === refreshed.intro &&
    (await detail.locator(".book-detail-facts").innerText()).includes(refreshed.kind) &&
    (await detail.locator(".book-detail-facts").innerText()).includes(refreshed.wordCount),
  "The open detail panel did not display the processed intro, kind, and word-count fields returned by Rust.");
  await page.screenshot({ path: join(outputDir, "book-info-refreshed.png"), fullPage: true });
  const privateAfterSuccess = JSON.parse(await readFile(beforePrivatePath, "utf8"));
  assert(privateAfterSuccess.book?.name === "Browser E2E Novel" &&
    privateAfterSuccess.book?.intro === refreshed.intro && privateAfterSuccess.book?.wordCount === refreshed.wordCount,
  "The refreshed, processed KMP details were not persisted in private Rust source state.");
  const bookJsonAfterSuccess = await readFile(bookDiskPath, "utf8");
  assert(bookJsonAfterSuccess !== bookJsonBefore,
    "The public book JSON was not updated when refreshed metadata changed.");
  for (const cached of cacheBefore) {
    const cacheBytes = await readFile(cached.cachePath);
    assert(createHash("sha256").update(cacheBytes).digest("hex") === cached.sha256,
      `Metadata refresh rewrote cached chapter '${cached.title}'.`);
    const chapter = refreshed.chapters.find((entry) => entry.id === cached.id);
    assert(chapter?.src, `Metadata refresh dropped the cached chapter URL for '${cached.title}'.`);
    const response = await fetch(chapter.src);
    assert(response.ok && createHash("sha256").update(Buffer.from(await response.arrayBuffer())).digest("hex") === cached.sha256,
      `The cached chapter resource changed after refreshing book metadata for '${cached.title}'.`);
  }

  const detailsAfterSuccess = {
    title: refreshed.title,
    author: refreshed.author,
    intro: refreshed.intro,
    kind: refreshed.kind,
    wordCount: refreshed.wordCount,
  };
  contentFixture.setReplacementBookInfoMode("failure");
  const failureCount = await completedCommandCount("refresh_book_info");
  const failureEventStart = await harnessEventCount();
  await page.getByTestId("book-refresh-info").click();
  await page.waitForFunction(() => {
    const button = document.querySelector('[data-testid="book-refresh-info"]');
    return button && !button.disabled && button.textContent.includes("刷新书籍信息");
  }, undefined, { timeout: 20_000 });
  const failedCall = (await commandLog()).filter((call) => call.command === "refresh_book_info")[failureCount];
  assert(failedCall?.error && failedCall.args.bookId === bookId,
    `The failed metadata request did not return a Rust command error: ${JSON.stringify(failedCall)}.`);
  await page.locator(".toast-message.error").filter({ hasText: "刷新书籍信息失败" }).waitFor({ timeout: 10_000 });
  const failureEvents = await page.evaluate((offset) => window.__LEGADO_BROWSER_HARNESS__.events.slice(offset)
    .filter((event) => event.event === "resource-updated" && event.payload?.kind === "book" || event.event === "shelf-updated"),
  failureEventStart);
  assert(failureEvents.length === 0,
    `A failed metadata refresh emitted success resource events: ${JSON.stringify(failureEvents)}.`);
  const afterFailure = (await getBookDocument(bookId)).document;
  const afterFailureMetadata = {
    title: afterFailure.title,
    author: afterFailure.author,
    intro: afterFailure.intro,
    kind: afterFailure.kind,
    wordCount: afterFailure.wordCount,
  };
  assert(JSON.stringify(afterFailureMetadata) === JSON.stringify(detailsAfterSuccess),
    `A failed refresh changed persisted book metadata: ${JSON.stringify(afterFailureMetadata)}.`);
  assert(await detail.locator("h2").innerText() === detailsAfterSuccess.title &&
    await detail.locator(".detail-author").innerText() === detailsAfterSuccess.author &&
    await detail.locator(".book-detail-intro").innerText() === detailsAfterSuccess.intro,
  "A failed refresh replaced the details currently displayed in the open panel.");
  assert(JSON.stringify(afterFailure.progress) === progressBefore && JSON.stringify(afterFailure.chapters) === chaptersBefore,
    "A failed refresh changed chapter catalog or reading progress.");
  const privateAfterFailure = JSON.parse(await readFile(beforePrivatePath, "utf8"));
  assert(JSON.stringify(privateAfterFailure.book) === JSON.stringify(privateAfterSuccess.book),
    "A failed refresh changed the previously persisted private source metadata.");
  assert(await readFile(bookDiskPath, "utf8") === bookJsonAfterSuccess,
    "A failed refresh changed the previously persisted public book details.");
  for (const cached of cacheBefore) {
    assert(createHash("sha256").update(await readFile(cached.cachePath)).digest("hex") === cached.sha256,
      `A failed metadata refresh changed cached chapter '${cached.title}'.`);
  }
  await page.screenshot({ path: join(outputDir, "book-info-failure-preserved.png"), fullPage: true });

  const restartCommandCalls = await commandLog();
  await page.goto("about:blank");
  await restartRustHarness();
  await page.goto(origin, { waitUntil: "domcontentloaded" });
  await page.getByTestId("nav-shelf-mobile").waitFor();
  const bootstrap = await waitForCompletedCommand("app_bootstrap", 0);
  const shelfAfterRestart = await readResourceJson(bootstrap.result.shelf);
  const restartedShelfBook = shelfAfterRestart.books?.find((entry) => entry.id === bookId);
  assert(restartedShelfBook?.title === detailsAfterSuccess.title && restartedShelfBook.author === detailsAfterSuccess.author &&
    restartedShelfBook.chapterCount === before.chapters.length,
  `The refreshed shelf projection or catalog count did not survive the Rust service restart: ${JSON.stringify(restartedShelfBook)}.`);
  await page.getByTestId("nav-shelf-mobile").click();
  await page.getByTestId(`shelf-book-${bookId}`).locator("button.cover-button").click();
  await detail.waitFor();
  const restarted = (await getBookDocument(bookId)).document;
  assert(JSON.stringify({
    title: restarted.title,
    author: restarted.author,
    intro: restarted.intro,
    kind: restarted.kind,
    wordCount: restarted.wordCount,
  }) === JSON.stringify(detailsAfterSuccess),
  `Processed per-book metadata did not survive the Rust service restart: ${JSON.stringify(restarted)}.`);
  assert(JSON.stringify(restarted.progress) === progressBefore &&
    JSON.stringify(restarted.chapters.map(({ id, title, index, src }) => ({ id, title, index, cached: Boolean(src) }))) ===
      JSON.stringify(before.chapters.map(({ id, title, index, src }) => ({ id, title, index, cached: Boolean(src) }))),
  "Chapter identities, cached state, or reading progress changed after refresh and Rust restart.");
  for (const cached of cacheBefore) {
    const chapter = restarted.chapters.find((entry) => entry.id === cached.id);
    assert(chapter?.src && (await fetch(chapter.src)).ok,
      `The cached chapter '${cached.title}' did not remain readable after the Rust restart.`);
    assert(createHash("sha256").update(await readFile(cached.cachePath)).digest("hex") === cached.sha256,
      `Restarting Rust after metadata refresh changed cached chapter '${cached.title}'.`);
  }
  assert(await detail.locator("h2").innerText() === detailsAfterSuccess.title &&
    await detail.locator(".book-detail-intro").innerText() === detailsAfterSuccess.intro,
  "The detail panel did not restore refreshed metadata after Rust service restart.");
  await page.screenshot({ path: join(outputDir, "book-info-after-restart.png"), fullPage: true });

  const fixtureModes = contentFixture.replacementBookInfoRequests().map((request) => request.mode);
  assert(fixtureModes.includes("refreshed") && fixtureModes.includes("failure"),
    `Real KMP book-info did not fetch both refreshed and failing fixture variants: ${JSON.stringify(fixtureModes)}.`);
  verificationEvidence.bookInfoRefresh = {
    bookId,
    command: "refresh_book_info",
    requestModes: fixtureModes,
    refreshed: detailsAfterSuccess,
    failedRefreshPreservedDetails: true,
    chapterCount: before.chapters.length,
    cachedChapters: cacheBefore.map(({ id, title, sha256 }) => ({ id, title, sha256 })),
    progressPreserved: true,
    metadataAndCachesSurvivedRustRestart: true,
    commands: (await commandLog()).map((call) => call.command),
    previousSessionCommands: restartCommandCalls.map((call) => call.command),
  };
  report("PASS: real refresh_book_info fetched updated metadata through KMP, changed only public/private book details, and preserved cached HTML/catalog/progress.");
  report("PASS: an upstream refresh failure preserved the previously displayed and persisted details; successful metadata, cache resources, and progress survived Rust service restart.");
  diagnostics.assertClean("Book metadata refresh");
  await context.close();
}

async function main() {
  assert(process.platform === "linux", "The browser harness currently targets the configured Linux Chromium environment.");
  await requireFile(chromiumPath, "Chromium", constants.X_OK);
  await requireFile(playwrightPath, "playwright-core");
  await mkdir(outputDir, { recursive: true });
  const contentFixture = await startFixtureServer();
  const { chromium } = await import(pathToFileURL(playwrightPath).href);
  browser = await chromium.launch({
    headless: true,
    executablePath: chromiumPath,
    args: ["--no-sandbox", "--disable-dev-shm-usage", "--disable-features=LocalNetworkAccessChecks"],
  });
  await startRustHarness();
  await verifyApplicationStartupProcessLock();
  const fixtureSources = await importFixtureSource();
  await startVite();
  const readerFixture = await testReaderFlow(fixtureSources.book, contentFixture);
  await testHomeRssFlow(fixtureSources, contentFixture);
  await testHomeConfigEditorFlow();
  await testTxtTocRulesAndImportFlow();
  await testSearchHistoryFlow();
  const replacementSources = await importReplacementFixtureSources();
  await testBookSourceChangeFlow(readerFixture.bookId, replacementSources, contentFixture);
  await testBookMetadataRefreshFlow(readerFixture.bookId, contentFixture);
}

async function cleanup() {
  releasePendingFixtureResponse?.(503);
  if (page) {
    await Promise.race([
      page.evaluate(() => window.__LEGADO_BROWSER_HARNESS__?.stop()).catch(() => {}),
      delay(1500),
    ]);
  }
  if (browser) {
    await Promise.race([browser.close().catch(() => {}), delay(5000)]);
  }
  for (const child of [viteProcess, ...rustHarnessProcesses]) {
    if (!child) continue;
    await stopProcessGroup(child);
    rustHarnessProcesses.delete(child);
  }
  rustProcess = null;
  if (fixtureServer) {
    fixtureServer.closeAllConnections();
    await Promise.race([new Promise((resolvePromise) => fixtureServer.close(resolvePromise)), delay(1000)]);
  }
  if (dataDir) await rm(dataDir, { recursive: true, force: true });
}

async function stopProcessGroup(child) {
  if (!child?.pid) return;
  const groupId = child.pid;
  signalProcessGroup(child, "SIGTERM");
  await waitForChildExit(child, 2500);
  if (processGroupExists(groupId)) {
    await delay(300);
    if (processGroupExists(groupId)) {
      try {
        process.kill(-groupId, "SIGKILL");
      } catch {
        // The process group exited while the timeout was pending.
      }
    }
    await waitForChildExit(child, 1000);
  }
}

function processGroupExists(groupId) {
  if (process.platform === "win32") return false;
  try {
    process.kill(-groupId, 0);
    return true;
  } catch (error) {
    return error?.code === "EPERM";
  }
}

function signalProcessGroup(child, signal) {
  if (process.platform !== "win32" && child.pid) {
    try {
      process.kill(-child.pid, signal);
      return;
    } catch {
      // The group may have exited while its parent process is still closing stdio.
    }
  }
  try {
    child.kill(signal);
  } catch {
    // Already exited.
  }
}

async function waitForChildExit(child, timeoutMs) {
  if (child.exitCode !== null || child.signalCode !== null) return true;
  return new Promise((resolvePromise) => {
    const finish = (exited) => {
      clearTimeout(timeout);
      child.off("close", onClose);
      resolvePromise(exited);
    };
    const onClose = () => finish(true);
    const timeout = setTimeout(() => finish(false), timeoutMs);
    child.once("close", onClose);
  });
}

async function saveRunArtifacts() {
  await mkdir(outputDir, { recursive: true });
  let screenshots = [];
  try {
    screenshots = (await readdir(outputDir)).filter((file) => file.endsWith(".png")).sort();
  } catch {
    // The report still records a startup failure when no screenshot directory exists.
  }
  const reportDocument = {
    status: runStatus,
    startedAt: runStartedAt,
    finishedAt: new Date().toISOString(),
    command: process.env.BROWSER_HARNESS_BINARY
      ? `source /workspace/.setup/activate.sh && BROWSER_HARNESS_BINARY=${process.env.BROWSER_HARNESS_BINARY} node scripts/e2e-browser.mjs`
      : "source /workspace/.setup/activate.sh && node scripts/e2e-browser.mjs",
    frontend: builtFrontendDist
      ? { mode: "vite-preview", dist: builtFrontendDist }
      : { mode: "vite-dev" },
    screenshots: screenshots.map((file) => join(outputDir, file)),
    layoutChecks: [...layoutChecks],
    verificationEvidence: { ...verificationEvidence },
    failure: runFailure,
  };
  await writeFile(join(outputDir, "run.log"), `${runMessages.join("\n")}\n`, "utf8");
  await writeFile(join(outputDir, "result.json"), `${JSON.stringify(reportDocument, null, 2)}\n`, "utf8");
}

async function saveFailureDiagnostics(error) {
  if (!page || page.isClosed()) return;
  await mkdir(outputDir, { recursive: true });
  const diagnostics = {
    error: error instanceof Error ? error.stack || error.message : String(error),
    pageUrl: page.url(),
    title: await page.title().catch(() => ""),
    bodyText: await page.locator("body").innerText({ timeout: 3000 }).catch(() => "<body text unavailable>"),
    appInvocations: await page.evaluate(() => window.__LEGADO_BROWSER_HARNESS__?.calls ?? []).catch(() => []),
    appEvents: await page.evaluate(() => window.__LEGADO_BROWSER_HARNESS__?.events ?? []).catch(() => []),
    browserErrors: [...errors],
    browserConsoleErrors: [...browserConsoleErrors],
    browserRequestFailures: [...browserRequestFailures],
    sourceRequests: [...sourceRequests],
    resourceRequests: [...resourceRequests],
    rustHarnessTail: rustProcess?.tail ?? [],
    viteTail: viteProcess?.tail ?? [],
  };
  await page.screenshot({ path: join(outputDir, "failure.png"), fullPage: true, timeout: 10_000 }).catch(() => {});
  await page.locator("html").evaluate((element) => element.outerHTML).then(
    (html) => writeFile(join(outputDir, "failure-dom.html"), html, "utf8"),
  ).catch(() => {});
  await writeFile(join(outputDir, "failure-body.txt"), diagnostics.bodyText, "utf8");
  await writeFile(join(outputDir, "failure-diagnostics.json"), `${JSON.stringify(diagnostics, null, 2)}\n`, "utf8");
}

async function run() {
  try {
    await main();
    runStatus = "PASS";
  } catch (error) {
    runStatus = "FAIL";
    runFailure = error instanceof Error ? error.stack || error.message : String(error);
    runMessages.push(runFailure);
    console.error(runFailure);
    await saveFailureDiagnostics(error).catch((diagnosticError) => {
      const message = `Could not save browser failure diagnostics: ${diagnosticError}`;
      runMessages.push(message);
      console.error(message);
    });
    if (rustProcess?.tail?.length) {
      const output = "Rust harness output:\n" + rustProcess.tail.join("");
      runMessages.push(output);
      console.error(output);
    }
    if (viteProcess?.tail?.length) {
      const output = "Vite output:\n" + viteProcess.tail.join("");
      runMessages.push(output);
      console.error(output);
    }
    process.exitCode = 1;
  } finally {
    await cleanup();
    await saveRunArtifacts();
  }
}

void run();
