<script setup lang="ts">
import { computed, ref } from "vue";
import {
  executeSourceEngine,
  type SourceEngineOperation,
  type SourceEngineRequest,
} from "./api/sourceEngine";

type JsonObject = Record<string, unknown>;

const sourceText = ref("");
const keyword = ref("");
const page = ref(1);
const loadingOperation = ref<SourceEngineOperation | null>(null);
const errorMessage = ref("");
const searchBooks = ref<JsonObject[]>([]);
const selectedBook = ref<JsonObject | null>(null);
const bookInfo = ref<JsonObject | null>(null);
const chapters = ref<JsonObject[]>([]);
const selectedChapter = ref<JsonObject | null>(null);
const chapterContent = ref("");
const lastResult = ref<unknown>(null);

const canUseBook = computed(() => Boolean(selectedBook.value) && !loadingOperation.value);
const canUseChapter = computed(() => canUseBook.value && Boolean(selectedChapter.value));

function parseSource(): JsonObject {
  let parsed: unknown;
  try {
    parsed = JSON.parse(sourceText.value);
  } catch (error) {
    throw new Error(`书源 JSON 格式错误：${error instanceof Error ? error.message : String(error)}`);
  }
  if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
    throw new Error("书源 JSON 顶层必须是对象");
  }
  return parsed as JsonObject;
}

function textField(value: JsonObject | null, key: string, fallback: string): string {
  const field = value?.[key];
  return typeof field === "string" && field.length > 0 ? field : fallback;
}

function bookLabel(book: JsonObject): string {
  const name = textField(book, "name", textField(book, "bookName", "未命名书籍"));
  const author = textField(book, "author", "作者未知");
  return `${name} · ${author}`;
}

function chapterLabel(chapter: JsonObject): string {
  return textField(chapter, "title", textField(chapter, "name", "未命名章节"));
}

async function runOperation(
  operation: SourceEngineOperation,
  extra: Partial<SourceEngineRequest> = {},
): Promise<unknown | null> {
  if (loadingOperation.value) return null;
  errorMessage.value = "";
  loadingOperation.value = operation;
  try {
    const request: SourceEngineRequest = {
      operation,
      source: parseSource(),
      page: page.value,
      ...extra,
    };
    const result = await executeSourceEngine(request);
    lastResult.value = result;
    return result;
  } catch (error) {
    errorMessage.value = error instanceof Error ? error.message : String(error);
    lastResult.value = { error: errorMessage.value };
    return null;
  } finally {
    loadingOperation.value = null;
  }
}

async function search(): Promise<void> {
  const result = await runOperation("search", { keyword: keyword.value });
  if (!result || typeof result !== "object") return;
  const books = (result as { books?: unknown }).books;
  searchBooks.value = Array.isArray(books)
    ? books.filter((book): book is JsonObject => Boolean(book) && typeof book === "object")
    : [];
  selectedBook.value = null;
  bookInfo.value = null;
  chapters.value = [];
  selectedChapter.value = null;
  chapterContent.value = "";
}

function selectBook(book: JsonObject): void {
  selectedBook.value = book;
  bookInfo.value = null;
  chapters.value = [];
  selectedChapter.value = null;
  chapterContent.value = "";
}

async function loadBookInfo(): Promise<void> {
  if (!selectedBook.value) return;
  const result = await runOperation("bookInfo", { book: selectedBook.value });
  if (result && typeof result === "object" && !Array.isArray(result)) {
    bookInfo.value = result as JsonObject;
    selectedBook.value = bookInfo.value;
  }
}

async function loadChapters(): Promise<void> {
  if (!selectedBook.value) return;
  const result = await runOperation("chapters", { book: selectedBook.value });
  if (!Array.isArray(result)) return;
  chapters.value = result.filter(
    (chapter): chapter is JsonObject => Boolean(chapter) && typeof chapter === "object",
  );
  selectedChapter.value = chapters.value[0] ?? null;
  chapterContent.value = "";
}

async function loadContent(): Promise<void> {
  if (!selectedBook.value || !selectedChapter.value) return;
  const chapterIndex = chapters.value.indexOf(selectedChapter.value);
  const nextChapterUrl =
    chapterIndex >= 0 && chapterIndex + 1 < chapters.value.length
      ? textField(chapters.value[chapterIndex + 1], "url", "")
      : undefined;
  const result = await runOperation("content", {
    book: selectedBook.value,
    chapter: selectedChapter.value,
    nextChapterUrl,
  });
  if (typeof result === "string") chapterContent.value = result;
}

