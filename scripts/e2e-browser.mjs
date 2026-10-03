#!/usr/bin/env node

import { createServer } from "node:http";
import { spawn, spawnSync } from "node:child_process";
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
const outputDir = resolve(process.env.PLAYWRIGHT_OUTPUT_DIR || "/tmp/legado-browser-e2e-results");
const origin = "http://127.0.0.1:1420";
const errors = [];
const sourceRequests = [];
const resourceRequests = [];
const browserRequestFailures = [];
const runMessages = [];
const runStartedAt = new Date().toISOString();
let runStatus = "NOT_RUN";
let runFailure;
let fixtureServer;
let fixtureOrigin;
let rustProcess;
let viteProcess;
let browser;
let browserHarnessUrl;
let resourceServerUrl;
let dataDir;
let page;

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
    sourceRequests.push({ method: request.method, url: request.url, body });
    const pathname = new URL(request.url || "/", fixtureOrigin || "http://127.0.0.1").pathname;
    const html = (value) => {
      response.writeHead(200, { "content-type": "text/html; charset=utf-8" });
      response.end(value);
    };
    if (pathname === "/search") {
      html("<div class='item'><h3><a href='/book'>Browser E2E Novel</a></h3><span class='author'>Fixture Author</span></div>");
    } else if (pathname === "/book") {
      html("<h1>Browser E2E Novel</h1><span class='author'>Fixture Author</span><a class='toc' href='/toc'>目录</a>");
    } else if (pathname === "/toc") {
      html("<ul id='list'><li><a href='/chapter/1'>Fixture Chapter One</a></li><li><a href='/chapter/2'>Fixture Chapter Two</a></li></ul>");
    } else if (pathname === "/chapter/1") {
      html(`<div class='content'>${fullText}</div>`);
    } else if (pathname === "/chapter/2") {
      html(`<div class='content'>第二章标记：${fullText}</div>`);
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
  return { paragraphs, fullText };
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
  dataDir = await mkdtemp(join(tmpdir(), "legado-browser-harness-"));
  const rustEnv = {
    ...process.env,
    LEGADO_BROWSER_HARNESS_DATA: dataDir,
    ANDROID_USER_HOME: process.env.ANDROID_USER_HOME || "/workspace/.setup/android-user",
  };
  rustProcess = startProcess(
    "cargo",
    ["run", "--manifest-path", "src-tauri/Cargo.toml", "--no-default-features", "--example", "browser_harness"],
    { env: rustEnv },
  );
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

async function startVite() {
  viteProcess = startProcess("npm", ["run", "dev", "--", "--host", "127.0.0.1"]);
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
  return result.sources[0];
}

async function installInvokeBridge(context) {
  await context.addInitScript((harnessUrl) => {
    if (window.top !== window) return;
    const callbacks = new Map();
    const listeners = new Map();
    const calls = [];
    let nextCallback = 1;
    let eventCursor = 0;
    let stop = false;

    const dispatch = (record) => {
      for (const [listenerId, listener] of listeners) {
        if (listener.event !== record.event) continue;
        const callback = callbacks.get(String(listener.callbackId));
        if (callback) callback({ event: record.event, id: listenerId, payload: record.payload });
      }
    };

    window.__LEGADO_BROWSER_HARNESS__ = {
      calls,
      stop() { stop = true; },
      async invoke(command, args = {}) {
        const call = { command, args, startedAt: performance.now() };
        calls.push(call);
        const response = await fetch(`${harnessUrl}/invoke`, {
          method: "POST",
          headers: { "content-type": "application/json" },
          body: JSON.stringify({ command, args }),
        });
        const result = await response.json();
        if (!response.ok || result.ok === false) throw new Error(result.error || `Rust command failed: ${command}`);
        call.result = result.value;
        call.finishedAt = performance.now();
        if (command === "plugin:event|listen") {
          listeners.set(result.value, { event: args.event, callbackId: args.handler });
        } else if (command === "plugin:event|unlisten") {
          listeners.delete(args.eventId);
        }
        return result.value;
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
        try {
          const response = await fetch(`${harnessUrl}/events?after=${eventCursor}`);
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
  })));
}

function readPageIndicator(value) {
  const match = value.match(/(\d+)\s*\/\s*(\d+)/);
  assert(Boolean(match), `Cannot parse reader page indicator: '${value}'`);
  return { current: Number(match[1]), total: Number(match[2]) };
}

async function visibleTextOnCurrentPage() {
  return page.getByTestId("reader-frame").locator("body").evaluate((body) => {
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
  return page.getByTestId("reader-frame").locator("body").evaluate((body) => getComputedStyle(body).fontSize);
}

async function getBookDocument(bookId) {
  const descriptor = await fetchJson(`${browserHarnessUrl}/invoke`, { command: "get_book", args: { bookId } });
  const document = await (await fetch(descriptor.src)).json();
  return { descriptor, document };
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
  await installInvokeBridge(context);
  page = await context.newPage();
  page.on("pageerror", (error) => errors.push(`pageerror: ${error.message}`));
  page.on("console", (message) => {
    if (message.type() === "error") errors.push(`console: ${message.text()}`);
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
  await page.getByTestId("nav-sources-mobile").click();
  await page.getByText(sourceMetadata.name, { exact: true }).waitFor();
  await page.getByTestId("nav-search-mobile").click();
  await setViewport(360);
  await page.screenshot({ path: join(outputDir, "mobile-search-360.png"), fullPage: true });

  await page.getByTestId("search-keyword").fill("Browser E2E Novel");
  await page.getByTestId("search-submit").click();
  const resultCard = page.locator('[data-testid^="search-result-"]').first();
  await resultCard.waitFor({ timeout: 45_000 });
  assert((await resultCard.innerText()).includes("Browser E2E Novel"), "Search results did not display the parsed fixture book.");
  const resultTestId = await resultCard.getAttribute("data-testid");
  const resultId = resultTestId.replace("search-result-", "");
  await page.getByTestId(`search-add-${resultId}`).click();
  await page.getByText("Browser E2E Novel", { exact: true }).last().waitFor();
  await page.getByTestId("nav-shelf-mobile").click();
  const shelfCard = page.locator('[data-testid^="shelf-book-"]').first();
  await shelfCard.waitFor();
  const shelfTestId = await shelfCard.getAttribute("data-testid");
  const bookId = shelfTestId.replace("shelf-book-", "");
  await shelfCard.locator("button.cover-button").click();
  await page.getByRole("button", { name: /开始阅读|继续阅读/ }).click();
  await page.getByTestId("reader-frame").waitFor({ timeout: 60_000 });
  await page.getByText("Fixture Chapter One", { exact: true }).waitFor();
  await page.waitForFunction(() => {
    const calls = window.__LEGADO_BROWSER_HARNESS__.calls.filter((call) => call.command === "prepare_chapters");
    return calls.some((call) => call.args.fromIndex === 0) && calls.some((call) => call.args.fromIndex === 1);
  }, { timeout: 45_000 });
  await page.waitForFunction(() => !document.querySelector(".preload-indicator"), null, { timeout: 45_000 }).catch(() => {});

  const { document: firstBookDoc } = await getBookDocument(bookId);
  const firstChapter = firstBookDoc.chapters[0];
  const firstCacheResponse = await fetch(firstChapter.src);
  assert(firstCacheResponse.ok, `Prepared chapter resource failed with HTTP ${firstCacheResponse.status}.`);
  const firstCacheHtml = await firstCacheResponse.text();
  const firstHash = createHash("sha256").update(firstCacheHtml).digest("hex");
  assert(firstCacheHtml.includes("font-size:"), "Chapter HTML does not include default reading styles.");

  const startFontSize = Number.parseFloat(await visibleReaderFontSize());
  await page.getByRole("button", { name: "阅读显示设置" }).click();
  await page.getByTestId("reader-font-increase").click();
  await page.waitForFunction((expected) => {
    const frame = document.querySelector('[data-testid="reader-frame"]');
    return frame?.contentDocument?.body && getComputedStyle(frame.contentDocument.body).fontSize !== expected;
  }, `${startFontSize}px`);
  const increasedFontSize = Number.parseFloat(await visibleReaderFontSize());
  assert(increasedFontSize > startFontSize, `Reader font size did not increase (${startFontSize} -> ${increasedFontSize}).`);
  await page.waitForFunction(() => window.__LEGADO_BROWSER_HARNESS__.calls.some((call) => call.command === "save_settings"), { timeout: 10_000 });
  const afterFontHtml = await (await fetch(firstChapter.src)).text();
  assert(createHash("sha256").update(afterFontHtml).digest("hex") === firstHash, "Changing reader font rewrote cached chapter HTML.");

  await setViewport(360);
  await page.screenshot({ path: join(outputDir, "reader-360.png"), fullPage: true });
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
  const callsBeforeChapterTurn = await commandLog();
  const commandCountBeforeChapterTurn = callsBeforeChapterTurn.length;
  await page.getByTestId("reader-next").click();
  await page.getByText("Fixture Chapter Two", { exact: true }).waitFor();
  await page.waitForTimeout(200);
  const callsAfterChapterTurn = await commandLog();
  assert(callsAfterChapterTurn.length === commandCountBeforeChapterTurn,
    `Turning to an already cached chapter invoked Rust: ${callsAfterChapterTurn.slice(commandCountBeforeChapterTurn).map((call) => call.command).join(", ")}`);
  assert(callsAfterChapterTurn.filter((call) => call.command === "prepare_chapters").length === callsBeforeChapterTurn.filter((call) => call.command === "prepare_chapters").length,
    "Turning between prepared chapters requested redundant chapter preparation.");

  await setViewport(1440);
  await page.screenshot({ path: join(outputDir, "reader-1440.png"), fullPage: true });
  const desktopIndicator = readPageIndicator(await page.getByTestId("reader-page-indicator").innerText());
  assert(desktopIndicator.current <= desktopIndicator.total, "Reader resize left its page position outside the new page range.");
  assert((await page.getByTestId("reader-frame").locator("body").innerText()).includes("第二章标记"), "Resizing the reader lost its current cached chapter.");
  const { document: secondBookDoc } = await getBookDocument(bookId);
  const secondChapter = secondBookDoc.chapters[1];
  const secondChapterFile = join(dataDir, "books", bookId, "chapters", `${secondChapter.id}.html`);
  const secondCache = await readFile(secondChapterFile);
  const secondHash = createHash("sha256").update(secondCache).digest("hex");
  await page.getByTestId("reader-next").click();
  const beforeSecondPage = readPageIndicator(await page.getByTestId("reader-page-indicator").innerText());
  if (beforeSecondPage.total > 1) {
    await page.waitForFunction((expected) => {
      const value = document.querySelector('[data-testid="reader-page-indicator"]')?.textContent || "";
      return new RegExp(`^${expected}\\s*/`).test(value.trim());
    }, beforeSecondPage.current + 1);
  }
  const secondPageIndicator = readPageIndicator(await page.getByTestId("reader-page-indicator").innerText());
  if (secondPageIndicator.total > 1) {
    await page.getByTestId("reader-next").click();
    await page.waitForFunction((expected) => {
      const value = document.querySelector('[data-testid="reader-page-indicator"]')?.textContent || "";
      return new RegExp(`^${expected}\\s*/`).test(value.trim());
    }, secondPageIndicator.current + 1);
  }
  const savedPosition = readPageIndicator(await page.getByTestId("reader-page-indicator").innerText());
  const expectedOffset = savedPosition.current - 1;
  await page.getByRole("button", { name: "返回目录" }).click();
  await page.waitForFunction(() => window.__LEGADO_BROWSER_HARNESS__.calls.some((call) => call.command === "save_progress"), { timeout: 15_000 });
  await page.waitForFunction(() => !document.querySelector(".reader-shell"));
  const persisted = await getBookDocument(bookId);
  assert(persisted.document.progress.chapterIndex === 1, `Saved progress has the wrong chapter: ${JSON.stringify(persisted.document.progress)}.`);
  assert(persisted.document.progress.offset === expectedOffset, `Saved progress has the wrong page offset: ${JSON.stringify(persisted.document.progress)}.`);

  await rm(secondChapterFile, { force: true });
  const prepareCountBefore404 = (await commandLog()).filter((call) => call.command === "prepare_chapters").length;
  await page.locator(`[data-testid="shelf-book-${bookId}"] button.cover-button`).click();
  await page.getByRole("button", { name: "继续阅读" }).click();
  await page.getByTestId("reader-frame").waitFor({ timeout: 60_000 });
  await page.getByText("Fixture Chapter Two", { exact: true }).waitFor();
  await page.waitForFunction((count) => window.__LEGADO_BROWSER_HARNESS__.calls.filter((call) => call.command === "prepare_chapters").length > count, prepareCountBefore404, { timeout: 45_000 });
  const restored = readPageIndicator(await page.getByTestId("reader-page-indicator").innerText());
  assert(restored.current === expectedOffset + 1,
    `Saved reader page was not restored after reopening and repairing the missing chapter: expected ${expectedOffset + 1}, got ${restored.current}.`);
  await access(secondChapterFile, constants.F_OK);
  const repairedHtml = await readFile(secondChapterFile, "utf8");
  assert(repairedHtml.includes("font-size:"), "404 recovery did not recreate a browser-ready HTML chapter resource.");
  assert(createHash("sha256").update(await readFile(join(dataDir, "books", bookId, "chapters", `${firstChapter.id}.html`))).digest("hex") === firstHash,
    "Repairing a second chapter modified the first chapter cache.");

  await assertPrivateSourceIsNotPublic();
  const allCalls = await commandLog();
  const requiredCommands = ["app_bootstrap", "list_sources", "search_books", "add_book", "get_book", "prepare_chapters", "save_settings", "save_progress"];
  for (const command of requiredCommands) assert(allCalls.some((call) => call.command === command), `Browser flow did not exercise real Rust command '${command}'.`);
  assert(sourceRequests.some((request) => request.url.startsWith("/search") && request.method === "POST"), "The local book-source engine did not execute a real fixture HTTP search.");
  assert(sourceRequests.some((request) => request.url.startsWith("/book")) && sourceRequests.some((request) => request.url.startsWith("/toc")), "The source engine did not fetch book details and chapter listing from the fixture server.");
  assert(sourceRequests.some((request) => request.url.startsWith("/chapter/1")) && sourceRequests.some((request) => request.url.startsWith("/chapter/2")), "The source engine did not fetch both chapter bodies from the fixture server.");
  assert(resourceRequests.some((request) => request.url.includes("/r/") && request.url.endsWith("/shelf.json") && request.status === 200), "Browser did not fetch the shelf JSON resource.");
  assert(resourceRequests.some((request) => request.url.includes("/chapters/") && request.status === 200), "Browser did not fetch a processed chapter HTML resource.");
  assert(resourceRequests.some((request) => request.url.includes("/chapters/") && request.status === 404), "Browser did not encounter the deliberately missing chapter resource.");
  assert(errors.length === 0, `Browser reported errors:\n${errors.join("\n")}`);

  report(`PASS: real Rust service + KMP source search/read on Chromium (${allCalls.length} IPC calls).`);
  report(`PASS: ${contentFixture.paragraphs.length} mobile-page paragraphs were all visible; font ${startFontSize}px -> ${increasedFontSize}px.`);
  report("PASS: cached chapter flips invoked no Rust commands; saved chapter/page restored after reopen.");
  report("PASS: cached HTML hash stayed stable after display-only font adjustment; missing chapter URL recovered through Rust.");
  report("PASS: private source rules stayed off the browser resource server; real JSON and HTML resources were fetched.");
  report(`Screenshots: ${outputDir}`);
  await context.close();
}

async function main() {
  assert(process.platform === "linux", "The browser harness currently targets the configured Linux Chromium environment.");
  await requireFile(chromiumPath, "Chromium", constants.X_OK);
  await requireFile(playwrightPath, "playwright-core");
  await mkdir(outputDir, { recursive: true });
  const { paragraphs, fullText } = await startFixtureServer();
  const { chromium } = await import(pathToFileURL(playwrightPath).href);
  browser = await chromium.launch({
    headless: true,
    executablePath: chromiumPath,
    args: ["--no-sandbox", "--disable-dev-shm-usage"],
  });
  await startRustHarness();
  await importFixtureSource();
  await startVite();
  await testReaderFlow({ name: "Local Browser E2E" }, { paragraphs, fullText });
}

async function cleanup() {
  if (page) {
    await Promise.race([
      page.evaluate(() => window.__LEGADO_BROWSER_HARNESS__?.stop()).catch(() => {}),
      delay(1500),
    ]);
  }
  if (browser) {
    await Promise.race([browser.close().catch(() => {}), delay(5000)]);
  }
  for (const child of [viteProcess, rustProcess]) {
    if (!child || child.exitCode !== null || child.signalCode !== null) continue;
    signalProcessGroup(child, "SIGTERM");
    const exited = await waitForChildExit(child, 3000);
    if (!exited) {
      signalProcessGroup(child, "SIGKILL");
      await waitForChildExit(child, 1000);
    }
  }
  await terminateRunnerDescendants();
  if (fixtureServer) {
    fixtureServer.closeAllConnections();
    await Promise.race([new Promise((resolvePromise) => fixtureServer.close(resolvePromise)), delay(1000)]);
  }
  if (dataDir) await rm(dataDir, { recursive: true, force: true });
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

async function terminateRunnerDescendants() {
  if (process.platform === "win32") return;
  const listing = spawnSync("ps", ["-eo", "pid=,ppid="], { encoding: "utf8" });
  if (listing.status !== 0) return;
  const parents = new Map();
  for (const line of listing.stdout.split(/\r?\n/)) {
    const match = line.trim().match(/^(\d+)\s+(\d+)$/);
    if (!match) continue;
    const pid = Number(match[1]);
    const ppid = Number(match[2]);
    const children = parents.get(ppid) ?? [];
    children.push(pid);
    parents.set(ppid, children);
  }
  const descendants = [];
  const pending = [...(parents.get(process.pid) ?? [])];
  while (pending.length) {
    const pid = pending.shift();
    descendants.push(pid);
    pending.push(...(parents.get(pid) ?? []));
  }
  for (const pid of descendants.reverse()) {
    try {
      process.kill(pid, "SIGTERM");
    } catch {
      // Already exited.
    }
  }
  await delay(500);
  for (const pid of descendants.reverse()) {
    try {
      process.kill(pid, "SIGKILL");
    } catch {
      // Already exited.
    }
  }
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
    command: "source /workspace/.setup/activate.sh && node scripts/e2e-browser.mjs",
    screenshots: screenshots.map((file) => join(outputDir, file)),
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
    browserErrors: [...errors],
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
