import { computed, onBeforeUnmount, ref, type Ref } from "vue";
import { listen } from "@tauri-apps/api/event";
import { isTauri } from "@tauri-apps/api/core";
import { startTts as beginTts, stopTts as endTts, submitTts, ttsVoices as loadVoices, type TtsProgress, type TtsVoice } from "../../api/tts";
import { readingBook } from "./readerBookState";
import { settings } from "../settings/settingsStore";
import { httpTtsConfigs } from "../settings/configurationStores";
import { clamp, type PdfPageResource } from "./readerContent";
import { mapTtsSegmentChunks, splitTtsText, type ReaderTtsChunkTarget, type ReaderTtsSegment } from "./ttsText";
import { openMediaSession } from "./mediaSession";
import { pauseTtsOutput, setTtsOutputRate, setupTtsOutput } from "./ttsOutput";

export interface ReaderTtsContext {
  visible: Ref<boolean>;
  busy: Ref<boolean>;
  mediaChapter: Ref<boolean>;
  pdfPage: Ref<PdfPageResource | null>;
  imagePage: Ref<boolean>;
  chapterIndex: Ref<number>;
  getLoadRevision: () => number;
}
export interface UseReaderTtsOptions {
  reader: ReaderTtsContext;
  loadSegments: (fromCurrentPosition?: boolean) => Promise<ReaderTtsSegment[]>;
  prepareSegments: (index: number) => Promise<ReaderTtsSegment[]>;
  openChapter: (index: number) => Promise<void>;
  highlightChunk: (target: ReaderTtsChunkTarget | undefined, loadRevision: number) => void;
  clearHighlight: () => void;
  errorText: (error: unknown) => string;
}