function formatJson(value: unknown): string {
  return value === null ? "" : JSON.stringify(value, null, 2);
}
</script>

<template>
  <main class="page-shell">
    <header class="topbar">
      <div>
        <p class="eyebrow">LEGADO · SOURCE ENGINE</p>
        <h1>书源测试</h1>
        <p class="subtitle">粘贴书源 JSON，按搜索、详情、目录、正文的顺序验证规则。</p>
      </div>
      <span class="engine-badge"><i></i> Rust Host · KMP Engine</span>
    </header>

    <section class="workspace">
      <section class="panel source-panel">
        <div class="panel-heading">
          <div>
            <h2>书源 JSON</h2>
            <p>粘贴完整书源对象</p>
          </div>
          <span class="step-number">01</span>
        </div>
        <textarea
          v-model="sourceText"
          class="source-input"
          spellcheck="false"
          placeholder="在这里粘贴书源 JSON…"
          aria-label="书源 JSON"
        ></textarea>
        <div class="source-note">
          规则与网络请求由 Rust 调用 KMP 执行；本页面不在 WebView 中解析规则。
        </div>
      </section>

      <section class="panel flow-panel">
        <div class="panel-heading">
          <div>
            <h2>测试流程</h2>
            <p>每一步的结果会带入下一步</p>
          </div>
          <span class="step-number">02</span>
        </div>

        <div class="search-controls">
          <label class="field-label" for="keyword">搜索关键词</label>
          <div class="search-row">
            <input
              id="keyword"
              v-model="keyword"
              type="text"
              placeholder="输入书名"
              @keydown.enter="search"
            />
            <input
              v-model.number="page"
              class="page-input"
              type="number"
              min="1"
              aria-label="搜索页码"
              title="搜索页码"
            />
            <button class="primary-button" :disabled="Boolean(loadingOperation)" @click="search">
              {{ loadingOperation === "search" ? "搜索中…" : "搜索" }}
            </button>
          </div>
        </div>

        <div v-if="errorMessage" class="error-box" role="alert">{{ errorMessage }}</div>

        <div class="result-section">
          <div class="section-title"><span>搜索结果</span><small>{{ searchBooks.length }} 本</small></div>
          <div v-if="searchBooks.length" class="result-list">
            <button
              v-for="(book, index) in searchBooks"
              :key="String(book.bookUrl ?? index)"
              class="result-item"
              :class="{ selected: selectedBook === book }"
              @click="selectBook(book)"
            >
              <span class="item-index">{{ String(index + 1).padStart(2, "0") }}</span>
              <span class="item-label">{{ bookLabel(book) }}</span>
              <span class="item-arrow">›</span>
            </button>
          </div>
          <div v-else class="empty-hint">运行搜索后，结果会显示在这里。</div>
        </div>

        <div class="action-row">
          <button class="secondary-button" :disabled="!canUseBook" @click="loadBookInfo">
            {{ loadingOperation === "bookInfo" ? "读取中…" : "测试详情" }}
          </button>
          <button class="secondary-button" :disabled="!canUseBook" @click="loadChapters">
            {{ loadingOperation === "chapters" ? "读取中…" : "读取目录" }}
          </button>
          <button class="secondary-button" :disabled="!canUseChapter" @click="loadContent">
            {{ loadingOperation === "content" ? "读取中…" : "读取正文" }}
          </button>
        </div>

        <div v-if="selectedBook" class="selection-card">
          <div class="section-title"><span>当前书籍</span></div>
          <strong>{{ bookLabel(bookInfo ?? selectedBook) }}</strong>
          <p v-if="bookInfo">详情规则已执行，可继续读取目录。</p>
          <p v-else>可直接读取目录，或先测试详情规则。</p>
        </div>

        <div v-if="chapters.length" class="result-section chapter-section">
          <div class="section-title"><span>章节目录</span><small>{{ chapters.length }} 章</small></div>
          <select v-model="selectedChapter" aria-label="选择要读取的章节">
            <option v-for="(chapter, index) in chapters" :key="String(chapter.url ?? index)" :value="chapter">
              {{ String(index + 1).padStart(2, "0") }} · {{ chapterLabel(chapter) }}
            </option>
          </select>
        </div>
      </section>
    </section>

    <section class="panel output-panel">
      <div class="panel-heading output-heading">
        <div>
          <h2>执行结果</h2>
          <p>正文使用阅读器预处理后的文本；原始响应 JSON 可展开查看</p>
        </div>
        <span class="step-number">03</span>
      </div>
      <div v-if="chapterContent" class="content-output">{{ chapterContent }}</div>
      <div v-else-if="lastResult !== null && !errorMessage" class="json-output">
        <pre>{{ formatJson(lastResult) }}</pre>
      </div>
      <div v-else class="empty-output">执行搜索或选择操作后，返回数据会显示在这里。</div>
      <details v-if="chapterContent" class="raw-result">
        <summary>查看正文返回值 JSON</summary>
        <pre>{{ formatJson(lastResult) }}</pre>
      </details>
    </section>
  </main>
