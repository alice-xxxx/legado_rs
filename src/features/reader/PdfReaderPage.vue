<script setup lang="ts">
// PDF 阅读视图持有渲染任务并在卸载时释放，避免跨章节残留画布。
import "core-js/actual/promise/with-resolvers.js";
import { getDocument, GlobalWorkerOptions, PasswordResponses, type PDFDocumentLoadingTask, type PDFDocumentProxy, type RenderTask } from "pdfjs-dist/legacy/build/pdf.mjs";
import { computed, onBeforeUnmount, onMounted, ref, shallowRef, watch } from "vue";
import workerUrl from "../../pdfjs-worker.ts?worker&url";
import PrototypeIcon from "../../ui/PrototypeIcon.vue";

type PdfZoom = "page-fit" | "page-width" | "actual-size";

const props = withDefaults(defineProps<{
  src: string;
  pageIndex: number;
  defaultZoom?: PdfZoom;
  initialPassword?: string;
  findOpen?: boolean;
}>(), {
  defaultZoom: "page-fit",
  initialPassword: "",
  findOpen: false,
});

const emit = defineEmits<{
  loaded: [pageCount: number];
  pageSelected: [pageIndex: number];
  "update:findOpen": [open: boolean];
  initialPasswordUsed: [];
  closeReader: [];
}>();

GlobalWorkerOptions.workerSrc = workerUrl;

const stage = ref<HTMLDivElement | null>(null);
const canvasScroller = ref<HTMLDivElement | null>(null);
const canvas = ref<HTMLCanvasElement | null>(null);
const thumbnails = ref<Array<{ index: number; src: string }>>([]);
const thumbnailStart = ref(0);
const thumbnailPageWindow = 5;
const findInput = ref<HTMLInputElement | null>(null);
const findQuery = ref("");
const findPageMatches = ref<number[]>([]);
const findSearching = ref(false);
const findError = ref("");
let findRevision = 0;
let findDebounce: ReturnType<typeof setTimeout> | null = null;
const loading = ref(true);
const error = ref("");
const passwordPromptOpen = ref(false);
const passwordInput = ref("");
const passwordError = ref("");
const documentProxy = shallowRef<PDFDocumentProxy | null>(null);
const pageCount = ref(0);
const renderError = ref("");

let loadingTask: PDFDocumentLoadingTask | null = null;
let renderTask: RenderTask | null = null;
let passwordUpdater: ((password: string) => void) | null = null;
let initialPasswordUsed = false;
let resizeObserver: ResizeObserver | null = null;
let disposed = false;
let renderRevision = 0;
let thumbnailRevision = 0;
const thumbnailCache = new Map<number, string>();
const thumbnailCacheLimit = 20;

const pageLabel = computed(() => `第 ${props.pageIndex + 1} / ${pageCount.value || "—"} 页`);

function visibleThumbStart(pageIndex: number, pages: number): number {
  return Math.max(0, Math.min(Math.max(0, pages - thumbnailPageWindow), pageIndex - 2));
}

function setThumbnailWindow(start: number): void {
  const next = Math.max(0, Math.min(Math.max(0, pageCount.value - thumbnailPageWindow), start));
  if (next === thumbnailStart.value) return;
  thumbnailStart.value = next;
  void renderThumbnails();
}

/**
 * Render only the small visible rail window from the same PDFDocumentProxy.
 * No fake thumbnails and no duplicate getDocument / network requests.
 */
