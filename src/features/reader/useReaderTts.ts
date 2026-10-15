import { openedBook } from "../books/bookDetailState";
import { computed, nextTick, onBeforeUnmount, ref, shallowRef, type ComponentPublicInstance, type Ref } from "vue";
import { getHttpTtsStreamStatus, releaseHttpTtsAudio, releaseHttpTtsStream, requestHttpTtsAudio, startHttpTtsStream } from "../../api/settings";
import { type HttpTtsStreamStatus, type ResourceDescriptor } from "../../api/types";
import { settings } from "../settings/settingsStore";
import { httpTtsConfigs } from "../settings/configurationStores";
import { clamp, type PdfPageResource } from "./readerContent";
import { mapTtsSegmentChunks, splitTtsText, type ReaderTtsChunkTarget, type ReaderTtsSegment } from "./ttsText";
import { NativeReaderAudio, usesNativeAudio, type ReaderMediaElement } from "./nativeAudio";
import { NativeSpeechQueue } from "./nativeSpeech";

type TtsAvailability = "checking" | "ready" | "empty" | "unsupported";
type TtsPlayback = "idle" | "loading" | "speaking" | "paused";

interface ReaderTtsChunkProgress {
  current: number;
  total: number;
}

interface PreparedHttpTtsAudio {
  kind: "audio";
  audioId: string;
  resource: ResourceDescriptor;
}

type HttpTtsChunkAudio = PreparedHttpTtsAudio | { kind: "empty" };

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
  changeChapter: (direction: -1 | 1) => Promise<void>;
  highlightChunk: (target: ReaderTtsChunkTarget | undefined, loadRevision: number) => void;
  clearHighlight: () => void;
  errorText: (error: unknown) => string;
}

