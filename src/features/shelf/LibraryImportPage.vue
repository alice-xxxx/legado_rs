<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { appBootstrap } from "../../api/core";
import { notify } from "../../app/notifications";
import { shelfBatchRecoveryRequired } from "../../app/recoveryState";
import { readResource } from "../../api/resources";
import { getLocalBookImportSession } from "../import/localBookImportSession";
import type { LocalImportFormat } from "../../api/books";
import type { AppBootstrap, BookResource, ResourceDescriptor, ShelfResource } from "../../api/types";
import PrototypeIcon from "../../ui/PrototypeIcon.vue";
import BackButton from "../../ui/BackButton.vue";
import router, { routeNames } from "../../router";
import { getReaderSession } from "../reader/readerSession";

type LinkKind = "source" | "rules";
const localBookImport = getLocalBookImportSession();
const localImportBusy = localBookImport.localBookImportBusy;
const readerSession = getReaderSession();
const linkOpening = ref(false);

const pageDataLoading = ref(true);
const pageDataError = ref("");
const shelf = ref<ShelfResource>({ schemaVersion: 1, groups: [], books: [] });
const localBooks = computed(() => shelf.value.books.filter((book) => book.id.startsWith("local-")).slice(0, 12));
const recoveryRequired = computed(() => shelfBatchRecoveryRequired.value);
const actionsBlocked = computed(() => localBookImport.localBookImportBusy.value || pageDataLoading.value || !!pageDataError.value
  || recoveryRequired.value || localBookImport.pdfPasswordPromptOpen.value
  || localBookImport.externalFileRequestBusy.value || localBookImport.backupBusy.value !== null || linkOpening.value);
const restoreRevision = ref(0);
let shelfReadRevision = 0;
let bootstrapReadRevision = 0;
let disposed = false;
const eventUnlisteners: UnlistenFn[] = [];