async function renderThumbnails(): Promise<void> {
  const pdf = documentProxy.value;
  const revision = ++thumbnailRevision;
  if (!pdf || disposed || pageCount.value <= 1) {
    thumbnails.value = [];
    return;
  }
  const images: Array<{ index: number; src: string }> = [];
  const end = Math.min(pageCount.value, thumbnailStart.value + thumbnailPageWindow);
  for (let index = thumbnailStart.value; index < end; index++) {
    const cached = thumbnailCache.get(index);
    if (cached) {
      thumbnailCache.delete(index);
      thumbnailCache.set(index, cached);
      images.push({ index, src: cached });
      continue;
    }
    try {
      const page = await pdf.getPage(index + 1);
      if (disposed || revision !== thumbnailRevision || documentProxy.value !== pdf) return;
      const base = page.getViewport({ scale: 1 });
      const scale = Math.min(1, 57 / Math.max(1, base.width));
      const viewport = page.getViewport({ scale });
      const preview = window.document.createElement("canvas");
      preview.width = Math.max(1, Math.ceil(viewport.width));
      preview.height = Math.max(1, Math.ceil(viewport.height));
      const context = preview.getContext("2d", { alpha: false });
      if (!context) return;
      const task = page.render({ canvas: preview, canvasContext: context, viewport, background: "#fff" });
      await task.promise;
      if (disposed || revision !== thumbnailRevision || documentProxy.value !== pdf) return;
      const src = preview.toDataURL("image/png");
      thumbnailCache.set(index, src);
      while (thumbnailCache.size > thumbnailCacheLimit) {
        const oldest = thumbnailCache.keys().next().value;
        if (oldest === undefined) break;
        thumbnailCache.delete(oldest);
      }
      images.push({ index, src });
    } catch {
      // An unreadable preview must not prevent opening or navigating the PDF.
    }
  }
  if (!disposed && revision === thumbnailRevision) thumbnails.value = images;
}


function cancelPdfFind(): void {
  findRevision += 1;
  if (findDebounce) clearTimeout(findDebounce);
  findDebounce = null;
  findSearching.value = false;
}
async function findPdfPages(): Promise<void> {
  const document = documentProxy.value;
  const query = findQuery.value.trim().toLocaleLowerCase();
  const rev = ++findRevision;
  findPageMatches.value = [];
  findError.value = "";
  if (!document || !query || !props.findOpen || disposed) {
    findSearching.value = false;
    return;
  }
  findSearching.value = true;
  try {
    const matches: number[] = [];
    for (let number = 1; number <= document.numPages; number++) {
      if (disposed || rev !== findRevision || documentProxy.value !== document) return;
      const page = await document.getPage(number);
      const text = await page.getTextContent();
      if (disposed || rev !== findRevision || documentProxy.value !== document) return;
      const fragments = text.items.map(item => "str" in item ? item.str : "");
      const exact = fragments.join("").toLocaleLowerCase();
      const spaced = fragments.join(" ").toLocaleLowerCase();
      if (exact.includes(query) || spaced.includes(query)) matches.push(number - 1);
      // Publish at most once per 10 pages; avoid re-rendering a large results list for each page.
      if (number % 10 === 0) {
        findPageMatches.value = matches.slice();
        await new Promise<void>(resolve => setTimeout(resolve, 0));
      }
    }
    if (rev === findRevision && !disposed) findPageMatches.value = matches;
  } catch {
    if (rev === findRevision) findError.value = "无法读取部分 PDF 页面的文本，请尝试其他关键词。";
  } finally {
    if (rev === findRevision && !disposed) findSearching.value = false;
  }
}
watch(() => [props.findOpen, findQuery.value] as const, () => {
  cancelPdfFind();
  if (!props.findOpen || !findQuery.value.trim()) {
    findPageMatches.value = [];
    findError.value = "";
    return;
  }
  findDebounce = setTimeout(() => { findDebounce = null; void findPdfPages(); }, 220);
});
watch(() => props.findOpen, open => {
  if (open) void Promise.resolve().then(() => findInput.value?.focus());
});

function readableError(value: unknown): string {
  if (value instanceof Error && /password/i.test(value.message)) return "密码不正确，或此文件无法用该密码打开。";
  return "无法显示这份 PDF，请检查文件后重试。";
}