/** 管理系统朗读和 HTTP TTS 的分段、播放状态及资源释放。 */
export function useReaderTts(options: UseReaderTtsOptions) {
  const nativePlayback = usesNativeAudio();
  const nativeSpeech = nativePlayback ? new NativeSpeechQueue() : null;
  const ttsAvailability = ref<TtsAvailability>("checking");
  const ttsVoices = ref<SpeechSynthesisVoice[]>([]);
  const ttsPlayback = ref<TtsPlayback>("idle");
  const ttsChunkProgress = ref<ReaderTtsChunkProgress | null>(null);
  const ttsSelectedText = ref("");
  const ttsSelectionPlayback = ref(false);
  const ttsError = ref("");
  const ttsNotice = ref("");
  const ttsExpectedChapterKey = ref<string | null>(null);

  const httpTtsExpectedChapterKey = ref<string | null>(null);
  const httpTtsAudioElement = shallowRef<ReaderMediaElement | null>(null);
  const httpTtsAudioResource = ref<ResourceDescriptor | null>(null);
  const httpTtsAudioBusy = ref(false);
  const httpTtsAudioActive = ref(false);
  const httpTtsAudioPaused = ref(false);
  const httpTtsAudioError = ref("");
  const httpTtsAudioNotice = ref("");
  const httpTtsChunkProgress = ref<ReaderTtsChunkProgress | null>(null);

  let ttsGeneration = 0;
  let httpTtsGeneration = 0;
  let httpTtsPrefetchGeneration = 0;
  let httpTtsAudioId: string | null = null;
  let httpTtsStreamId: string | null = null;
  let httpTtsStreamStatus: HttpTtsStreamStatus | null = null;
  let httpTtsStreamStartSkipped = 0;
  let httpTtsStreamPollAt = 0;
  let httpTtsStreamPollPending: string | null = null;
  let endingHttpTtsResource: ResourceDescriptor | null = null;
  let httpTtsLoadRevision = 0;
  let httpTtsChunks: Array<{ text: string; target: ReaderTtsChunkTarget }> = [];
  let httpTtsChunkIndex = 0;
  const httpTtsPreparedAudio = new Map<number, HttpTtsChunkAudio>();
  const httpTtsPrefetchTasks = new Map<number, Promise<HttpTtsChunkAudio | null>>();
  let httpTtsSkippedChunks = 0;
  let ttsVoiceLoadGeneration = 0;
  let ttsChunks: string[] = [];
  let ttsChunkTargets: ReaderTtsChunkTarget[] = [];
  let ttsChunkIndex = 0;
  let ttsVoicesChangedListener: (() => void) | null = null;
  let ttsVoicesLoadingTimer: ReturnType<typeof setTimeout> | null = null;

  function setHttpTtsAudioElement(element: Element | ComponentPublicInstance | null): void {
    httpTtsAudioElement.value = element instanceof HTMLAudioElement ? element : null;
  }

  const ttsCanStart = computed(() => options.reader.visible.value
    && ttsAvailability.value === "ready"
    && ttsPlayback.value === "idle"
    && !options.reader.busy.value
    && !options.reader.mediaChapter.value
    && !options.reader.pdfPage.value
    && !options.reader.imagePage.value);

  const ttsEngineSelection = computed(() => settings.value.reader.ttsEngine === "http"
    ? `http:${settings.value.reader.httpTtsConfigId ?? ""}`
    : `system:${settings.value.reader.ttsVoiceURI}`);
  const ttsControlPlayback = computed(() => settings.value.reader.ttsEngine === "http"
    ? httpTtsAudioBusy.value ? "loading" : httpTtsAudioActive.value ? httpTtsAudioPaused.value ? "paused" : "speaking" : "idle"
    : ttsPlayback.value);
  const ttsControlCanStart = computed(() => settings.value.reader.ttsEngine === "http"
    ? options.reader.visible.value
      && !!openedBook.value
      && !!settings.value.reader.httpTtsConfigId
      && httpTtsConfigs.value.some((item) => item.id === settings.value.reader.httpTtsConfigId && item.enabled)
      && !options.reader.busy.value
      && !options.reader.mediaChapter.value
      && !options.reader.pdfPage.value
      && !options.reader.imagePage.value
      && !httpTtsAudioBusy.value
      && !httpTtsAudioActive.value
    : ttsCanStart.value);
  const ttsControlStatus = computed(() => {
    if (settings.value.reader.ttsEngine !== "http") return ttsStatusMessage.value;
    if (!settings.value.reader.httpTtsConfigId) return "请选择网络朗读引擎，或先添加一个配置。";
    if (httpTtsAudioBusy.value) return httpTtsAudioNotice.value || "正在准备网络朗读…";
    if (httpTtsAudioPaused.value) return "网络朗读已暂停。";
    return httpTtsAudioNotice.value;
  });
  const ttsControlError = computed(() => settings.value.reader.ttsEngine === "http"
    ? httpTtsAudioError.value
    : ttsError.value);
  const ttsControlProgress = computed(() => settings.value.reader.ttsEngine === "http"
    ? httpTtsChunkProgress.value
    : ttsChunkProgress.value);

  const ttsStatusMessage = computed(() => {
    if (ttsPlayback.value === "loading") return "正在准备朗读内容…";
    if (ttsPlayback.value === "speaking") return ttsNotice.value || "正在朗读本章";
    if (ttsPlayback.value === "paused") return ttsNotice.value || "朗读已暂停";
    if (ttsAvailability.value === "unsupported") return "当前 WebView 不支持系统朗读。";
    if (ttsAvailability.value === "checking") return "正在检查系统语音…";
    if (ttsAvailability.value === "empty") return "尚未检测到系统语音。可以重新检测；若仍为空，请在设备中启用或安装语音服务。";
    return ttsNotice.value || "选择语音和速度后即可朗读。";
  });

  function refreshTtsVoices(): void {
    if (nativeSpeech) { void loadNativeVoices(++ttsVoiceLoadGeneration); return; }
    if (ttsAvailability.value === "unsupported" || typeof window === "undefined" || !("speechSynthesis" in window)) return;
    try {
      const voices = window.speechSynthesis.getVoices();
      if (voices.length) {
        ttsVoices.value = voices;
        ttsAvailability.value = "ready";
        ttsError.value = "";
        ttsVoiceLoadGeneration += 1;
        if (ttsVoicesLoadingTimer) clearTimeout(ttsVoicesLoadingTimer);
        ttsVoicesLoadingTimer = null;
        return;
      }
      // 部分 WebView 刷新语音列表时会短暂返回空数组，保留上次可用的语音。
      if (ttsVoices.value.length) return;
      if (ttsAvailability.value !== "checking") ttsAvailability.value = "empty";
    } catch (error) {
      ttsAvailability.value = "empty";
      ttsError.value = `读取系统语音失败：${options.errorText(error)}。可以重新检测，或检查设备的语音服务。`;
      if (ttsVoicesLoadingTimer) clearTimeout(ttsVoicesLoadingTimer);
      ttsVoicesLoadingTimer = null;
    }
  }

  function setupTts(): void {
    const generation = ++ttsVoiceLoadGeneration;
    if (nativeSpeech) {
      ttsAvailability.value = "checking";
      if (settings.value.reader.ttsEngine === "system") settings.value.reader.ttsRate = Math.min(4, settings.value.reader.ttsRate);
      void loadNativeVoices(generation);
      return;
    }
    if (ttsVoicesLoadingTimer) clearTimeout(ttsVoicesLoadingTimer);
    ttsVoicesLoadingTimer = null;
    if (ttsVoicesChangedListener && typeof window !== "undefined" && "speechSynthesis" in window) {
      try {
        window.speechSynthesis.removeEventListener("voiceschanged", ttsVoicesChangedListener);
      } catch {
        // 宿主语音服务替换期间，移除旧监听失败不应阻止重新检测。
      }
    }
    ttsVoicesChangedListener = null;
    ttsVoices.value = [];
    ttsError.value = "";
    if (typeof window === "undefined" || !("speechSynthesis" in window) || typeof SpeechSynthesisUtterance === "undefined") {
      ttsAvailability.value = "unsupported";
      ttsError.value = "此运行环境没有系统朗读接口。请在支持系统语音的 WebView 中打开；若宿主能力刚发生变化，可重新检测。";
      return;
    }

    ttsAvailability.value = "checking";
    try {
      const synthesis = window.speechSynthesis;
      ttsVoicesChangedListener = () => refreshTtsVoices();
      synthesis.addEventListener("voiceschanged", ttsVoicesChangedListener);
      refreshTtsVoices();
    } catch (error) {
      ttsAvailability.value = "empty";
      ttsError.value = `无法连接系统语音服务：${options.errorText(error)}。请检查设备语音服务后重新检测。`;
      return;
    }
    if (ttsAvailability.value === "checking") {
      ttsVoicesLoadingTimer = setTimeout(() => {
        if (generation !== ttsVoiceLoadGeneration) return;
        ttsVoicesLoadingTimer = null;
        if (ttsVoices.value.length) return;
        ttsAvailability.value = "empty";
        ttsError.value = "系统语音仍未加载。请启用设备的文字转语音服务或安装语音包，然后重新检测。";
      }, 5000);
    }
  }

  async function loadNativeVoices(generation: number): Promise<void> {
    try {
      const voices = await nativeSpeech!.voices();
      if (generation !== ttsVoiceLoadGeneration) return;
      ttsVoices.value = voices.map(voice => ({ voiceURI: voice.id, name: voice.name,
        lang: voice.language, localService: true, default: false }));
      ttsAvailability.value = voices.length ? "ready" : "empty";
      ttsError.value = voices.length ? "" : "没有检测到系统语音，请安装或启用设备的语音包。";
    } catch (error) {
      if (generation !== ttsVoiceLoadGeneration) return;
      ttsAvailability.value = "empty";
      ttsError.value = nativeError(error);
    }
  }
  function nativeError(error: unknown): string {
    return typeof error === "object" && error !== null && "message" in error
      ? String(error.message) : options.errorText(error);
  }
  function retryTtsAvailability(): void {
    ttsNotice.value = "";
    setupTts();
  }

  function selectTtsVoice(value: string): void {
    ttsError.value = "";
    settings.value.reader.ttsVoiceURI = value;
    ttsNotice.value = nativeSpeech ? "新语音将在下次开始朗读时使用。"
      : ttsPlayback.value === "speaking" || ttsPlayback.value === "paused"
      ? "声音将在当前片段结束后切换；暂停中的片段会按原语音继续。"
      : "语音偏好已更新。";
  }

  function setTtsRate(value: number): void {
    ttsError.value = "";
    settings.value.reader.ttsRate = clamp(value, 0.5, nativeSpeech && settings.value.reader.ttsEngine === "system" ? 4 : 5);
  }

  function selectHttpTtsConfig(configId: string): void {
    const selected = configId && httpTtsConfigs.value.some((item) => item.id === configId && item.enabled)
      ? configId
      : null;
    settings.value.reader.httpTtsConfigId = selected;
    if (!selected && settings.value.reader.ttsEngine === "http") {
      settings.value.reader.ttsEngine = "system";
    }
    httpTtsAudioError.value = "";
  }

  function selectTtsEngine(value: string): void {
    if (value.startsWith("http:")) {
      const configId = value.slice("http:".length);
      if (!configId) return;
      selectHttpTtsConfig(configId);
      settings.value.reader.ttsEngine = "http";
      return;
    }
    if (value.startsWith("system:")) {
      settings.value.reader.ttsEngine = "system";
      if (nativeSpeech) settings.value.reader.ttsRate = Math.min(4, settings.value.reader.ttsRate);
      selectTtsVoice(value.slice("system:".length));
    }
  }

  function startSelectedTts(selectedText?: string): void {
    if (settings.value.reader.ttsEngine === "http") {
      void startHttpTtsAudio();
    } else {
      void startTts(selectedText);
    }
  }

  function pauseSelectedTts(): void {
    if (settings.value.reader.ttsEngine === "http") pauseHttpTtsAudio();
    else pauseTts();
  }

  function resumeSelectedTts(): void {
    if (settings.value.reader.ttsEngine === "http") void resumeHttpTtsAudio();
    else resumeTts();
  }

  function stopSelectedTts(): void {
    if (settings.value.reader.ttsEngine === "http") stopHttpTtsAudio(true);
    else stopTts();
  }

  function stopTts(showNotice = true): void {
    ttsGeneration += 1;
    ttsExpectedChapterKey.value = null;
    ttsSelectionPlayback.value = false;
    ttsChunkProgress.value = null;
    ttsChunks = [];
    ttsChunkTargets = [];
    ttsChunkIndex = 0;
    options.clearHighlight();
    if (nativeSpeech) void nativeSpeech.stop().catch(error => console.debug("Native speech stop failed:", error));
    if (!nativeSpeech && typeof window !== "undefined" && "speechSynthesis" in window) {
      try {
        window.speechSynthesis.cancel();
      } catch {
        // WebView 销毁语音服务时，停止操作允许失败。
      }
    }
    ttsPlayback.value = "idle";
    if (showNotice) ttsNotice.value = "朗读已停止。";
  }

  function stopTtsForDisplayChange(): void {
    if (ttsPlayback.value === "idle") return;
    stopTts(false);
    ttsError.value = "";
    ttsNotice.value = "阅读显示已更新，朗读已停止。请重新开始。";
  }

  function setTtsSegments(segments: ReaderTtsSegment[]): void {
    ttsChunks = [];
    ttsChunkTargets = [];
    for (const segment of segments) {
      for (const chunk of mapTtsSegmentChunks(segment)) {
        ttsChunks.push(chunk.text);
        ttsChunkTargets.push(chunk.target);
      }
    }
  }

  async function collectHttpTtsChunks(fromCurrentPosition: boolean): Promise<Array<{ text: string; target: ReaderTtsChunkTarget }>> {
    const segments = await options.loadSegments(fromCurrentPosition);
    return segments.flatMap(mapTtsSegmentChunks);
  }

  async function startTts(selectedText?: string): Promise<void> {
    if (!ttsCanStart.value) return;
    stopHttpTtsAudio(false);
    const selected = selectedText?.trim() ?? "";
    ttsSelectedText.value = "";
    ttsSelectionPlayback.value = Boolean(selected);
    ttsChunkProgress.value = null;
    const generation = ++ttsGeneration;
    ttsExpectedChapterKey.value = null;
    options.clearHighlight();
    ttsError.value = "";
    ttsNotice.value = "";
    ttsPlayback.value = "loading";
    try {
      if (selected) {
        ttsChunks = splitTtsText(selected).map((chunk) => chunk.text);
        ttsChunkTargets = ttsChunks.map(() => ({ target: null, startNode: null, startOffset: 0, endNode: null, endOffset: 0 }));
      } else {
        const segments = await options.loadSegments(settings.value.reader.ttsStartPosition === "current");
        if (generation !== ttsGeneration) return;
        setTtsSegments(segments);
      }
      if (generation !== ttsGeneration) return;
      ttsChunkIndex = 0;
      if (!ttsChunks.length) throw new Error("当前章节没有可朗读的正文。");
      speakTtsChunk(generation);
    } catch (error) {
      if (generation !== ttsGeneration) return;
      ttsSelectionPlayback.value = false;
      ttsChunkProgress.value = null;
      ttsPlayback.value = "idle";
      ttsChunks = [];
      ttsChunkTargets = [];
      options.clearHighlight();
      ttsError.value = options.errorText(error);
    }
  }

  function speakTtsChunk(generation: number): void {
    if (generation !== ttsGeneration) return;
    const text = ttsChunks[ttsChunkIndex];
    if (!text) {
      options.clearHighlight();
      ttsChunkProgress.value = null;
      void advanceTtsChapter(generation);
      return;
    }
    ttsChunkProgress.value = { current: ttsChunkIndex + 1, total: ttsChunks.length };
    const target = ttsChunkTargets[ttsChunkIndex];
    const loadRevision = options.reader.getLoadRevision();
    if (nativeSpeech) {
      const fail = (error: unknown) => {
        if (generation !== ttsGeneration) return;
        ttsPlayback.value = "idle"; ttsSelectionPlayback.value = false; ttsChunkProgress.value = null;
        options.clearHighlight(); ttsError.value = nativeError(error);
      };
      void nativeSpeech.start([...ttsChunks], { voiceId: settings.value.reader.ttsVoiceURI,
        rate: settings.value.reader.ttsRate }, {
        start: index => {
          if (generation !== ttsGeneration || loadRevision !== options.reader.getLoadRevision()) return;
          ttsChunkIndex = index; ttsPlayback.value = "speaking";
          ttsChunkProgress.value = { current: index + 1, total: ttsChunks.length };
          options.highlightChunk(ttsChunkTargets[index], loadRevision);
        },
        paused: () => { if (generation === ttsGeneration) ttsPlayback.value = "paused"; },
        resumed: () => { if (generation === ttsGeneration) ttsPlayback.value = "speaking"; },
        finished: () => {
          if (generation !== ttsGeneration) return;
          ttsChunkIndex = ttsChunks.length; ttsChunkProgress.value = null; options.clearHighlight();
          void advanceTtsChapter(generation);
        },
        error: fail,
        notice: message => { if (generation === ttsGeneration) ttsNotice.value = message; },
      }, ttsChunkIndex).catch(fail);
      return;
    }
    try {
      const utterance = new SpeechSynthesisUtterance(text);
      const selectedVoice = ttsVoices.value.find((voice) => voice.voiceURI === settings.value.reader.ttsVoiceURI);
      if (ttsNotice.value.startsWith("声音将在当前片段结束后切换")) ttsNotice.value = "";
      if (selectedVoice) utterance.voice = selectedVoice;
      else if (settings.value.reader.ttsVoiceURI) ttsNotice.value = "已保存的语音当前不可用，正在使用系统默认语音。";
      utterance.rate = settings.value.reader.ttsRate;
      utterance.onstart = () => {
        if (generation !== ttsGeneration || loadRevision !== options.reader.getLoadRevision()) return;
        options.highlightChunk(target, loadRevision);
        ttsPlayback.value = "speaking";
      };
      utterance.onpause = () => {
        if (generation === ttsGeneration && loadRevision === options.reader.getLoadRevision()) ttsPlayback.value = "paused";
      };
      utterance.onresume = () => {
        if (generation === ttsGeneration && loadRevision === options.reader.getLoadRevision()) ttsPlayback.value = "speaking";
      };
      utterance.onend = () => {
        if (generation !== ttsGeneration) return;
        options.clearHighlight();
        ttsChunkIndex += 1;
        speakTtsChunk(generation);
      };
      utterance.onerror = (event) => {
        if (generation !== ttsGeneration) return;
        ttsSelectionPlayback.value = false;
        ttsChunkProgress.value = null;
        ttsPlayback.value = "idle";
        ttsChunks = [];
        ttsChunkTargets = [];
        options.clearHighlight();
        ttsError.value = event.error === "not-allowed"
          ? "系统阻止了朗读，请通过点击“开始朗读”重新启动。"
          : `朗读中断：${event.error || "语音服务发生错误"}`;
      };
      ttsPlayback.value = "speaking";
      window.speechSynthesis.speak(utterance);
    } catch (error) {
      if (generation !== ttsGeneration) return;
      ttsSelectionPlayback.value = false;
      ttsChunkProgress.value = null;
      ttsPlayback.value = "idle";
      ttsChunks = [];
      ttsChunkTargets = [];
      options.clearHighlight();
      ttsError.value = `启动朗读失败：${options.errorText(error)}`;
    }
  }

  async function advanceTtsChapter(generation: number): Promise<void> {
    if (generation !== ttsGeneration) return;
    if (ttsSelectionPlayback.value) {
      ttsSelectionPlayback.value = false;
      ttsPlayback.value = "idle";
      ttsChunks = [];
      ttsChunkTargets = [];
      options.clearHighlight();
      ttsNotice.value = "选中文字朗读已完成。";
      return;
    }
    if (!settings.value.reader.ttsContinueAcrossChapters) {
      ttsSelectionPlayback.value = false;
      ttsExpectedChapterKey.value = null;
      ttsPlayback.value = "idle";
      ttsChunks = [];
      ttsChunkTargets = [];
      options.clearHighlight();
      ttsNotice.value = "本章朗读已完成。";
      return;
    }
    const book = openedBook.value;
    if (!book) {
      stopTts(false);
      return;
    }
    const nextIndex = options.reader.chapterIndex.value + 1;
    const chapterCount = book.chapterCount ?? book.chapters.length;
    if (nextIndex >= chapterCount) {
      ttsSelectionPlayback.value = false;
      ttsPlayback.value = "idle";
      ttsChunks = [];
      ttsChunkTargets = [];
      options.clearHighlight();
      ttsNotice.value = "已朗读至最后一章。";
      return;
    }

    ttsPlayback.value = "loading";
    ttsExpectedChapterKey.value = `${book.id}:${nextIndex}`;
    options.clearHighlight();
    try {
      await options.changeChapter(1);
      if (generation !== ttsGeneration) return;
      if (openedBook.value?.id !== book.id || options.reader.chapterIndex.value !== nextIndex) {
        throw new Error("下一章未能打开，朗读已停止。");
      }
      const segments = await options.loadSegments();
      if (generation !== ttsGeneration) return;
      if (!settings.value.reader.ttsContinueAcrossChapters) {
        ttsSelectionPlayback.value = false;
        ttsExpectedChapterKey.value = null;
        ttsPlayback.value = "idle";
        ttsChunks = [];
        ttsChunkTargets = [];
        options.clearHighlight();
        ttsNotice.value = "连续朗读已关闭。";
        return;
      }
      setTtsSegments(segments);
      ttsChunkIndex = 0;
      if (!ttsChunks.length) throw new Error("下一章没有可朗读的正文。");
      speakTtsChunk(generation);
    } catch (error) {
      if (generation !== ttsGeneration) return;
      ttsSelectionPlayback.value = false;
      ttsExpectedChapterKey.value = null;
      ttsPlayback.value = "idle";
      ttsChunks = [];
      ttsChunkTargets = [];
      options.clearHighlight();
      ttsError.value = `准备下一章朗读失败：${options.errorText(error)}`;
    }
  }

  function pauseTts(): void {
    if (ttsPlayback.value !== "speaking") return;
    if (nativeSpeech) {
      void nativeSpeech.pause().catch(error => { ttsError.value = nativeError(error); });
      return;
    }
    try {
      const synthesis = window.speechSynthesis;
      const generation = ttsGeneration;
      synthesis.pause();
      ttsError.value = "";
      ttsPlayback.value = "paused";
      setTimeout(() => {
        if (generation !== ttsGeneration || ttsPlayback.value !== "paused" || synthesis.paused) return;
        ttsPlayback.value = "speaking";
        ttsError.value = "系统没有确认暂停，朗读可能仍在继续；请稍后重试或点击停止。";
      }, 250);
    } catch (error) {
      ttsError.value = `暂停朗读失败：${options.errorText(error)}`;
    }
  }

  function resumeTts(): void {
    if (ttsPlayback.value !== "paused") return;
    if (nativeSpeech) {
      void nativeSpeech.resume().catch(error => { ttsError.value = nativeError(error); });
      return;
    }
    try {
      const synthesis = window.speechSynthesis;
      const generation = ttsGeneration;
      synthesis.resume();
      ttsError.value = "";
      ttsPlayback.value = "speaking";
      setTimeout(() => {
        if (generation !== ttsGeneration || ttsPlayback.value !== "speaking" || !synthesis.paused) return;
        ttsPlayback.value = "paused";
        ttsError.value = "系统语音仍处于暂停状态。请重试继续，或停止后重新开始朗读。";
      }, 250);
    } catch (error) {
      ttsError.value = `继续朗读失败：${options.errorText(error)}`;
    }
  }

  function releaseCurrentHttpTtsAudio(): void {
    const audioId = httpTtsAudioId;
    const streamId = httpTtsStreamId;
    httpTtsAudioId = null;
    httpTtsStreamId = null;
    httpTtsStreamStatus = null;
    httpTtsStreamStartSkipped = 0;
    httpTtsStreamPollPending = null;
    httpTtsStreamPollAt = 0;
    httpTtsAudioResource.value = null;
    const audio = httpTtsAudioElement.value;
    if (audio instanceof NativeReaderAudio) {
      audio.dispose(); httpTtsAudioElement.value = null;
    } else if (audio) {
      audio.pause();
      audio.removeAttribute("src");
      audio.load();
    }
    if (audioId) void releaseHttpTtsAudio(audioId).catch(() => undefined);
    if (streamId) void releaseHttpTtsStream(streamId).catch(() => undefined);
  }

  function clearHttpTtsPrefetch(): void {
    httpTtsPrefetchGeneration += 1;
    for (const prepared of httpTtsPreparedAudio.values()) {
      if (prepared.kind === "audio") void releaseHttpTtsAudio(prepared.audioId).catch(() => undefined);
    }
    httpTtsPreparedAudio.clear();
    // 正在生成的请求无法取消；代次变化后会在返回时立即释放结果。
    httpTtsPrefetchTasks.clear();
  }

  function requestHttpTtsChunkAudio(index: number, configId: string): Promise<HttpTtsChunkAudio> {
    const chunk = httpTtsChunks[index];
    if (!chunk) return Promise.reject(new Error("朗读段落已失效。"));
    const legacyRate = Math.round(clamp(settings.value.reader.ttsRate * 10 - 5, 0, 45));
    return requestHttpTtsAudio(configId, chunk.text, legacyRate).then((result) => result.status === "empty"
      ? { kind: "empty" }
      : { kind: "audio", audioId: result.audioId, resource: result.resource });
  }

  function prefetchFollowingHttpTtsChunks(generation: number): void {
    const configId = settings.value.reader.httpTtsConfigId;
    if (generation !== httpTtsGeneration || !httpTtsAudioActive.value || !configId) return;
    const prefetchGeneration = httpTtsPrefetchGeneration;
    const lastIndex = Math.min(httpTtsChunks.length - 1, httpTtsChunkIndex + 2);
    for (let index = httpTtsChunkIndex + 1; index <= lastIndex; index += 1) {
      if (httpTtsPreparedAudio.has(index) || httpTtsPrefetchTasks.has(index)) continue;
      const task = requestHttpTtsChunkAudio(index, configId)
        .then((prepared) => {
          const stillNeeded = index >= httpTtsChunkIndex && index <= httpTtsChunkIndex + 2;
          if (generation !== httpTtsGeneration || prefetchGeneration !== httpTtsPrefetchGeneration
            || !httpTtsAudioActive.value || !stillNeeded
            || settings.value.reader.httpTtsConfigId !== configId) {
            if (prepared.kind === "audio") void releaseHttpTtsAudio(prepared.audioId).catch(() => undefined);
            return null;
          }
          httpTtsPreparedAudio.set(index, prepared);
          return prepared;
        })
        .catch(() => null);
      httpTtsPrefetchTasks.set(index, task);
      void task.then(() => {
        if (httpTtsPrefetchTasks.get(index) === task) httpTtsPrefetchTasks.delete(index);
      });
    }
  }

  function stopHttpTtsAudio(showNotice = false): void {
    const wasActive = httpTtsAudioActive.value || httpTtsAudioBusy.value;
    httpTtsGeneration += 1;
    httpTtsExpectedChapterKey.value = null;
    httpTtsAudioBusy.value = false;
    httpTtsAudioActive.value = false;
    httpTtsAudioPaused.value = false;
    httpTtsChunkProgress.value = null;
    httpTtsChunks = [];
    httpTtsChunkIndex = 0;
    releaseCurrentHttpTtsAudio();
    clearHttpTtsPrefetch();
    if (wasActive) options.clearHighlight();
    if (showNotice && wasActive) httpTtsAudioNotice.value = "远程朗读已停止。";
  }

  function pauseHttpTtsAudio(): void {
    if (!httpTtsAudioActive.value || httpTtsAudioBusy.value || httpTtsAudioPaused.value) return;
    const audio = httpTtsAudioElement.value;
    if (!audio) return;
    audio.pause();
    httpTtsAudioPaused.value = true;
    httpTtsAudioNotice.value = "网络朗读已暂停。";
  }

  async function resumeHttpTtsAudio(): Promise<void> {
    if (!httpTtsAudioActive.value || httpTtsAudioBusy.value || !httpTtsAudioPaused.value) return;
    const audio = httpTtsAudioElement.value;
    if (!audio) {
      httpTtsAudioError.value = "当前语音资源尚未就绪，请重新开始朗读。";
      return;
    }
    try {
      const generation = httpTtsGeneration;
      await audio.play();
      if (generation !== httpTtsGeneration || httpTtsAudioElement.value !== audio) return;
      httpTtsAudioPaused.value = false;
      httpTtsAudioError.value = "";
      httpTtsAudioNotice.value = "网络朗读已恢复。";
    } catch (error) {
      httpTtsAudioError.value = `继续网络朗读失败：${options.errorText(error)}`;
    }
  }

  async function startHttpTtsAudio(): Promise<void> {
    const configId = settings.value.reader.httpTtsConfigId;
    if (!options.reader.visible.value || !openedBook.value || !configId || options.reader.busy.value
      || options.reader.mediaChapter.value || options.reader.pdfPage.value || options.reader.imagePage.value) {
      httpTtsAudioError.value = configId ? "当前章节暂不支持远程朗读。" : "请先在设置中选择 HTTP TTS 配置。";
      return;
    }
    stopTts(false);
    stopHttpTtsAudio(false);
    const generation = ++httpTtsGeneration;
    httpTtsAudioError.value = "";
    httpTtsAudioNotice.value = "正在准备本章朗读内容…";
    httpTtsAudioPaused.value = false;
    httpTtsAudioBusy.value = true;
    httpTtsAudioActive.value = true;
    httpTtsSkippedChunks = 0;
    httpTtsLoadRevision = options.reader.getLoadRevision();
    try {
      httpTtsChunks = await collectHttpTtsChunks(settings.value.reader.ttsStartPosition === "current");
      if (generation !== httpTtsGeneration) return;
      if (!httpTtsChunks.length) throw new Error("当前章节没有可朗读的正文。");
      httpTtsChunkIndex = 0;
      httpTtsAudioNotice.value = "正在生成语音…";
      await startHttpTtsChapterPlayback(generation);
    } catch (error) {
      if (generation !== httpTtsGeneration) return;
      stopHttpTtsAudio(false);
      httpTtsAudioError.value = `远程朗读失败：${options.errorText(error)}`;
    } finally {
      if (generation === httpTtsGeneration) httpTtsAudioBusy.value = false;
    }
  }

  async function startHttpTtsChapterPlayback(generation: number): Promise<void> {
    if (generation !== httpTtsGeneration || !httpTtsAudioActive.value) return;
    const configId = settings.value.reader.httpTtsConfigId;
    if (!configId) return;
    if (httpTtsChunks.length > 2048
      || httpTtsChunks.reduce((size, chunk) => size + chunk.text.length * 3, 0) > 8 * 1024 * 1024) {
      await requestNextHttpTtsChunk(generation);
      return;
    }
    const legacyRate = Math.round(clamp(settings.value.reader.ttsRate * 10 - 5, 0, 45));
    // DOM nodes stay in Vue; Rust carries their stable chapter-local indices with the audio.
    const result = await startHttpTtsStream(configId,
      httpTtsChunks.map((chunk, highlightIndex) => ({ text: chunk.text, highlightIndex })), legacyRate);
    if (generation !== httpTtsGeneration || !httpTtsAudioActive.value) {
      if (result.status === "stream") void releaseHttpTtsStream(result.streamId).catch(() => undefined);
      if (result.status === "fallback" && result.first.status === "ready") void releaseHttpTtsAudio(result.first.audioId).catch(() => undefined);
      return;
    }
    if (result.status === "empty") {
      httpTtsSkippedChunks += result.skipped;
      await advanceHttpTtsChapter(generation);
      return;
    }
    httpTtsChunkIndex = result.firstIndex;
    httpTtsSkippedChunks += result.firstIndex;
    if (result.status === "fallback") {
      if (result.first.status === "ready") {
        httpTtsPreparedAudio.set(result.firstIndex, { kind: "audio", audioId: result.first.audioId, resource: result.first.resource });
      }
      await requestNextHttpTtsChunk(generation);
      return;
    }
    httpTtsStreamId = result.streamId;
    httpTtsStreamStartSkipped = result.skipped;
    httpTtsChunkProgress.value = { current: result.firstIndex + 1, total: httpTtsChunks.length };
    httpTtsAudioResource.value = { resourceId: `resource://tts-stream/${result.streamId}.mp3`,
      src: result.src, contentType: "audio/mpeg", format: "audio" };
    httpTtsAudioBusy.value = false;
    httpTtsAudioNotice.value = "正在连续播放本章语音。";
    await playHttpTtsResource(generation);
  }

  async function playHttpTtsResource(generation: number): Promise<void> {
    const resource = httpTtsAudioResource.value;
    if (!resource) return;
    if (nativePlayback) {
      const book = openedBook.value;
      const nativeAudio = new NativeReaderAudio({ key: `tts:${book?.id}:${options.reader.chapterIndex.value}:${httpTtsStreamId ?? httpTtsChunkIndex}`,
        title: book?.chapters.find(chapter => chapter.index === options.reader.chapterIndex.value)?.title || book?.title || "朗读",
        artist: book?.author, artworkUrl: book?.coverSrc });
      nativeAudio.src = resource.src;
      const current = () => generation === httpTtsGeneration && httpTtsAudioElement.value === nativeAudio;
      nativeAudio.addEventListener("play", () => { if (current()) onHttpTtsAudioPlay(); });
      nativeAudio.addEventListener("pause", () => { if (current()) onHttpTtsAudioPause(); });
      nativeAudio.addEventListener("timeupdate", () => { if (current()) void onHttpTtsAudioTimeUpdate(); });
      nativeAudio.addEventListener("ended", () => { if (current()) void onHttpTtsAudioEnded(); });
      nativeAudio.addEventListener("error", () => { if (current()) onHttpTtsAudioError(); });
      httpTtsAudioElement.value = nativeAudio;
    }
    await nextTick();
    if (generation !== httpTtsGeneration || !httpTtsAudioActive.value) return;
    const audio = httpTtsAudioElement.value;
    if (!audio) {
      httpTtsAudioPaused.value = true;
      httpTtsAudioNotice.value = "语音已准备，请点击继续朗读。";
      return;
    }
    audio.load();
    try {
      await audio.play();
    } catch {
      if (generation !== httpTtsGeneration || httpTtsAudioElement.value !== audio) return;
      httpTtsAudioPaused.value = true;
      httpTtsAudioNotice.value = "语音已准备，请点击继续朗读或使用播放器播放。";
    }
  }

  async function onHttpTtsAudioTimeUpdate(): Promise<void> {
    const streamId = httpTtsStreamId;
    const audio = httpTtsAudioElement.value;
    if (!streamId || !audio || !httpTtsAudioActive.value) return;
    const now = Date.now();
    if (!httpTtsStreamPollPending && now - httpTtsStreamPollAt >= 750) {
      httpTtsStreamPollAt = now;
      httpTtsStreamPollPending = streamId;
      try {
        const status = await getHttpTtsStreamStatus(streamId);
        if (httpTtsStreamId === streamId) httpTtsStreamStatus = status;
      } catch { /* A stopped stream is handled by its playback event. */ }
      finally { if (httpTtsStreamPollPending === streamId) httpTtsStreamPollPending = null; }
    }
    if (httpTtsStreamId !== streamId || !httpTtsStreamStatus) return;
    const position = Math.max(0, audio.currentTime);
    const boundary = httpTtsStreamStatus.boundaries.find(item => position < item.endSeconds)
      ?? httpTtsStreamStatus.boundaries[httpTtsStreamStatus.boundaries.length - 1];
    if (!boundary || boundary.index === httpTtsChunkIndex) return;
    httpTtsChunkIndex = boundary.index;
    httpTtsChunkProgress.value = { current: boundary.index + 1, total: httpTtsChunks.length };
    httpTtsAudioNotice.value = `正在朗读第 ${boundary.index + 1} / ${httpTtsChunks.length} 段。`;
    if (httpTtsLoadRevision === options.reader.getLoadRevision()) {
      options.highlightChunk(httpTtsChunks[boundary.highlightIndex]?.target, httpTtsLoadRevision);
    }
  }

  async function requestNextHttpTtsChunk(generation: number): Promise<void> {
    if (generation !== httpTtsGeneration || !httpTtsAudioActive.value) return;
    const configId = settings.value.reader.httpTtsConfigId;
    if (!configId) return;
    httpTtsAudioBusy.value = true;
    try {
      while (httpTtsChunkIndex < httpTtsChunks.length) {
        if (generation !== httpTtsGeneration || !httpTtsAudioActive.value) return;
        const index = httpTtsChunkIndex;
        const chunk = httpTtsChunks[index];
        if (!chunk) break;
        httpTtsChunkProgress.value = { current: index + 1, total: httpTtsChunks.length };
        httpTtsAudioNotice.value = `正在准备第 ${index + 1} / ${httpTtsChunks.length} 段语音…`;
        let prepared = httpTtsPreparedAudio.get(index);
        if (prepared) {
          httpTtsPreparedAudio.delete(index);
        } else {
          const pending = httpTtsPrefetchTasks.get(index);
          if (pending) {
            prepared = (await pending) ?? undefined;
            if (generation !== httpTtsGeneration || !httpTtsAudioActive.value) return;
            if (prepared) httpTtsPreparedAudio.delete(index);
          }
        }
        if (!prepared) {
          prepared = await requestHttpTtsChunkAudio(index, configId);
          if (generation !== httpTtsGeneration || !httpTtsAudioActive.value) {
            if (prepared.kind === "audio") void releaseHttpTtsAudio(prepared.audioId).catch(() => undefined);
            return;
          }
        }

        if (prepared.kind === "empty") {
          // KMP 对无音频响应使用明确错误；该段跳过，其他错误仍交给错误界面处理。
          httpTtsSkippedChunks += 1;
          httpTtsChunkIndex += 1;
          continue;
        }

        if (generation !== httpTtsGeneration || !httpTtsAudioActive.value) return;
        httpTtsAudioId = prepared.audioId;
        httpTtsAudioResource.value = prepared.resource;
        httpTtsAudioBusy.value = false;
        httpTtsAudioNotice.value = "语音已准备，正在播放。";
        // 当前段开始播放时，并行生成后面两段，切段时直接使用已缓存音频。
        prefetchFollowingHttpTtsChunks(generation);
        await playHttpTtsResource(generation);
        return;
      }
      await advanceHttpTtsChapter(generation);
    } catch (error) {
      if (generation !== httpTtsGeneration) return;
      throw error;
    }
  }

  function onHttpTtsAudioPlay(): void {
    const chunk = httpTtsChunks[httpTtsChunkIndex];
    if (!httpTtsAudioActive.value || !chunk || httpTtsLoadRevision !== options.reader.getLoadRevision()) return;
    httpTtsAudioPaused.value = false;
    httpTtsAudioNotice.value = `正在朗读第 ${httpTtsChunkIndex + 1} / ${httpTtsChunks.length} 段。`;
    options.highlightChunk(chunk.target, httpTtsLoadRevision);
  }

  function onHttpTtsAudioPause(): void {
    const audio = httpTtsAudioElement.value;
    if (!httpTtsAudioActive.value || !audio || audio.ended) return;
    httpTtsAudioPaused.value = true;
    httpTtsAudioNotice.value = "网络朗读已暂停。";
    options.clearHighlight();
  }

  async function onHttpTtsAudioEnded(): Promise<void> {
    const resource = httpTtsAudioResource.value;
    if (!httpTtsAudioActive.value || !resource || endingHttpTtsResource === resource) return;
    endingHttpTtsResource = resource;
    if (httpTtsStreamId) {
      const streamId = httpTtsStreamId;
      const generation = httpTtsGeneration;
      let status: HttpTtsStreamStatus;
      try { status = await getHttpTtsStreamStatus(streamId); }
      catch (error) {
        if (generation === httpTtsGeneration) {
          stopHttpTtsAudio(false);
          httpTtsAudioError.value = `连续朗读状态读取失败：${options.errorText(error)}`;
        }
        return;
      }
      if (generation !== httpTtsGeneration || httpTtsStreamId !== streamId) return;
      httpTtsSkippedChunks += Math.max(0, status.skipped - httpTtsStreamStartSkipped);
      options.clearHighlight();
      releaseCurrentHttpTtsAudio();
      if (status.state === "error") {
        stopHttpTtsAudio(false);
        httpTtsAudioError.value = `远程朗读失败：${status.error ?? "音频生成中断"}`;
      } else if (status.state === "fallback" || status.state === "running") {
        httpTtsChunkIndex = status.state === "fallback" ? status.nextIndex : Math.max(0, httpTtsChunkIndex);
        try { await requestNextHttpTtsChunk(generation); }
        catch (error) {
          if (generation === httpTtsGeneration) {
            stopHttpTtsAudio(false);
            httpTtsAudioError.value = `远程朗读失败：${options.errorText(error)}`;
          }
        }
      } else {
        await advanceHttpTtsChapter(generation);
      }
      return;
    }
    options.clearHighlight();
    releaseCurrentHttpTtsAudio();
    httpTtsChunkIndex += 1;
    if (httpTtsChunkIndex < httpTtsChunks.length) {
      const generation = httpTtsGeneration;
      try {
        await requestNextHttpTtsChunk(generation);
      } catch (error) {
        if (generation === httpTtsGeneration) {
          stopHttpTtsAudio(false);
          httpTtsAudioError.value = `远程朗读失败：${options.errorText(error)}`;
        }
      }
      return;
    }
    await advanceHttpTtsChapter(httpTtsGeneration);
  }

  async function advanceHttpTtsChapter(generation: number): Promise<void> {
    if (generation !== httpTtsGeneration) return;
    if (!settings.value.reader.ttsContinueAcrossChapters) {
      const skippedChunks = httpTtsSkippedChunks;
      stopHttpTtsAudio(false);
      httpTtsAudioNotice.value = skippedChunks > 0
        ? `朗读已完成，跳过 ${skippedChunks} 段无音频内容。`
        : "本章朗读已完成。";
      return;
    }
    const book = openedBook.value;
    if (!book) {
      stopHttpTtsAudio(false);
      return;
    }
    const nextIndex = options.reader.chapterIndex.value + 1;
    if (nextIndex >= (book.chapterCount ?? book.chapters.length)) {
      const skippedChunks = httpTtsSkippedChunks;
      stopHttpTtsAudio(false);
      httpTtsAudioNotice.value = skippedChunks > 0
        ? `已朗读至最后一章，跳过 ${skippedChunks} 段无音频内容。`
        : "已朗读至最后一章。";
      return;
    }
    httpTtsAudioBusy.value = true;
    httpTtsExpectedChapterKey.value = `${book.id}:${nextIndex}`;
    options.clearHighlight();
    try {
      clearHttpTtsPrefetch();
      await options.changeChapter(1);
      if (generation !== httpTtsGeneration) return;
      if (openedBook.value?.id !== book.id || options.reader.chapterIndex.value !== nextIndex) {
        throw new Error("下一章未能打开，远程朗读已停止。");
      }
      httpTtsExpectedChapterKey.value = null;
      httpTtsLoadRevision = options.reader.getLoadRevision();
      httpTtsChunks = await collectHttpTtsChunks(false);
      if (generation !== httpTtsGeneration) return;
      httpTtsChunkIndex = 0;
      if (!httpTtsChunks.length) throw new Error("下一章没有可朗读的正文。");
      await startHttpTtsChapterPlayback(generation);
    } catch (error) {
      if (generation !== httpTtsGeneration) return;
      stopHttpTtsAudio(false);
      httpTtsAudioError.value = options.errorText(error);
    } finally {
      if (generation === httpTtsGeneration) httpTtsAudioBusy.value = false;
    }
  }

  function onHttpTtsAudioError(): void {
    if (!httpTtsAudioResource.value || !httpTtsAudioActive.value) return;
    if (httpTtsStreamId) {
      const generation = httpTtsGeneration;
      releaseCurrentHttpTtsAudio();
      void requestNextHttpTtsChunk(generation).catch(error => {
        if (generation === httpTtsGeneration) {
          stopHttpTtsAudio(false);
          httpTtsAudioError.value = `连续播放不可用：${options.errorText(error)}`;
        }
      });
      return;
    }
    stopHttpTtsAudio(false);
    httpTtsAudioError.value = "远程语音资源无法播放，请重新生成。";
  }

  onBeforeUnmount(() => {
    stopTts(false);
    stopHttpTtsAudio(false);
    if (nativeSpeech) void nativeSpeech.dispose().catch(error => console.debug("Native speech disposal failed:", error));
    ttsVoiceLoadGeneration += 1;
    if (ttsVoicesLoadingTimer) clearTimeout(ttsVoicesLoadingTimer);
    if (ttsVoicesChangedListener && typeof window !== "undefined" && "speechSynthesis" in window) {
      window.speechSynthesis.removeEventListener("voiceschanged", ttsVoicesChangedListener);
    }
  });

  return {
    nativePlayback,
    refreshNativeSpeech: () => nativeSpeech?.refresh().catch(error => { ttsError.value = nativeError(error); }),
    ttsAvailability,
    ttsVoices,
    ttsPlayback,
    ttsChunkProgress,
    ttsSelectedText,
    ttsSelectionPlayback,
    ttsError,
    ttsNotice,
    ttsExpectedChapterKey,
    httpTtsExpectedChapterKey,
    setHttpTtsAudioElement,
    httpTtsAudioResource,
    httpTtsAudioBusy,
    httpTtsAudioActive,
    httpTtsAudioPaused,
    httpTtsAudioError,
    httpTtsAudioNotice,
    httpTtsChunkProgress,
    ttsCanStart,
    ttsEngineSelection,
    ttsControlPlayback,
    ttsControlCanStart,
    ttsControlStatus,
    ttsControlError,
    ttsControlProgress,
    ttsStatusMessage,
    setupTts,
    refreshTtsVoices,
    retryTtsAvailability,
    selectTtsVoice,
    selectTtsEngine,
    setTtsRate,
    selectHttpTtsConfig,
    startSelectedTts,
    pauseSelectedTts,
    resumeSelectedTts,
    stopSelectedTts,
    startTts,
    stopTts,
    stopTtsForDisplayChange,
    pauseTts,
    resumeTts,
    startHttpTtsAudio,
    stopHttpTtsAudio,
    pauseHttpTtsAudio,
    resumeHttpTtsAudio,
    onHttpTtsAudioPlay,
    onHttpTtsAudioPause,
    onHttpTtsAudioEnded,
    onHttpTtsAudioTimeUpdate,
    onHttpTtsAudioError,
  };
}
