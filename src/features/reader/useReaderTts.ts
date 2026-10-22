import { computed, onBeforeUnmount, ref, type Ref } from "vue";
import { listen } from "@tauri-apps/api/event";
import { isTauri } from "@tauri-apps/api/core";
import { startTts as beginTts, stopTts as endTts, submitTts, type TtsProgress } from "../../api/tts";
import { openedBook } from "../books/bookDetailState";
import { settings } from "../settings/settingsStore";
import { httpTtsConfigs } from "../settings/configurationStores";
import { clamp, type PdfPageResource } from "./readerContent";
import { mapTtsSegmentChunks, splitTtsText, type ReaderTtsChunkTarget, type ReaderTtsSegment } from "./ttsText";
import { disposeNativeAudio, registerBeforeNativeAudioStart, usesNativeAudio } from "./nativeAudio";
import { setupTtsOutput, ttsVoices as loadVoices, type TtsVoice } from "./ttsOutput";

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
  const nativePlayback = usesNativeAudio();
  const ttsVoices = ref<TtsVoice[]>([]);
  const ttsActive = ref(false);
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
    if (typeof progress === "string") { ttsError.value = progress; return; }
    latestIndex = progress;
    if (advancing) return;
    const chapter = chapters.find(chapter => latestIndex >= chapter.start && latestIndex < chapter.start + chapter.targets.length);
    if (!chapter) return;
    const current = generation;
    try {
      // 音频已提前排好。这里只跟随实际朗读位置翻章，翻章不阻塞播放器。
      if (chapter.index !== options.reader.chapterIndex.value) {
        advancing = true;
        ttsExpectedChapterKey.value = `${openedBook.value?.id}:${chapter.index}`;
        await options.openChapter(chapter.index);
        const visible = (await options.loadSegments(false)).flatMap(mapTtsSegmentChunks);
        if (current !== generation) return;
        chapter.targets = visible.map(chunk => chunk.target);
        chapter.revision = options.reader.getLoadRevision();
        chapters = chapters.filter(entry => entry.start >= chapter.start);
        void prepareNext(chapter.index, current);
      }
      const index = latestIndex - chapter.start;
      ttsChunkProgress.value = { current: index + 1, total: chapter.targets.length };
      options.highlightChunk(chapter.targets[index], chapter.revision);
    } catch (error) { if (current === generation) ttsError.value = options.errorText(error); }
    finally { advancing = false; ttsExpectedChapterKey.value = null; }
  }

  async function prepareNext(index: number, current: number): Promise<void> {
    if (selected || !settings.value.reader.ttsContinueAcrossChapters) return;
    const next = openedBook.value?.chapters.find(chapter => !chapter.volume && chapter.index > index);
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
      const output = await setupTtsOutput();
      const unlisten = await listen<TtsProgress>("tts", event => { void onProgress(event.payload); });
      if (destroyed) { output(); unlisten(); return; }
      cleanup.push(output, unlisten);
      await refreshTtsVoices();
    })().catch(error => { ready = undefined; throw error; });
    return ready;
  }

  async function deliver(index: number, chunks: Chunk[], current: number): Promise<void> {
    if (current !== generation || destroyed) return;
    if (!chunks.length) { ttsNotice.value = "没有可朗读的内容，等待继续投递。"; ttsActive.value = true; return; }
    const start = nextIndex;
    nextIndex += chunks.length;
    chapters.push({ index, start, targets: chunks.map(chunk => chunk.target), revision: options.reader.getLoadRevision() });
    await submitTts(chunks.map((chunk, index) => ({ text: chunk.text, index: start + index })));
  }

  async function startTts(text?: string): Promise<void> {
    if (!ttsCanStart.value) return;
    const current = ++generation;
    ttsActive.value = true;
    ttsError.value = ttsNotice.value = "";
    chapters = [];
    nextIndex = 0;
    selected = Boolean(text?.trim());
    try {
      await setupTts();
      if (nativePlayback) await disposeNativeAudio();
      if (current !== generation) return;
      const reader = settings.value.reader;
      await beginTts({ configId: reader.ttsEngine === "http" ? reader.httpTtsConfigId ?? null : null,
        voiceId: reader.ttsVoiceURI || null, rate: reader.ttsRate });
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
  const unregister = registerBeforeNativeAudioStart(() => stopTts(false));
  onBeforeUnmount(() => {
    destroyed = true;
    unregister();
    void stopTts(false);
    cleanup.splice(0).forEach(dispose => dispose());
  });

  return {
    nativePlayback, ttsVoices, ttsActive, ttsSelectedText, ttsError, ttsExpectedChapterKey, ttsCanStart,
    ttsEngineSelection, ttsNotice, ttsChunkProgress,
    setupTts, refreshTtsVoices, selectHttpTtsConfig, selectTtsEngine, startTts, stopTts, stopTtsForDisplayChange,
    setTtsRate: (rate: number) => { settings.value.reader.ttsRate = clamp(rate, 0.5, 5); },
  };
}