async function openDocument(): Promise<void> {
  cancelPdfFind();
  findPageMatches.value = [];
  ++renderRevision;
  renderTask?.cancel();
  renderTask = null;
  thumbnailCache.clear();
  loading.value = true;
  error.value = "";
  renderError.value = "";
  documentProxy.value = null;
  thumbnailRevision++;
  thumbnails.value = [];
  pageCount.value = 0;
  passwordPromptOpen.value = false;
  passwordUpdater = null;
  let task: PDFDocumentLoadingTask | null = null;
  try {
    task = getDocument({
      url: props.src,
      rangeChunkSize: 256 * 1024,
      disableAutoFetch: false,
      isOffscreenCanvasSupported: false,
      isImageDecoderSupported: false,
    });
    loadingTask = task;
    task.onPassword = (updatePassword: (password: string) => void, reason: number) => {
      if (disposed || loadingTask !== task) return;
      if (!initialPasswordUsed && props.initialPassword) {
        const initialPassword = props.initialPassword;
        initialPasswordUsed = true;
        emit("initialPasswordUsed");
        updatePassword(initialPassword);
        return;
      }
      passwordUpdater = updatePassword;
      passwordError.value = reason === PasswordResponses.INCORRECT_PASSWORD ? "密码不正确，请再试一次。" : "输入 PDF 密码后继续。";
      passwordInput.value = "";
      passwordPromptOpen.value = true;
      loading.value = false;
    };
    const document = await task.promise;
    if (disposed || loadingTask !== task) return;
    documentProxy.value = document;
    pageCount.value = document.numPages;
    if (props.findOpen && findQuery.value.trim()) void findPdfPages();
    thumbnailStart.value = visibleThumbStart(props.pageIndex, document.numPages);
    void renderThumbnails();
    passwordPromptOpen.value = false;
    passwordInput.value = "";
    passwordError.value = "";
    await renderPage();
  } catch (cause) {
    if (disposed || loadingTask !== task) return;
    if (!passwordPromptOpen.value) error.value = readableError(cause);
  } finally {
    if (!disposed && loadingTask === task && !passwordPromptOpen.value) loading.value = false;
  }
}

async function renderPage(): Promise<void> {
  const document = documentProxy.value;
  const target = canvas.value;
  const container = canvasScroller.value ?? stage.value;
  if (!document || !target || !container || disposed) return;
  const revision = ++renderRevision;
  renderTask?.cancel();
  renderTask = null;
  renderError.value = "";
  loading.value = true;
  try {
    const page = await document.getPage(Math.min(Math.max(1, props.pageIndex + 1), document.numPages));
    if (disposed || revision !== renderRevision || canvas.value !== target) return;
    const unscaled = page.getViewport({ scale: 1 });
    const width = Math.max(1, container.clientWidth - 24);
    const height = Math.max(1, container.clientHeight - 24);
    const widthScale = width / unscaled.width;
    const heightScale = height / unscaled.height;
    const scale = props.defaultZoom === "actual-size"
      ? 1
      : props.defaultZoom === "page-width"
        ? widthScale
        : Math.min(widthScale, heightScale);
    const viewport = page.getViewport({ scale });
    const pixelRatio = Math.min(2, Math.max(1, window.devicePixelRatio || 1));
    target.width = Math.max(1, Math.floor(viewport.width * pixelRatio));
    target.height = Math.max(1, Math.floor(viewport.height * pixelRatio));
    target.style.width = `${Math.ceil(viewport.width)}px`;
    target.style.height = `${Math.ceil(viewport.height)}px`;
    target.dataset.pdfZoom = props.defaultZoom;
    target.dataset.pdfScale = scale.toFixed(4);
    const context = target.getContext("2d", { alpha: false });
    if (!context) throw new Error("Canvas is unavailable.");
    renderTask = page.render({
      canvas: target,
      canvasContext: context,
      viewport,
      transform: pixelRatio === 1 ? undefined : [pixelRatio, 0, 0, pixelRatio, 0, 0],
      background: "#fff",
    });
    await renderTask.promise;
    if (disposed || revision !== renderRevision) return;
    emit("loaded", document.numPages);
  } catch (cause) {
    if (disposed || revision !== renderRevision || (cause instanceof Error && cause.name === "RenderingCancelledException")) return;
    renderError.value = readableError(cause);
  } finally {
    if (!disposed && revision === renderRevision) loading.value = false;
  }
}

function submitPassword(): void {
  const password = passwordInput.value;
  const updatePassword = passwordUpdater;
  if (!password || !updatePassword) return;
  passwordInput.value = "";
  passwordError.value = "";
  passwordPromptOpen.value = false;
  passwordUpdater = null;
  loading.value = true;
  updatePassword(password);
}