</template>

<style>
:root {
  font-family: Inter, "Segoe UI", "Microsoft YaHei", sans-serif;
  color: #202d35;
  background: #f2f5f4;
  font-synthesis: none;
  text-rendering: optimizeLegibility;
  -webkit-font-smoothing: antialiased;
  -moz-osx-font-smoothing: grayscale;
  font-optical-sizing: auto;
}

* { box-sizing: border-box; }
body { min-width: 720px; min-height: 100vh; margin: 0; }
button, input, textarea, select { font: inherit; }
button { cursor: pointer; }
button:disabled { cursor: not-allowed; opacity: .48; }

.page-shell { max-width: 1440px; margin: 0 auto; padding: 30px 34px 40px; }
.topbar { display: flex; justify-content: space-between; align-items: center; gap: 24px; margin: 0 0 23px; }
.eyebrow { margin: 0 0 8px; color: #488d7c; font-size: 10px; font-weight: 750; letter-spacing: .18em; }
h1 { margin: 0; color: #1d2b31; font-size: 28px; letter-spacing: -.035em; }
.subtitle { margin: 7px 0 0; color: #7b898b; font-size: 13px; }
.engine-badge { display: inline-flex; align-items: center; gap: 9px; padding: 9px 13px; border: 1px solid #dce8e3; border-radius: 20px; color: #507568; background: #f9fcfa; font-size: 11px; font-weight: 650; white-space: nowrap; }
.engine-badge i { width: 7px; height: 7px; border-radius: 50%; background: #48ad81; box-shadow: 0 0 0 3px #e3f3eb; }
.workspace { display: grid; grid-template-columns: minmax(330px, .92fr) minmax(420px, 1.08fr); align-items: stretch; gap: 17px; }
.panel { min-width: 0; border: 1px solid #e2e9e6; border-radius: 14px; background: #fff; box-shadow: 0 5px 18px #27413708; }
.source-panel, .flow-panel { display: flex; flex-direction: column; padding: 20px; }
.panel-heading { display: flex; justify-content: space-between; align-items: flex-start; gap: 14px; margin-bottom: 16px; }
.panel-heading h2 { margin: 0; color: #26353a; font-size: 15px; font-weight: 700; }
.panel-heading p { margin: 5px 0 0; color: #99a5a3; font-size: 11px; }
.step-number { color: #a7b8b0; font-size: 11px; font-weight: 750; letter-spacing: .1em; }
.source-input { width: 100%; min-height: 430px; flex: 1; resize: vertical; padding: 14px; border: 1px solid #e5ebe8; border-radius: 9px; outline: none; background: #fbfcfb; color: #42524f; font: 11px/1.65 "Cascadia Code", Consolas, monospace; tab-size: 2; }
.source-input:focus, input:focus, select:focus { border-color: #78b49a; box-shadow: 0 0 0 3px #57a78015; }
.source-input::placeholder { color: #b2bfba; }
.source-note { margin-top: 11px; color: #9ba7a3; font-size: 10px; line-height: 1.6; }
.search-controls { margin: 0 0 17px; }
.field-label { display: block; margin-bottom: 7px; color: #60716c; font-size: 11px; font-weight: 650; }
.search-row { display: flex; gap: 8px; }
input, select { min-width: 0; height: 37px; padding: 0 10px; border: 1px solid #e5ebe8; border-radius: 8px; outline: none; background: #fff; color: #34443f; font-size: 12px; }
.search-row > input:first-child { flex: 1; }
.page-input { width: 66px; }
.primary-button, .secondary-button { border: 0; border-radius: 8px; font-size: 11px; font-weight: 700; transition: background .15s ease, transform .15s ease; }
.primary-button { min-width: 75px; padding: 0 13px; color: #fff; background: #397e68; }
.primary-button:hover:not(:disabled) { background: #2f6d5a; }
.secondary-button { min-height: 35px; padding: 0 12px; color: #427561; background: #edf5f0; }
.secondary-button:hover:not(:disabled) { background: #e0eee6; }
.result-section { min-width: 0; }
.section-title { display: flex; justify-content: space-between; align-items: center; margin-bottom: 8px; color: #65756e; font-size: 11px; font-weight: 700; }
.section-title small { color: #a2aeaa; font-size: 10px; font-weight: 500; }
.result-list { max-height: 198px; overflow-y: auto; border: 1px solid #edf0ee; border-radius: 8px; }
.result-item { display: flex; width: 100%; min-height: 38px; align-items: center; gap: 10px; padding: 7px 10px; border: 0; border-bottom: 1px solid #f0f3f1; background: #fff; color: #51615b; text-align: left; }
.result-item:last-child { border-bottom: 0; }
.result-item:hover, .result-item.selected { background: #f1f8f4; }
.item-index { color: #a0b4a9; font: 10px "Cascadia Code", Consolas, monospace; }
.item-label { min-width: 0; flex: 1; overflow: hidden; font-size: 11px; text-overflow: ellipsis; white-space: nowrap; }
.item-arrow { color: #9ab6a8; font-size: 18px; line-height: 1; }
.empty-hint { display: grid; min-height: 75px; place-items: center; border: 1px dashed #e6ece8; border-radius: 8px; color: #aab5b0; font-size: 11px; }
.action-row { display: flex; flex-wrap: wrap; gap: 7px; margin-top: 13px; }
.selection-card { margin-top: 14px; padding: 11px 12px; border-radius: 8px; background: #f7faf8; }
.selection-card strong { display: block; overflow: hidden; color: #41534b; font-size: 11px; text-overflow: ellipsis; white-space: nowrap; }
.selection-card p { margin: 6px 0 0; color: #9aa6a1; font-size: 10px; }
.chapter-section { margin-top: 14px; }
.chapter-section select { width: 100%; }
.error-box { margin: -5px 0 13px; padding: 9px 11px; border: 1px solid #f1d5d1; border-radius: 8px; background: #fff7f5; color: #ac5143; font-size: 11px; line-height: 1.5; overflow-wrap: anywhere; }
.output-panel { margin-top: 17px; padding: 17px 20px 19px; }
.output-heading { margin-bottom: 10px; }
.content-output, .json-output, .empty-output { min-height: 82px; max-height: 250px; overflow: auto; border: 1px solid #edf0ee; border-radius: 8px; background: #fbfcfb; }
.content-output { padding: 13px 14px; color: #40514a; font: 12px/1.8 "Microsoft YaHei", sans-serif; white-space: pre-wrap; overflow-wrap: anywhere; }
.json-output pre, .raw-result pre { margin: 0; padding: 12px 14px; color: #586762; font: 10px/1.6 "Cascadia Code", Consolas, monospace; white-space: pre-wrap; overflow-wrap: anywhere; }
.empty-output { display: grid; place-items: center; color: #aab5b0; font-size: 11px; }
.raw-result { margin-top: 9px; color: #7b8a83; font-size: 10px; }
.raw-result summary { cursor: pointer; }
.raw-result pre { max-height: 170px; margin-top: 6px; overflow: auto; border: 1px solid #edf0ee; border-radius: 8px; background: #fbfcfb; }

@media (max-width: 900px) {
  .page-shell { padding: 22px 20px 30px; }
  .workspace { grid-template-columns: 1fr; }
  .source-input { min-height: 280px; }
  .topbar { align-items: flex-start; }
  .engine-badge { margin-top: 5px; }
}
</style>