/** Vue 只投递正文和显示进度；朗读、流式音频和队列由 Rust 管理。 */
export function useReaderTts(options: UseReaderTtsOptions) {
  let mediaSession: ReturnType<typeof openMediaSession> | undefined;
  const ttsVoices = ref<TtsVoice[]>([]);
  const ttsActive = ref(false);
  const ttsPaused = ref(false);
  const ttsBuffering = ref(false);
  let initialRate = 1;
  const ttsSelectedText = ref("");
  const ttsError = ref("");
  const ttsNotice = ref("");
  const ttsExpectedChapterKey = ref<string | null>(null);
  const ttsChunkProgress = ref<{ current: number; total: number } | null>(null);
  type Chunk = { text: string; target?: ReaderTtsChunkTarget };
  let chapters: Array<{ index: number; start: number; targets: Array<ReaderTtsChunkTarget | undefined>; revision: number }> = [];
  let nextIndex = 0;
  let latestIndex = 0;
  let generation = 0;
  let selected = false;
  let advancing = false;
  let ready: Promise<void> | undefined;
  let destroyed = false;
  const cleanup: Array<() => void> = [];

  const ttsEngineSelection = computed(() => settings.value.reader.ttsEngine === "http"
    ? `http:${settings.value.reader.httpTtsConfigId ?? ""}` : `system:${settings.value.reader.ttsVoiceURI}`);
  const ttsCanStart = computed(() => options.reader.visible.value && !options.reader.busy.value
    && !options.reader.mediaChapter.value && !options.reader.pdfPage.value && !options.reader.imagePage.value
    && !ttsActive.value && (settings.value.reader.ttsEngine !== "http"
      || httpTtsConfigs.value.some(item => item.id === settings.value.reader.httpTtsConfigId && item.enabled)));

  async function refreshTtsVoices(): Promise<void> {
    try { ttsVoices.value = await loadVoices(); }
    catch (error) { ttsError.value = options.errorText(error); }
  }

  async function onProgress(progress: TtsProgress): Promise<void> {
    if (!ttsActive.value) return;
    if (typeof progress === "string") { ttsError.value = progress; await stopTts(false); return; }
    if (typeof progress === "object") {
      if ("finished" in progress) { ttsNotice.value = "朗读完成。"; await stopTts(false); }
      else if ("buffering" in progress) ttsBuffering.value = progress.buffering;
      else { ttsPaused.value = progress.paused; mediaSession?.setPaused(progress.paused); }
      return;
    }
    latestIndex = progress;
    ttsBuffering.value = false;
    if (advancing) return;
    const current = generation;
    advancing = true;
    try {
      while (current === generation && ttsActive.value) {
        const chapter = chapters.find(chapter => latestIndex >= chapter.start && latestIndex < chapter.start + chapter.targets.length);
        if (!chapter) return;
        // 保留投递音频时的原文范围；切章后只在新 DOM 中解析，不重新切分或按序号猜位置。
        if (chapter.index !== options.reader.chapterIndex.value) {
          options.clearHighlight();
          ttsExpectedChapterKey.value = `${readingBook.value?.id}:${chapter.index}`;
          await options.openChapter(chapter.index);
          if (current !== generation) return;
          if (chapter.index !== options.reader.chapterIndex.value) throw new Error("正在朗读的章节尚未就绪。");
          chapter.revision = options.reader.getLoadRevision();
          chapters = chapters.filter(entry => entry.start >= chapter.start);
          void prepareNext(chapter.index, current);
          // 加载期间音频可能已推进，重新读取最新进度。
          continue;
        }
        const index = latestIndex - chapter.start;
        ttsChunkProgress.value = { current: index + 1, total: chapter.targets.length };
        options.highlightChunk(chapter.targets[index], chapter.revision);
        return;
      }
    } catch (error) { if (current === generation) ttsError.value = options.errorText(error); }
    finally { if (current === generation) { advancing = false; ttsExpectedChapterKey.value = null; } }
  }

  async function prepareNext(index: number, current: number): Promise<void> {
    if (selected || !settings.value.reader.ttsContinueAcrossChapters) return;
    const next = readingBook.value?.chapters.find(chapter => !chapter.volume && chapter.index > index);
    if (!next) return;
    try {
      const chunks = (await options.prepareSegments(next.index)).flatMap(mapTtsSegmentChunks);
      if (current !== generation || !chunks.length) return;
      await deliver(next.index, chunks, current);
    } catch (error) { if (current === generation) ttsError.value = options.errorText(error); }
  }

  function setupTts(): Promise<void> {
    ready ??= (async () => {
      if (!isTauri()) return;
      const output = await setupTtsOutput(media => mediaSession?.sync(media));
      const unlisten = await listen<TtsProgress>("tts", event => { void onProgress(event.payload); });
      if (destroyed) { output(); unlisten(); return; }
      cleanup.push(output, unlisten);
    })().catch(error => { ready = undefined; throw error; });
    return ready;
  }

  async function deliver(index: number, chunks: Chunk[], current: number): Promise<void> {
    if (current !== generation || destroyed) return;
    if (!chunks.length) { ttsNotice.value = "没有可朗读的内容，等待继续投递。"; ttsActive.value = true; return; }
    const start = nextIndex;
    nextIndex += chunks.length;
    chapters.push({ index, start, targets: chunks.map(chunk => chunk.target), revision: options.reader.getLoadRevision() });
    const finished = selected || !settings.value.reader.ttsContinueAcrossChapters
      || !readingBook.value?.chapters.some(chapter => !chapter.volume && chapter.index > index);
    await submitTts(chunks.map((chunk, index) => ({ text: chunk.text, index: start + index })), finished);
  }

  async function startTts(text?: string): Promise<void> {
    if (!ttsCanStart.value) return;
    const current = ++generation;
    ttsActive.value = true;
    ttsBuffering.value = true;
    ttsError.value = ttsNotice.value = "";
    chapters = [];
    nextIndex = 0;
    latestIndex = 0;
    advancing = false;
    selected = Boolean(text?.trim());
    try {
      await setupTts();
      if (current !== generation) return;
      const reader = settings.value.reader;
      initialRate = reader.ttsRate;
      ttsPaused.value = false;
      const book = readingBook.value;
      await beginTts({ configId: reader.ttsEngine === "http" ? reader.httpTtsConfigId ?? null : null,
        voiceId: reader.ttsVoiceURI || null, rate: initialRate });
      if (current !== generation) return;
      mediaSession = openMediaSession({ title: book?.title || "朗读", artist: book?.author,
        artwork: book?.coverSrc ? [{ src: book.coverSrc }] : [] }, {
        play: () => setPaused(false),
        pause: () => setPaused(true),
        stop: () => stopTts(),
      }, error => { ttsError.value = options.errorText(error); });
      await mediaSession.ready;
      if (current !== generation) return;
      const chunks = selected ? splitTtsText(text!.trim())
        : (await options.loadSegments(reader.ttsStartPosition === "current")).flatMap(mapTtsSegmentChunks);
      await deliver(options.reader.chapterIndex.value, chunks, current);
      // 本章刚投递就准备下一章，不等本章读完再请求。
      if (current === generation) void prepareNext(options.reader.chapterIndex.value, current);
    } catch (error) {
      if (current === generation) { ttsError.value = options.errorText(error); await stopTts(false); }
    }
  }

  async function stopTts(showNotice = true): Promise<void> {
    ++generation;
    ttsActive.value = false;
    advancing = false;
    ttsPaused.value = false;
    ttsBuffering.value = false;
    mediaSession?.close(); mediaSession = undefined;
    chapters = [];
    ttsChunkProgress.value = null;
    ttsExpectedChapterKey.value = null;
    options.clearHighlight();
    if (showNotice) ttsNotice.value = "朗读已停止。";
    try { await endTts(); } catch (error) { ttsError.value = options.errorText(error); }
  }

  function selectHttpTtsConfig(id: string): void {
    settings.value.reader.httpTtsConfigId = httpTtsConfigs.value.some(item => item.id === id && item.enabled) ? id : null;
    if (!settings.value.reader.httpTtsConfigId) settings.value.reader.ttsEngine = "system";
  }
  function selectTtsEngine(value: string): void {
    if (value.startsWith("http:")) { selectHttpTtsConfig(value.slice(5)); settings.value.reader.ttsEngine = "http"; }
    else { settings.value.reader.ttsEngine = "system"; settings.value.reader.ttsVoiceURI = value.slice(7); }
  }
  function stopTtsForDisplayChange(): void { if (ttsActive.value && !ttsExpectedChapterKey.value) void stopTts(false); }
  async function setPaused(paused: boolean): Promise<void> {
    if (!ttsActive.value || paused === ttsPaused.value) return;
    const current = generation;
    try {
      await pauseTtsOutput(paused, settings.value.reader.ttsEngine === "http");
      if (current === generation) { ttsPaused.value = paused; mediaSession?.setPaused(paused); }
    } catch (error) { if (current === generation) ttsError.value = options.errorText(error); }
  }
  async function toggleTtsPaused(): Promise<void> { await setPaused(!ttsPaused.value); }
  async function setTtsRate(rate: number): Promise<void> {
    if (!Number.isFinite(rate) || ttsActive.value && settings.value.reader.ttsEngine === "system") return;
    const next = clamp(rate, 0.5, settings.value.reader.ttsEngine === "system" ? 4 : 5);
    try {
      if (ttsActive.value) await setTtsOutputRate(next / initialRate);
      settings.value.reader.ttsRate = next;
    } catch (error) { ttsError.value = options.errorText(error); }
  }
  onBeforeUnmount(() => {
    destroyed = true;
    void stopTts(false);
    cleanup.splice(0).forEach(dispose => dispose());
  });

  return {
    ttsVoices, ttsActive, ttsPaused, ttsBuffering, toggleTtsPaused, ttsSelectedText, ttsError, ttsExpectedChapterKey, ttsCanStart,
    ttsEngineSelection, ttsNotice, ttsChunkProgress,
    setupTts, refreshTtsVoices, selectHttpTtsConfig, selectTtsEngine, startTts, stopTts, stopTtsForDisplayChange,
    setTtsRate,
  };
}