async function cancelPassword(): Promise<void> {
  passwordInput.value = "";
  passwordUpdater = null;
  passwordPromptOpen.value = false;
  if (loadingTask) {
    const task = loadingTask;
    loadingTask = null;
    await task.destroy().catch(() => {});
  }
  emit("closeReader");
}

async function retry(): Promise<void> {
  const task = loadingTask;
  loadingTask = null;
  documentProxy.value = null;
  await task?.destroy().catch(() => {});
  await openDocument();
}

watch(() => props.src, () => void retry());
watch(() => props.pageIndex, () => {
  if (props.pageIndex < thumbnailStart.value
      || props.pageIndex >= thumbnailStart.value + thumbnailPageWindow) {
    thumbnailStart.value = visibleThumbStart(props.pageIndex, pageCount.value);
    void renderThumbnails();
  }
  void renderPage();
});
watch(() => props.defaultZoom, () => void renderPage());

onMounted(() => {
  resizeObserver = new ResizeObserver(() => void renderPage());
  if (stage.value) resizeObserver.observe(stage.value);
  void openDocument();
});

onBeforeUnmount(() => {
  disposed = true;
  ++renderRevision;
  cancelPdfFind();
  thumbnailRevision++;
  thumbnailCache.clear();
  thumbnails.value = [];
  resizeObserver?.disconnect();
  renderTask?.cancel();
  renderTask = null;
  passwordInput.value = "";
  passwordUpdater = null;
  const task = loadingTask;
  loadingTask = null;
  if (task) void task.destroy().catch(() => {});
});
</script>

<template>
  <div ref="stage" class="pdf-reader-stage">
    <section v-if="findOpen && !passwordPromptOpen" class="pdf-find-panel">
      <header><strong>搜索 PDF 文本</strong><button type="button" @click="emit('update:findOpen', false)"><PrototypeIcon name="close"/></button></header>
      <label for="pdf-find-input">关键词</label>
      <input id="pdf-find-input" ref="findInput" v-model="findQuery" type="search" placeholder="搜索文档中的文字" autocomplete="off"/>
      <p>{{ findSearching ? '正在扫描 PDF 页面…' : !findQuery.trim() ? '输入关键词查找包含它的页面' : `找到 ${findPageMatches.length} 页` }}</p>
      <p v-if="findError">{{ findError }}</p>
      <div v-if="findPageMatches.length" class="pdf-find-results">
        <button v-for="index in findPageMatches.slice(0,100)" :key="index" type="button"
          :class="{ active: index === pageIndex }" @click="emit('pageSelected', index)">
          第 {{ index + 1 }} 页
        </button>
      </div>
      <small v-if="findPageMatches.length > 100">仅展示前 100 个匹配页面，可缩小关键词范围。</small>
    </section>
    <nav v-if="pageCount > 1 && !passwordPromptOpen" class="pdf-thumbnail-rail">
      <button type="button" class="pdf-thumb-window-control" :disabled="thumbnailStart <= 0" @click="setThumbnailWindow(thumbnailStart - thumbnailPageWindow)">上一组</button>
      <div class="pdf-thumb-list">
        <button v-for="thumbnail in thumbnails" :key="thumbnail.index" type="button"
          :class="{ active: thumbnail.index === pageIndex }"
          @click="emit('pageSelected',thumbnail.index)">
          <img :src="thumbnail.src" loading="lazy"/>
          <small>{{ thumbnail.index + 1 }}</small>
        </button>
      </div>
      <button type="button" class="pdf-thumb-window-control" :disabled="thumbnailStart + thumbnailPageWindow >= pageCount" @click="setThumbnailWindow(thumbnailStart + thumbnailPageWindow)">下一组</button>
    </nav>
    <div ref="canvasScroller" class="pdf-canvas-scroll" :class="{ 'pdf-with-thumbnail-rail': pageCount > 1 && !passwordPromptOpen }">
      <canvas ref="canvas"></canvas>
    </div>
    <div v-if="loading && !passwordPromptOpen" class="pdf-reader-status">正在显示 PDF…</div>
    <div v-if="error || renderError" class="pdf-reader-status pdf-reader-error">
      <p>{{ error || renderError }}</p>
      <button class="button secondary" @click="retry">重试</button>
      <button class="button secondary" @click="emit('closeReader')">返回书架</button>
    </div>

    <div v-if="passwordPromptOpen" class="pdf-password-backdrop">
      <form class="pdf-password-dialog" @submit.prevent="submitPassword">
        <h2>输入 PDF 密码</h2>
        <p>密码只用于本次阅读，不会保存。</p>
        <label for="pdf-reader-password">PDF 密码</label>
        <input id="pdf-reader-password" v-model="passwordInput" type="password" autocomplete="off" autofocus />
        <p v-if="passwordError" class="pdf-password-error">{{ passwordError }}</p>
        <div class="pdf-password-actions">
          <button type="button" class="button secondary" @click="cancelPassword">取消阅读</button>
          <button type="submit" class="button primary" :disabled="!passwordInput">打开 PDF</button>
        </div>
      </form>
    </div>
    <span class="pdf-page-label">{{ pageLabel }}</span>
  </div>
