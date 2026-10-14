<script setup lang="ts">
import { openedBookFromShelf, switchOpenedBook } from "../books/bookDetailState";
import { shelfBooks as shelf } from "./shelfBooks";
import { computed, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { notify } from "../../app/notifications";
import { shelfBatchRecoveryRequired } from "../../app/recoveryState";
import { backupBusy } from "../settings/backupState";
import { getLocalBookImportSession } from "../import/localBookImportSession";
import type { LocalImportFormat } from "../../api/books";
import type { AppBootstrap, ShelfResource } from "../../api/types";
import PrototypeIcon from "../../ui/PrototypeIcon.vue";
import BackButton from "../../ui/BackButton.vue";
import router, { routeNames } from "../../router";
import { getReaderSession } from "../reader/readerSession";

const localBookImport = getLocalBookImportSession();
const localImportBusy = localBookImport.localBookImportBusy;
const readerSession = getReaderSession();

const pageDataLoading = ref(true);
const pageDataError = ref("");
const localBooks = computed(() => shelf.value.books.filter((book) => book.id.startsWith("local-")).slice(0, 12));
const recoveryRequired = computed(() => shelfBatchRecoveryRequired.value);
const actionsBlocked = computed(() => localBookImport.localBookImportBusy.value || pageDataLoading.value || !!pageDataError.value
  || recoveryRequired.value || localBookImport.pdfPasswordPromptOpen.value
  || localBookImport.externalFileRequestBusy.value || backupBusy.value !== null);
let disposed = false;
const eventUnlisteners: UnlistenFn[] = [];

function errorText(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}



function loadShelf(): void {
  pageDataLoading.value = false;
  pageDataError.value = "";
}

async function setupEvents(): Promise<void> {
  function register(unlisten: UnlistenFn): void {
    if (disposed) unlisten();
    else eventUnlisteners.push(unlisten);
  }
  try {
    register(await listen<AppBootstrap>("app-state-updated", (event) => {
      void event;
      loadShelf();
    }));
  } catch (error) {
    console.debug("Native shelf event subscription unavailable:", errorText(error));
  }
}

onMounted(() => {
  void (async () => {
    await setupEvents();
    await loadShelf();
  })();
});

onBeforeUnmount(() => {
  disposed = true;
  for (const unlisten of eventUnlisteners.splice(0)) unlisten();
});

watch(localBookImport.pdfPasswordPromptOpen, (open, wasOpen) => {
  if (wasOpen && !open) void loadShelf();
});

async function importFile(format: LocalImportFormat): Promise<void> {
  if (actionsBlocked.value) return;
  try {
    await localBookImport.importLocalBook(format);
    await loadShelf();
  } catch (error) {
    notify(`本地导入失败：${errorText(error)}`, "error");
  }
}

const formats = [
  { format: "txt", title: "TXT", desc: "纯文本与自动目录", icon: "text" },
  { format: "epub", title: "EPUB", desc: "电子书与元数据", icon: "book" },
  { format: "cbz", title: "CBZ", desc: "图片压缩包", icon: "image" },
  { format: "pdf", title: "PDF", desc: "文档阅读", icon: "pdf" },
] as const;
const linkInput = ref("");
const linkError = ref("");
function parseLink(): void {
  if (actionsBlocked.value) return;
  linkError.value = "";
  try { localBookImport.requestLinkImport(linkInput.value); }
  catch (error) { linkError.value = errorText(error); }
}

async function openLocalBook(bookId: string): Promise<void> {
  const book = shelf.value.books.find((entry) => entry.id === bookId);
  if (!book?.bookSrc) {
    notify("本地书籍资源暂不可用", "error");
    return;
  }
  try {
    openedBookFromShelf.value = false;
    const current = await switchOpenedBook(bookId);
    if (current) await readerSession.startReading(current);
  } catch (error) {
    notify(`打开阅读失败：${errorText(error)}`, "error");
  }
}
function bookFormat(book: ShelfResource["books"][number]): string {
  if (book.kind === "comic") return "CBZ / 图片";
  if (book.mediaType === "audio") return "音频";
  if (book.mediaType === "video") return "视频";
  return "本地书";
}
</script>

<template>
  <section class="library-import-page">
    <header class="import-subpage-header safe-area-top">
      <BackButton label="返回内容库" @click="router.push({ name: routeNames.shelf })" />
      <div class="import-subpage-title"><strong>导入</strong></div>
      <span></span>
    </header>

    <div class="library-import-body">
      <div class="library-import-grid">
        <button v-for="format in formats" :key="format.format" type="button"
          :disabled="actionsBlocked"
          @click="importFile(format.format)">
          <span class="library-import-file-icon"><PrototypeIcon :name="format.icon"/></span>
          <strong>{{ format.title }}</strong>
          <small>{{ format.desc }}</small>
        </button>
      </div>
      <p v-if="localImportBusy" class="library-import-status">正在通过系统文件选择器和本地解析引擎处理导入…</p>
      <p v-if="recoveryRequired" class="library-import-warning">书架事务需要恢复，请重启应用完成恢复后再继续导入。</p>

      <section class="library-import-section">
        <h2 id="library-import-link-title">通过链接导入</h2>
        <p>支持源、RSS、替换规则、TXT 目录规则和 HTTP TTS 配置。</p>
        <form class="library-import-form" @submit.prevent="parseLink">
          <input v-model="linkInput" type="text" inputmode="url" autocomplete="url"
            placeholder="粘贴链接或 JSON"
            :disabled="actionsBlocked"
            @input="linkError = ''"/>
          <button type="submit" class="library-import-primary" :disabled="actionsBlocked || !linkInput.trim()">识别</button>
        </form>
        <p v-if="linkError" class="library-import-warning">{{ linkError }}</p>
      </section>

      <section class="library-import-section">
        <h2 id="library-import-local-title">已导入的本地内容</h2>
        <p v-if="pageDataLoading" class="library-import-status">正在读取内容库…</p>
        <p v-else-if="pageDataError" class="library-import-warning">
          内容库读取失败：{{ pageDataError }}
          <button type="button" class="library-import-retry" @click="loadShelf">重试</button>
        </p>
        <div v-else-if="localBooks.length" class="library-import-recent-list">
          <article v-for="book in localBooks" :key="book.id">
            <div class="library-import-recent-detail">
              <strong>{{ book.title }}</strong>
              <small>{{ bookFormat(book) }} · {{ book.author || '作者未知' }}</small>
            </div>
            <button type="button" @click="openLocalBook(book.id)">打开</button>
          </article>
        </div>
        <div v-else class="library-import-empty">
          当前没有已导入的本地内容。选择上方格式即可使用系统文件选择器导入。
        </div>
      </section>
    </div>
  </section>
</template>

<style scoped>
.library-import-page{width:100%;min-height:100%;color:var(--app-text)}
.import-subpage-header{position:sticky;top:0;z-index:5;display:grid;grid-template-columns:44px 1fr 38px;align-items:center;gap:12px;min-height:var(--ui-header-height);padding-inline:var(--ui-header-gutter);border-bottom:1px solid var(--app-line);background:var(--app-header);backdrop-filter:blur(14px)}
.import-subpage-title{display:flex;flex-direction:column;align-items:flex-start;gap:3px}
.import-subpage-title strong{font-size:var(--ui-font-heading);font-weight:750}
.library-import-body{width:100%;margin:0 auto;padding:var(--ui-space-6) var(--ui-page-gutter) var(--ui-space-8);max-width:var(--app-content-width)}
.library-import-grid{display:grid;grid-template-columns:repeat(4,minmax(0,1fr));gap:10px}
.library-import-grid button{display:flex;flex-direction:column;align-items:flex-start;gap:5px;min-width:0;min-height:126px;padding:14px;border:1px solid var(--app-line);border-radius:14px;background:var(--app-surface);cursor:pointer;text-align:left}
.library-import-grid button strong{font-size:var(--ui-font-body)}
.library-import-grid button small{color:var(--app-subtle);font-size:var(--ui-font-caption);line-height:1.4}
.library-import-file-icon{display:grid;place-items:center;width:38px;height:38px;margin-bottom:auto;border-radius:11px;background:var(--app-accent-soft);color:var(--app-accent-ink)}
.library-import-file-icon :deep(svg){width:20px;height:20px}
.library-import-section{margin-top:28px;padding-top:20px;border-top:1px solid var(--app-line)}
.library-import-section h2{margin:0;font-size:var(--ui-font-body)}
.library-import-form{display:flex;gap:8px;margin-top:12px}
.library-import-form input{flex:1;min-width:0;min-height:39px;padding:0 12px;border:1px solid var(--app-line);border-radius:10px;background:var(--app-surface);font-size:var(--ui-font-body)}
.library-import-primary{min-height:39px;padding:0 18px;border:0;border-radius:10px;background:var(--app-accent);color:var(--app-on-accent);font-size:var(--ui-font-caption);font-weight:700;cursor:pointer}
.library-import-status,.library-import-warning{font-size:var(--ui-font-caption);line-height:1.6}
.library-import-warning{color:var(--app-danger)}
.library-import-retry{margin-left:8px;padding:2px 7px;border:1px solid currentColor;border-radius:6px;background:transparent;color:inherit;font-size:var(--ui-font-caption);cursor:pointer}
.library-import-recent-list{margin-top:13px}
.library-import-recent-list article{display:grid;grid-template-columns:minmax(0,1fr) auto;gap:12px;align-items:center;min-height:62px;border-bottom:1px solid var(--app-line)}
.library-import-recent-detail{display:flex;flex-direction:column;gap:5px;min-width:0}
.library-import-recent-detail strong{font-size:var(--ui-font-caption);overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
.library-import-recent-detail small{font-size:var(--ui-font-caption);color:var(--app-subtle);overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
.library-import-recent-list article>button{padding:9px 12px;border:0;border-radius:9px;background:var(--app-panel-soft);color:var(--app-muted);font-size:var(--ui-font-body);cursor:pointer}
.library-import-empty{padding:19px 0;color:var(--app-subtle);font-size:var(--ui-font-caption)}
@media(max-width:640px){
.import-subpage-header{min-height:var(--ui-header-height);padding-inline:var(--ui-header-gutter)}
.library-import-body{width:100%;padding:var(--ui-space-6) var(--ui-page-gutter) var(--ui-space-8)}
.library-import-grid{grid-template-columns:repeat(2,minmax(0,1fr));gap:9px}
.library-import-grid button{min-height:112px;padding:12px}
.library-import-section{margin-top:22px;padding-top:18px}
}
</style>
