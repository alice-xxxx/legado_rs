<script setup lang="ts">
import "core-js/actual/promise/with-resolvers.js";
import { getDocument, GlobalWorkerOptions, PasswordResponses, type PDFDocumentLoadingTask, type PDFDocumentProxy, type RenderTask } from "pdfjs-dist/legacy/build/pdf.mjs";
import { computed, onBeforeUnmount, onMounted, ref, shallowRef, watch } from "vue";
import workerUrl from "../pdfjs-worker.ts?worker&url";

type PdfZoom = "page-fit" | "page-width" | "actual-size";

const props = withDefaults(defineProps<{
  src: string;
  pageIndex: number;
  defaultZoom?: PdfZoom;
  initialPassword?: string;
}>(), {
  defaultZoom: "page-fit",
  initialPassword: "",
});

const emit = defineEmits<{
  loaded: [pageCount: number];
  initialPasswordUsed: [];
  closeReader: [];
}>();

GlobalWorkerOptions.workerSrc = workerUrl;

const stage = ref<HTMLDivElement | null>(null);
const canvas = ref<HTMLCanvasElement | null>(null);
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

const pageLabel = computed(() => `第 ${props.pageIndex + 1} / ${pageCount.value || "—"} 页`);

function readableError(value: unknown): string {
  if (value instanceof Error && /password/i.test(value.message)) return "密码不正确，或此文件无法用该密码打开。";
  return "无法显示这份 PDF，请检查文件后重试。";
}

async function openDocument(): Promise<void> {
  loading.value = true;
  error.value = "";
  renderError.value = "";
  documentProxy.value = null;
  pageCount.value = 0;
  passwordPromptOpen.value = false;
  passwordUpdater = null;
  try {
    const task = getDocument({
      url: props.src,
      rangeChunkSize: 256 * 1024,
      disableAutoFetch: false,
      isOffscreenCanvasSupported: false,
      isImageDecoderSupported: false,
    });
    loadingTask = task;
    task.onPassword = (updatePassword: (password: string) => void, reason: number) => {
      if (disposed) return;
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
    passwordPromptOpen.value = false;
    passwordInput.value = "";
    passwordError.value = "";
    await renderPage();
  } catch (cause) {
    if (disposed) return;
    if (!passwordPromptOpen.value) error.value = readableError(cause);
  } finally {
    if (!disposed && !passwordPromptOpen.value) loading.value = false;
  }
}

async function renderPage(): Promise<void> {
  const document = documentProxy.value;
  const target = canvas.value;
  const container = stage.value;
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
    target.setAttribute("aria-label", `PDF ${pageLabel.value}`);
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
watch(() => [props.pageIndex, props.defaultZoom] as const, () => void renderPage());

onMounted(() => {
  resizeObserver = new ResizeObserver(() => void renderPage());
  if (stage.value) resizeObserver.observe(stage.value);
  void openDocument();
});

onBeforeUnmount(() => {
  disposed = true;
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
  <div ref="stage" class="pdf-reader-stage" data-testid="pdf-reader-stage">
    <div class="pdf-canvas-scroll">
      <canvas ref="canvas" data-testid="pdf-canvas" aria-label="PDF 页面"></canvas>
    </div>
    <div v-if="loading && !passwordPromptOpen" class="pdf-reader-status" role="status">正在显示 PDF…</div>
    <div v-if="error || renderError" class="pdf-reader-status pdf-reader-error" role="alert">
      <p>{{ error || renderError }}</p>
      <button class="button secondary" data-testid="pdf-reader-retry" @click="retry">重试</button>
      <button class="button secondary" data-testid="pdf-reader-close" @click="emit('closeReader')">返回书架</button>
    </div>

    <div v-if="passwordPromptOpen" class="pdf-password-backdrop">
      <form class="pdf-password-dialog" data-testid="pdf-reader-password-dialog" @submit.prevent="submitPassword">
        <h2>输入 PDF 密码</h2>
        <p>密码只用于本次阅读，不会保存。</p>
        <label for="pdf-reader-password">PDF 密码</label>
        <input id="pdf-reader-password" v-model="passwordInput" data-testid="pdf-reader-password" type="password" autocomplete="off" autofocus />
        <p v-if="passwordError" class="pdf-password-error" role="alert">{{ passwordError }}</p>
        <div class="pdf-password-actions">
          <button type="button" class="button secondary" data-testid="pdf-reader-password-cancel" @click="cancelPassword">取消阅读</button>
          <button type="submit" class="button primary" data-testid="pdf-reader-password-submit" :disabled="!passwordInput">打开 PDF</button>
        </div>
      </form>
    </div>
    <span class="pdf-page-label" data-testid="pdf-page-label">{{ pageLabel }}</span>
  </div>
</template>