function errorText(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function normalizeShelf(next: ShelfResource): ShelfResource {
  return { ...next, groups: next.groups ?? [], books: next.books ?? [] };
}

async function refreshShelf(descriptor: ResourceDescriptor): Promise<void> {
  const requestRevision = ++shelfReadRevision;
  const currentRestoreRevision = restoreRevision.value;
  const next = normalizeShelf(await readResource<ShelfResource>(descriptor));
  if (disposed || currentRestoreRevision !== restoreRevision.value || requestRevision !== shelfReadRevision) return;
  shelf.value = next;
  pageDataError.value = "";
}

async function loadShelf(): Promise<void> {
  const requestRevision = ++bootstrapReadRevision;
  const currentRestoreRevision = restoreRevision.value;
  pageDataLoading.value = true;
  pageDataError.value = "";
  try {
    const snapshot = await appBootstrap();
    if (disposed || currentRestoreRevision !== restoreRevision.value || requestRevision !== bootstrapReadRevision) return;
    await refreshShelf(snapshot.shelf);
  } catch (error) {
    if (disposed || currentRestoreRevision !== restoreRevision.value || requestRevision !== bootstrapReadRevision) return;
    pageDataError.value = errorText(error);
  } finally {
    if (!disposed && currentRestoreRevision === restoreRevision.value && requestRevision === bootstrapReadRevision) {
      pageDataLoading.value = false;
    }
  }
}

async function refreshRestoredShelf(snapshot: AppBootstrap): Promise<void> {
  restoreRevision.value += 1;
  bootstrapReadRevision += 1;
  const currentRestoreRevision = restoreRevision.value;
  pageDataLoading.value = true;
  pageDataError.value = "";
  try {
    await refreshShelf(snapshot.shelf);
  } catch (error) {
    if (disposed || currentRestoreRevision !== restoreRevision.value) return;
    pageDataError.value = errorText(error);
    notify(`书架恢复后刷新失败：${pageDataError.value}`, "error");
  } finally {
    if (!disposed && currentRestoreRevision === restoreRevision.value) pageDataLoading.value = false;
  }
}

async function setupEvents(): Promise<void> {
  function register(unlisten: UnlistenFn): void {
    if (disposed) unlisten();
    else eventUnlisteners.push(unlisten);
  }
  try {
    register(await listen<ResourceDescriptor>("shelf-updated", (event) => {
      void refreshShelf(event.payload).catch((error) => notify(`书架刷新失败：${errorText(error)}`, "error"));
    }));
    register(await listen<AppBootstrap>("app-state-updated", (event) => {
      void refreshRestoredShelf(event.payload);
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
  restoreRevision.value += 1;
  shelfReadRevision += 1;
  bootstrapReadRevision += 1;
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
  { format: "cbz", title: "CBZ", desc: "漫画压缩包", icon: "image" },
  { format: "pdf", title: "PDF", desc: "文档阅读", icon: "pdf" },
] as const;
const linkInput = ref("");
const linkError = ref("");
const recognizedUrl = ref("");
const recognizedKind = ref<LinkKind | null>(null);
const linkKinds = [
  { kind: "source", label: "内容源 / RSS 配置", note: "交给内容引擎预览和确认，不会直接保存" },
  { kind: "rules", label: "替换与净化规则", note: "打开规则导入确认页面" },
] as const;
function parseLink(): void {
  linkError.value = "";
  recognizedUrl.value = "";
  recognizedKind.value = null;
  const raw = linkInput.value.trim();
  if (!raw) { linkError.value = "请先输入链接。"; return; }
  if (raw.length > 4096) { linkError.value = "链接不能超过 4096 个字符。"; return; }
  try {
    const parsed = new URL(raw);
    let target = parsed;
    if (parsed.protocol === "legado-rs:") {
      if (parsed.username || parsed.password || parsed.hash || (parsed.pathname && parsed.pathname !== "/")
        || !["import-source", "import-rules"].includes(parsed.hostname) || parsed.searchParams.getAll("url").length !== 1) {
        throw new Error("不支持该应用导入链接。");
      }
      recognizedKind.value = parsed.hostname === "import-source" ? "source" : "rules";
      target = new URL(parsed.searchParams.get("url") ?? "");
    }
    if (!["http:", "https:"].includes(target.protocol) || target.username || target.password) {
      throw new Error("仅支持无账号密码的 HTTP/HTTPS 导入地址。");
    }
    recognizedUrl.value = target.href;
  } catch (error) {
    recognizedKind.value = null;
    linkError.value = error instanceof Error ? error.message : "无效的导入链接。";
  }
}
async function openLink(kind: LinkKind): Promise<void> {
  if (!recognizedUrl.value || actionsBlocked.value || linkOpening.value) return;
  linkOpening.value = true;
  try {
    const target = kind === "source"
      ? router.push({ name: routeNames.sources, query: { sourceImport: "url", importUrl: recognizedUrl.value } })
      : router.push({ name: routeNames.settingsRules, query: { importUrl: recognizedUrl.value } });
    const failure = await target;
    if (failure) notify("无法打开导入页面", "error");
  } catch (error) {
    notify(errorText(error), "error");
  } finally {
    linkOpening.value = false;
  }
}

async function openLocalBook(bookId: string): Promise<void> {
  const book = shelf.value.books.find((entry) => entry.id === bookId);
  if (!book?.bookSrc) {
    notify("本地书籍资源暂不可用", "error");
    return;
  }
  try {
    await readerSession.startReading(await readResource<BookResource>(book.bookSrc));
  } catch (error) {
    notify(`打开阅读失败：${errorText(error)}`, "error");
  }
}
function bookFormat(book: ShelfResource["books"][number]): string {
  if (book.kind === "comic") return "CBZ / 漫画";
  if (book.mediaType === "audio") return "音频";
  if (book.mediaType === "video") return "视频";
  return "本地书";
}
</script>

<template>
  <section class="library-import-page">
    <header class="import-subpage-header">
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
        <p>支持内容源、RSS 来源配置、替换与净化规则的 HTTP/HTTPS 地址或本应用导入链接。</p>
        <form class="library-import-form" @submit.prevent="parseLink">
          <input v-model="linkInput" type="url" inputmode="url" autocomplete="url"
            placeholder="粘贴链接"
            :disabled="actionsBlocked"
            @input="recognizedUrl = ''; recognizedKind = null; linkError = ''"/>
          <button type="submit" class="library-import-primary" :disabled="actionsBlocked || !linkInput.trim()">识别</button>
        </form>
        <p v-if="linkError" class="library-import-warning">{{ linkError }}</p>
        <div v-if="recognizedUrl" class="library-import-link-recognized">
          <p v-if="recognizedKind">已识别为{{ recognizedKind === 'source' ? '内容源' : '替换规则' }}导入链接。将进入对应的预览/确认流程。</p>
          <p v-else>已验证地址格式。链接内容尚未请求，请选择需要使用的导入处理器。</p>
          <button v-for="kind in linkKinds.filter(item => !recognizedKind || item.kind === recognizedKind)"
            :key="kind.kind" type="button" :disabled="actionsBlocked"
            @click="openLink(kind.kind)">
            <strong>{{ kind.label }}</strong><small>{{ kind.note }}</small><PrototypeIcon name="chevron-right"/>
          </button>
        </div>
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
.library-import-page{width:100%;min-height:100%;color:#262832}
.import-subpage-header{position:sticky;top:0;z-index:5;display:grid;grid-template-columns:44px 1fr 38px;align-items:center;gap:12px;min-height:64px;padding:0 20px;border-bottom:1px solid var(--app-line);background:#f6f7f9f5;backdrop-filter:blur(14px)}
.import-subpage-title{display:flex;flex-direction:column;align-items:flex-start;gap:3px}
.import-subpage-title strong{font-size:14px;font-weight:750}
.library-import-body{width:min(860px,calc(100% - 36px));margin:0 auto;padding:24px 0 48px}
.library-import-grid{display:grid;grid-template-columns:repeat(4,minmax(0,1fr));gap:10px}
.library-import-grid button{display:flex;flex-direction:column;align-items:flex-start;gap:5px;min-width:0;min-height:126px;padding:14px;border:1px solid var(--app-line);border-radius:14px;background:#fff;cursor:pointer;text-align:left}
.library-import-grid button:hover:not(:disabled){border-color:#d2c9f5;background:#fbfaff}
.library-import-grid button strong{font-size:12px}
.library-import-grid button small{color:#999da6;font-size:10px;line-height:1.4}
.library-import-file-icon{display:grid;place-items:center;width:38px;height:38px;margin-bottom:auto;border-radius:11px;background:#efebff;color:#6857c9}
.library-import-file-icon :deep(svg){width:20px;height:20px}
.library-import-section{margin-top:28px;padding-top:20px;border-top:1px solid var(--app-line)}
.library-import-section h2{margin:0;font-size:13px}
.library-import-form{display:flex;gap:8px;margin-top:12px}
.library-import-form input{flex:1;min-width:0;min-height:39px;padding:0 12px;border:1px solid #e1e4eb;border-radius:10px;background:#fff;font-size:12px}
.library-import-primary{min-height:39px;padding:0 18px;border:0;border-radius:10px;background:var(--app-accent);color:#fff;font-size:12px;font-weight:700;cursor:pointer}
.library-import-status,.library-import-warning{font-size:11px;line-height:1.6}
.library-import-warning{color:#b0535b}
.library-import-retry{margin-left:8px;padding:2px 7px;border:1px solid currentColor;border-radius:6px;background:transparent;color:inherit;font-size:10px;cursor:pointer}
.library-import-link-recognized{display:grid;gap:6px;margin-top:12px;padding:11px;border:1px solid #dedaf4;border-radius:12px;background:#f7f5ff}
.library-import-link-recognized p{margin:0 0 5px;color:#757281;font-size:11px}
.library-import-link-recognized button{display:grid;grid-template-columns:1fr 20px;gap:4px 8px;min-height:45px;padding:8px 10px;border:1px solid #e6e3f3;border-radius:9px;background:#fff;text-align:left;cursor:pointer}
.library-import-link-recognized button strong{font-size:11px}
.library-import-link-recognized button small{color:#9498a1;font-size:10px;grid-column:1}
.library-import-link-recognized button :deep(svg){grid-column:2;grid-row:1/3;align-self:center;width:17px;height:17px}
.library-import-recent-list{margin-top:13px}
.library-import-recent-list article{display:grid;grid-template-columns:minmax(0,1fr) auto;gap:12px;align-items:center;min-height:62px;border-bottom:1px solid #e9eaf0}
.library-import-recent-detail{display:flex;flex-direction:column;gap:5px;min-width:0}
.library-import-recent-detail strong{font-size:12px;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
.library-import-recent-detail small{font-size:10px;color:#9398a3;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
.library-import-recent-list article>button{padding:9px 12px;border:0;border-radius:9px;background:#edeef2;color:#636977;font-size:11px;cursor:pointer}
.library-import-empty{padding:19px 0;color:#9195a0;font-size:11px}
@media(max-width:640px){
.import-subpage-header{min-height:55px;padding:0 12px}
.library-import-body{width:calc(100% - 28px);padding:16px 0 32px}
.library-import-grid{grid-template-columns:repeat(2,minmax(0,1fr));gap:9px}
.library-import-grid button{min-height:112px;padding:12px}
.library-import-section{margin-top:22px;padding-top:18px}
}
</style>