</template>

<style scoped>

.pdf-reader-stage { position: absolute; inset: 0; overflow: hidden; background: #fff; }
.pdf-canvas-scroll {
  position: absolute; inset: 0;
  display: flex; align-items: flex-start;
  justify-content: center; overflow: auto;
  padding: 12px;
}
.pdf-canvas-scroll canvas { display: block; flex: none; background: #fff; }
.pdf-canvas-scroll.pdf-with-thumbnail-rail { left: 82px; }

.pdf-thumbnail-rail {
  position: absolute; z-index: 3;
  inset: 0 auto 0 0; display: flex;
  width: 82px; flex-direction: column; gap: 6px;
  overflow: hidden; padding: 10px 6px;
}
.pdf-thumb-window-control { min-height: 32px; flex: none; }
.pdf-thumb-list { display: flex; min-height: 0; flex: 1; flex-direction: column; align-items: center; gap: 8px; overflow-y: auto; }
.pdf-thumb-list button { display: grid; width: 100%; justify-items: center; gap: 3px; padding: 5px 2px; border: 1px solid transparent; }
.pdf-thumb-list button.active { border-color: var(--app-accent); }
.pdf-thumb-list img { display: block; max-width: 100%; max-height: 90px; object-fit: contain; }

.pdf-reader-status { position: absolute; z-index: 2; inset: 0; display: grid; place-content: center; gap: 12px; text-align: center; }
.pdf-reader-status p { margin: 0; }
.pdf-reader-error { justify-items: center; }
.pdf-page-label { position: absolute; right: 10px; bottom: 10px; padding: 4px 8px; background: #fff; }
.pdf-password-backdrop {
  position: absolute; z-index: 4;
  inset: 0; display: grid;
  place-items: center; padding: 16px;
  background: rgb(0 0 0 / 24%);
}
.pdf-password-dialog { display: grid; width: min(380px, 100%); gap: 10px; padding: 22px; background: var(--app-surface); }
.pdf-password-dialog h2, .pdf-password-dialog p { margin: 0; }
.pdf-password-dialog input { width: 100%; min-height: 44px; box-sizing: border-box; }
.pdf-password-actions { display: flex; flex-wrap: wrap; justify-content: flex-end; gap: 8px; }

.pdf-find-panel {
  position: absolute; z-index: 8;
  top: 10px; right: 11px;
  display: grid; width: min(310px, calc(100% - 72px));
  max-height: calc(100% - 70px); gap: 8px;
  overflow: auto; padding: 13px;
  background: var(--app-surface);
}
.pdf-find-panel header { display: flex; align-items: center; justify-content: space-between; gap: 8px; }
.pdf-find-panel input { width: 100%; min-height: 39px; box-sizing: border-box; }
.pdf-find-panel p { margin: 0; }
.pdf-find-results { display: grid; grid-template-columns: repeat(3, minmax(0, 1fr)); gap: 6px; }
.pdf-find-results button { min-height: 36px; border: 1px solid transparent; }
.pdf-find-results button.active { border-color: var(--app-accent); }

@media (max-width: 640px) {
  .pdf-thumbnail-rail { width: 54px; }
  .pdf-thumb-list img { max-height: 64px; }
  .pdf-canvas-scroll.pdf-with-thumbnail-rail { left: 54px; }
}

</style>
