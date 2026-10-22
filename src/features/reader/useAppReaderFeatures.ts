import { textPosition, imagePosition, type ReaderPosition } from "./readerPosition";
import { computed, nextTick, onScopeDispose, ref, watch } from "vue";
import { notifyReading } from "../../api/books";
import { mainText } from "../../api/mainText";
import { bookContentType } from "../../api/bookContentType";
import type { BookResource } from "../../api/types";
import { backupBusy } from "../settings/backupState";
import { shelfBatchRecoveryRequired } from "../../app/recoveryState";
import { isImageOnlyChapter, type PdfPageResource } from "./readerContent";
import type { ReaderPendingFragmentState, ReaderTouchStartState } from "./useReaderPageInteraction";
import { settings } from "../settings/settingsStore";
import { useReaderDisplay } from "./useReaderDisplay";
import { useReaderChapterWindow } from "./useReaderChapterWindow";
import { useReaderTts } from "./useReaderTts";
import { useReaderTtsActions } from "./useReaderTtsActions";
import { type CachedChapterContent, useReaderChapterLoading } from "./useReaderChapterLoading";
import { useReaderMedia } from "./useReaderMedia";
import { useReaderNavigationActions } from "./useReaderNavigationActions";
import { useReaderDictionary } from "./useReaderDictionary";
import { useReaderBookmarksAndHistory } from "./useReaderBookmarksAndHistory";
import { useReaderPageInteraction } from "./useReaderPageInteraction";
import { useReaderMediaBinding } from "./useReaderMediaBinding";
import { NativeReaderAudio, seekMedia, type ReaderMediaElement } from "./nativeAudio";
import { registerReaderTtsConfigSelectionWriter } from "./readerTtsConfigService";
import { clearOpenedBook, openedBook, refreshOpenedBook, switchOpenedBook } from "../books/bookDetailState";
import { collectReaderTtsSegments } from "./ttsText";

type ReaderControlsMode = "settings" | "tts" | "appearance" | "search";

interface AppReaderFeatureDependencies {
  getRestoreRevision: () => number;
  notify: (message: string, kind?: "success" | "error") => void;
  errorText: (error: unknown) => string;
  openOtherSettings: () => void;
  showShelfBehindReader: (book: BookResource) => Promise<void>;
}

/** 组合阅读器状态和阅读相关功能。 */
export function useAppReaderFeatures(options: AppReaderFeatureDependencies) {
const { getRestoreRevision, notify, errorText,
  openOtherSettings } = options;
const readerVisible = ref(false);
const readingBook = openedBook;
const readingChapterIndex = ref(0);
const readingContent = ref<CachedChapterContent | null>(null);
const readerViewport = ref<HTMLElement | null>(null);
const readingPdfPage = computed(() => readingContent.value?.kind === "pdfPage" ? readingContent.value : null);
const readingPdfZoomOverride = ref<PdfPageResource["defaultZoom"] | null>(null);
const activePdfZoom = computed(() => readingPdfZoomOverride.value ?? readingPdfPage.value?.defaultZoom ?? "page-fit");
const readingContentType = computed(() => {
  const content = readingContent.value;
  if (content?.kind === "media") return content.mediaType;
  return bookContentType(readingBook.value)
    ?? (content?.kind === "richText" && isImageOnlyChapter(content.markup) ? "image" : "text");
});
const readingImagePage = computed(() => readingContentType.value === "image");
const readingMediaChapter = computed(() => readingContentType.value === "audio" || readingContentType.value === "video");
const readingVideoChapter = computed(() => readingContentType.value === "video");
const readerMediaTimeMs = ref(0);
const readerMediaDurationMs = ref(0);
const readerMediaPlaying = ref(false);
const readerMediaError = ref("");
const readerMediaRetrying = ref(false);
const readerVideoQualityChoices = computed(() => readingContent.value?.kind === "media" ? readingContent.value.sources ?? [] : []);
const readerVideoQualityIndex = ref(0);
const comicImageScrollTop = ref(0);
const comicScaleMode = ref<"fit-width" | "actual-size">("fit-width");
const readerPendingPosition = ref<ReaderPosition | null>(null);
const readerPageIndex = ref(0);
const readerPageCount = ref(1);
const readerControlsOpen = ref(false);
let readerTouchStart: ReaderTouchStartState | null = null;
let readerPendingFragment: ReaderPendingFragmentState | null = null;
let readerPageInteraction!: ReturnType<typeof useReaderPageInteraction>;
let readerDisplay!: ReturnType<typeof useReaderDisplay>;
let readerChapterLoading!: ReturnType<typeof useReaderChapterLoading>;
let readerChapterWindow!: ReturnType<typeof useReaderChapterWindow>;
const readerControlsMode = ref<ReaderControlsMode>("settings");
const readerDirectoryOpen = ref(false);
const readerMenuOpen = ref(false);
const readerFindQuery = ref("");
const readerFindMatches = ref<HTMLElement[]>([]);
const readerFindIndex = ref(-1);
const readerBusy = ref(false);
const readerChapterError = ref("");
const readerProgressDirty = ref(false);
const progressSaveState = ref<"idle" | "saving" | "saved" | "error">("idle");
const readerBookDownloadBusy = ref(false);
let progressRevision = 0;
let readerLoadRevision = 0;
let readerMediaElement: ReaderMediaElement | null = null;

let readerClickSuppressedUntil = 0;
let readerMediaSleepTimer: ReturnType<typeof setTimeout> | null = null;
let readerMediaSleepTimerGeneration = 0;
const readerMediaSleepTimerMinutes = ref<0 | 15 | 30 | 60>(0);
const currentChapter = computed(() => readingBook.value?.chapters?.find((chapter) => chapter.index === readingChapterIndex.value) ?? null);
const currentChapterTitle = computed(() => currentChapter.value?.title ?? `第${readingChapterIndex.value + 1}章`);
const readerChapterCount = computed(() => Math.max(1, readingBook.value?.chapterCount ?? readingBook.value?.chapters?.length ?? 1));
function currentReaderPosition(): ReaderPosition {
  if (readerPendingPosition.value) return readerPendingPosition.value;
  if (readingMediaChapter.value) return { offset: readerMediaTimeMs.value,
    quality: readingVideoChapter.value ? readerVideoQualityChoices.value[readerVideoQualityIndex.value]?.label : undefined };
  if (readingPdfPage.value) return { offset: 0 };
  if (readingImagePage.value) return imagePosition(readerViewport.value);
  return textPosition(readerViewport.value, settings.value.reader.readingMode === "vertical");
}
const readerCurrentOffset = computed(() => {
  void readerPageIndex.value; void comicImageScrollTop.value; void readerMediaTimeMs.value;
  return currentReaderPosition().offset;
});

function isBackupRestoreInProgress(): boolean {
  return backupBusy.value === "restore";
}

function isReaderMediaControlBlocked(): boolean {
  return isBackupRestoreInProgress() || shelfBatchRecoveryRequired.value || readerBusy.value || readerMediaRetrying.value;
}

/** Native media element remains the single player and progress owner. */
function toggleReaderMediaPlayback(): void {
  const media = readerMediaElement;
  if (!media || isReaderMediaControlBlocked()) return;
  if (!media.paused) {
    media.pause();
    return;
  }
  void media.play().catch((error: unknown) => {
    if (readerMediaElement !== media) return;
    readerMediaError.value = "无法开始播放：" + errorText(error);
  });
}

function seekReaderMedia(seconds: number): void {
  const media = readerMediaElement;
  if (!media || isReaderMediaControlBlocked() || !Number.isFinite(seconds)) return;
  const maximum = Number.isFinite(media.duration) && media.duration >= 0 ? media.duration : Number.POSITIVE_INFINITY;
  try {
    seekMedia(media, Math.max(0, Math.min(seconds, maximum)));
    // The existing seeked listener owns durable progress saving.
  } catch (error) {
    readerMediaError.value = "无法跳转播放位置：" + errorText(error);
  }
}

function skipReaderMedia(seconds: number): void {
  if (readerMediaElement) seekReaderMedia(readerMediaElement.currentTime + seconds);
}

function getReaderCurrentOffset(): number {
  return readerCurrentOffset.value;
}

async function notifyCurrentReading(refresh = false, prepareCurrent = false): Promise<void> {
  const book = openedBook.value;
  if (!book) return;
  const chapter = currentChapter.value;
  await notifyReading(book.id, { chapterId: chapter?.id ?? null,
    chapterIndex: readingChapterIndex.value, ...currentReaderPosition(), updatedAtMs: Date.now() }, prepareCurrent);
  if (refresh && chapter) {
    await mainText(book.id, chapter.id);
    await refreshOpenedBook(book.id);
  }
}

function selectedReaderText(): string {
  const root = readerViewport.value;
  const selection = root?.ownerDocument.getSelection();
  return root && selection?.anchorNode && root.contains(selection.anchorNode)
    ? selection.toString().trim() : "";
}

function clearReaderMediaSleepTimer(): void {
  readerMediaSleepTimerGeneration += 1;
  if (readerMediaSleepTimer) clearTimeout(readerMediaSleepTimer);
  readerMediaSleepTimer = null;
  readerMediaSleepTimerMinutes.value = 0;
}

function setReaderMediaSleepTimer(minutes: number): void {
  if (![0, 15, 30, 60].includes(minutes)) return;
  clearReaderMediaSleepTimer();
  if (minutes === 0) return;
  const generation = readerMediaSleepTimerGeneration;
  const bookId = readingBook.value?.id;
  if (!bookId || !readerVisible.value || !readingMediaChapter.value || isReaderMediaControlBlocked()) return;
  readerMediaSleepTimerMinutes.value = minutes as 0 | 15 | 30 | 60;
  readerMediaSleepTimer = setTimeout(async () => {
    if (generation !== readerMediaSleepTimerGeneration) return;
    clearReaderMediaSleepTimer();
    if (readerVisible.value && readingBook.value?.id === bookId && readingMediaChapter.value) await readerMediaBinding.pauseReaderMediaAndSave();
  }, minutes * 60_000);
}

function onReaderMediaSleepTimerChange(event: Event): void {
  setReaderMediaSleepTimer(Number((event.target as HTMLSelectElement).value));
}

async function closeReader(discardProgress = false): Promise<void> {
  if (isBackupRestoreInProgress() && !discardProgress) return;
  readerTts.stopTts(false);
  ttsSelectedText.value = "";
  readerPendingFragment = null;
  if (!discardProgress) {
    if (readerMediaElement && !(await readerMediaBinding.pauseReaderMediaAndSave())) return;
    await readerNavigationActions.saveCurrentProgress();
    if (readerProgressDirty.value) return;
    await readerBookmarksAndHistory.finishReadingSession();
  } else {
    readerProgressDirty.value = false;
    progressRevision += 1;
  }
  readerLoadRevision += 1;
  readerMediaBinding.clearReaderMediaBinding(true);
  readerDisplay.clearReaderScrollListeners();
  readerPageInteraction.clearReaderFindMatches();
  readerFindQuery.value = "";
  readerChapterLoading.cancel();
  readerBookmarksAndHistory.closeBookmarkEditor();
  readerDictionary.closeReaderDictionary();
  readerVisible.value = false;
  readerControlsOpen.value = false;
  readerDirectoryOpen.value = false;
  readerChapterError.value = "";
  clearReaderMediaSleepTimer();
  readingContent.value = null;
  readingPdfZoomOverride.value = null;
  readerViewport.value = null;
  readerBusy.value = false;
  if (discardProgress) {
    clearOpenedBook();
    readingChapterIndex.value = 0;
  }
}

function handleReaderResize(): void {
  readerDisplay.handleReaderResize();
}



let readerNavigationActions!: ReturnType<typeof useReaderNavigationActions>;
let readerMediaBinding!: ReturnType<typeof useReaderMediaBinding>;
let readerMediaPlayback!: ReturnType<typeof useReaderMedia>;
let readerTts!: ReturnType<typeof useReaderTts>;
let readerTtsActions!: ReturnType<typeof useReaderTtsActions>;
let readerBookmarksAndHistory!: ReturnType<typeof useReaderBookmarksAndHistory>;

function loadReaderChapter(index: number, mediaOffset?: number, fragment?: string): Promise<boolean> {
  return readerChapterLoading.loadReaderChapter(index, mediaOffset, fragment);
}

async function startReading(book: BookResource, index?: number, offset?: number, positionHint?: ReaderPosition): Promise<void> {
  const activeBook = openedBook.value?.id === book.id ? openedBook.value : await switchOpenedBook(book.id);
  if (activeBook) await readerChapterLoading.startReading(activeBook, index, offset, positionHint);
}

function setReaderPage(index: number): void { readerNavigationActions.setReaderPage(index); }
function turnPage(direction: -1 | 1): void {
  if (!readerBusy.value && readerChapterWindow?.turn(direction)) return;
  readerNavigationActions.turnPage(direction);
}
function changeChapter(direction: -1 | 1): Promise<void> { return readerNavigationActions.changeChapter(direction); }
function saveCurrentProgress(): Promise<void> { return readerNavigationActions.saveCurrentProgress(); }

function formatReaderMediaTime(milliseconds: number): string {
  const seconds = Math.floor(Math.max(0, milliseconds) / 1000);
  return `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, "0")}`;
}

readerMediaPlayback = useReaderMedia({
  reader: {
    visible: readerVisible,
    mediaChapter: readingMediaChapter,
    videoChapter: readingVideoChapter,
    chapterTitle: currentChapterTitle,
    getCurrentElement: () => readerMediaElement,
    isControlBlocked: isReaderMediaControlBlocked,
    error: readerMediaError,
  },
  changeChapter,
  pauseAndSave: () => readerMediaBinding.pauseReaderMediaAndSave(),
  clearQualityResumeHandler: (media) => readerMediaBinding.clearQualityResumeHandler(media),
  notify,
  errorText,
  getSeekSeconds: () => settings.value.reader.audioSkipSeconds,
});

readerMediaBinding = useReaderMediaBinding({
  reader: {
    visible: readerVisible,
    mediaChapter: readingMediaChapter,
    videoChapter: readingVideoChapter,
    chapterIndex: readingChapterIndex,
    timeMs: readerMediaTimeMs,
    durationMs: readerMediaDurationMs,
    playing: readerMediaPlaying,
    error: readerMediaError,
    progressDirty: readerProgressDirty,
    progressSaveState,
    qualityChoices: readerVideoQualityChoices,
    qualityIndex: readerVideoQualityIndex,
    getLoadRevision: () => readerLoadRevision,
    getMediaElement: () => readerMediaElement,
    setMediaElement: (media) => { readerMediaElement = media; },
    incrementProgressRevision: () => { progressRevision += 1; },
  },
  actions: {
    changeChapter,
    saveCurrentProgress,
    isControlBlocked: isReaderMediaControlBlocked,

    notify,
    errorText,
    formatMediaTime: formatReaderMediaTime,
  },
  mediaPlayback: readerMediaPlayback,
});

readerDisplay = useReaderDisplay({
  reader: { viewport: readerViewport, visible: readerVisible, busy: readerBusy,
    imagePage: readingImagePage, mediaChapter: readingMediaChapter,
    menuOpen: readerMenuOpen,
    pageIndex: readerPageIndex, pageCount: readerPageCount, comicScrollTop: comicImageScrollTop,
    getLoadRevision: () => readerLoadRevision, getTouchStart: () => readerTouchStart,
    setTouchStart: value => { readerTouchStart = value; }, getClickSuppressedUntil: () => readerClickSuppressedUntil,
    setClickSuppressedUntil: value => { readerClickSuppressedUntil = value; } },
  getPageInteraction: () => readerPageInteraction,
  actions: { setPage: setReaderPage, turnPage, saveProgress: saveCurrentProgress,
    markProgress: () => { readerProgressDirty.value = true; progressRevision += 1; },
    stopTts: () => readerTts.stopTtsForDisplayChange(), clearTtsHighlight: () => readerTtsActions.clearTtsHighlight(),
    updateTtsSelection: () => updateTtsSelection(), onTextScroll: () => readerChapterWindow?.onScroll(),
    chapterPreview: direction => readerChapterWindow?.chapterPreview(direction) ?? null,
    commitChapterTurn: direction => readerChapterWindow.commitChapterTurn(direction) },
});
const { scrollReaderFrameToPage } = readerDisplay;

readerPageInteraction = useReaderPageInteraction({
  reader: {
    viewport: readerViewport,
    chapterIndex: readingChapterIndex,
    pageIndex: readerPageIndex,
    pageCount: readerPageCount,
    visible: readerVisible,
    busy: readerBusy,
    mediaChapter: readingMediaChapter,
    imagePage: readingImagePage,
    pdfPage: readingPdfPage,
    sourceRefreshBusy: readerMediaRetrying,
    menuOpen: readerMenuOpen,
    controlsOpen: readerControlsOpen,
    directoryOpen: readerDirectoryOpen,
    findQuery: readerFindQuery,
    findMatches: readerFindMatches,
    findIndex: readerFindIndex,
    getLoadRevision: () => readerLoadRevision,
    getPendingFragment: () => readerPendingFragment,
    setPendingFragment: (value) => { readerPendingFragment = value; },
    getTouchStart: () => readerTouchStart,
    setTouchStart: (value) => { readerTouchStart = value; },
    getClickSuppressedUntil: () => readerClickSuppressedUntil,
    setClickSuppressedUntil: (value) => { readerClickSuppressedUntil = value; },
  },
  actions: { setPage: setReaderPage, turnPage, loadChapter: loadReaderChapter },
});
const { readerSegmentForElement, readerSegmentForRect, onReaderSurfaceClick, rebuildReaderFind, moveReaderFind,
  handleReaderFindKeydown } = readerPageInteraction;
watch(readerFindQuery, () => rebuildReaderFind(true));

readerChapterLoading = useReaderChapterLoading({
  reader: {
    chapterIndex: readingChapterIndex, pendingPosition: readerPendingPosition,
    content: readingContent,
    pdfPage: readingPdfPage,
    pdfZoomOverride: readingPdfZoomOverride,
    imagePage: readingImagePage,
    mediaChapter: readingMediaChapter,
    mediaTimeMs: readerMediaTimeMs,
    mediaDurationMs: readerMediaDurationMs,
    mediaError: readerMediaError,
    mediaRetrying: readerMediaRetrying,
    pageIndex: readerPageIndex,
    pageCount: readerPageCount,
    comicScrollTop: comicImageScrollTop,
    visible: readerVisible,
    controlsOpen: readerControlsOpen,
    directoryOpen: readerDirectoryOpen,
    controlsMode: readerControlsMode,
    menuOpen: readerMenuOpen,
    busy: readerBusy,
    progressDirty: readerProgressDirty,
    getMediaElement: () => readerMediaElement,
    getExpectedTtsChapter: () => readerTts?.ttsExpectedChapterKey.value,
    getLoadRevision: () => readerLoadRevision,
    advanceLoadRevision: () => { readerLoadRevision += 1; return readerLoadRevision; },
    advanceProgressRevision: () => { progressRevision += 1; },
    setPendingFragment: (value) => { readerPendingFragment = value; },
    resetClickSuppression: () => { readerClickSuppressedUntil = 0; },
    chapterError: readerChapterError,
  },
  actions: {
    notifyCurrentReading,
    showShelfBehindReader: options.showShelfBehindReader,
    stopTts: (showNotice) => readerTts.stopTts(showNotice),
    pauseReaderMediaAndSave: () => readerMediaBinding.pauseReaderMediaAndSave(),
    clearReaderMediaBinding: (pause) => readerMediaBinding.clearReaderMediaBinding(pause),
    clearReaderFindMatches: () => readerPageInteraction.clearReaderFindMatches(),
    clearReaderScrollListeners: () => readerDisplay.clearReaderScrollListeners(),
    applyPendingReaderFragment: () => readerPageInteraction.applyPendingReaderFragment(),
    applyComicScrollPosition: () => readerDisplay.applyComicScrollPosition(),
    restoreReadingPosition: (position) => readerDisplay.restoreReadingPosition(position),
    captureReadingPosition: currentReaderPosition,
    measureReaderPages: () => readerDisplay.measureReaderPages(),
    beginReadingSession: () => readerBookmarksAndHistory.beginReadingSession(),
    isBackupRestoreInProgress,
    errorText,
    notify,
  },
});

readerNavigationActions = useReaderNavigationActions({
  comicScaleMode, readingPdfZoomOverride,
  readerPageIndex, readerPageCount, readerVisible, readerBusy, readerDirectoryOpen, readingImagePage, readingMediaChapter,
  readingPdfPage, readingChapterIndex, readerProgressDirty, progressSaveState,
  readerMediaRetrying, shelfBatchRecoveryRequired, notifyCurrentReading,
  getProgressRevision: () => progressRevision,
  advanceProgressRevision: () => { progressRevision += 1; },
  isBackupRestoreInProgress,
  scrollReaderFrameToPage: (animate, direction) => scrollReaderFrameToPage(animate, direction),
  loadReaderChapter,
  notify,
  errorText,
});

readerChapterWindow = useReaderChapterWindow({
  book: readingBook, index: readingChapterIndex, content: readingContent, visible: readerVisible,
  busy: readerBusy, viewport: readerViewport,
  adopt: chapter => readerChapterLoading.adoptPreparedChapter(chapter),
  animate: (viewport, direction, done) => readerDisplay.animateChapterTurn(viewport, direction, done),
  measure: () => readerDisplay.measureReaderPages(), setPage: setReaderPage, save: saveCurrentProgress, notify,
});

readerBookmarksAndHistory = useReaderBookmarksAndHistory({
  readingChapterIndex,
  getReaderCurrentOffset, getReaderCurrentPosition: currentReaderPosition,
  isBackupRestoreInProgress,
  getRestoreRevision,
  startReading,
  notify,
  errorText,
});

const ttsSelectionError = ref("");
readerTtsActions = useReaderTtsActions({
  reader: { viewport: readerViewport, busy: readerBusy, visible: readerVisible, mediaChapter: readingMediaChapter,
    imagePage: readingImagePage, pdfPage: readingPdfPage, getLoadRevision: () => readerLoadRevision },
  selection: { getCurrentText: selectedReaderText, rememberedText: computed(() => readerTts.ttsSelectedText.value), error: ttsSelectionError },
  playback: { start: (selectedText) => readerTts.startTts(selectedText) },
  display: { openControls: () => openReaderControls("tts"), setPage: setReaderPage,
    segmentForElement: readerSegmentForElement, segmentForRect: readerSegmentForRect,
    scrollToPage: () => scrollReaderFrameToPage() },
});
readerTts = useReaderTts({
  reader: { visible: readerVisible, busy: readerBusy, mediaChapter: readingMediaChapter,
    pdfPage: readingPdfPage, imagePage: readingImagePage, chapterIndex: readingChapterIndex,
    getLoadRevision: () => readerLoadRevision },
  loadSegments: readerTtsActions.waitForReaderTtsSegments,
  prepareSegments: async index => {
    const book = openedBook.value;
    const chapter = book?.chapters.find(chapter => chapter.index === index);
    if (!book || !chapter) return [];
    if (!chapter.resource) {
      await mainText(book.id, chapter.id);
      await refreshOpenedBook(book.id);
    }
    if (!(await readerChapterWindow.prepare(index))) return [];
    await nextTick();
    const root = readerChapterWindow.chapterElement(index);
    if (!root) return [];
    return collectReaderTtsSegments(root.ownerDocument, false, { width: root.clientWidth, height: root.clientHeight }, root, true);
  },
  openChapter: async index => { await readerNavigationActions.selectReaderChapter(index); },
  highlightChunk: readerTtsActions.highlightTtsChunk,
  clearHighlight: readerTtsActions.clearTtsHighlight,
  errorText,
});
const { ttsSelectedText, selectHttpTtsConfig } = readerTts;
onScopeDispose(registerReaderTtsConfigSelectionWriter(selectHttpTtsConfig));
watch(
  [readerVisible, () => readingBook.value?.id ?? "",
    () => currentChapter.value?.index ?? readingChapterIndex.value,
    () => currentChapter.value?.id ?? "", shelfBatchRecoveryRequired,
    () => backupBusy.value === "restore"] as const,
  ([visible, bookId, chapterIndex, , recovering, restoring]) => {
    readerTts.ttsSelectedText.value = "";
    if (!visible || recovering || restoring) {
      readerTts.stopTts(false);
      return;
    }
    const key = `${bookId}:${chapterIndex}`;
    if (readerTts.ttsExpectedChapterKey.value === key) {
      return;
    }
    if (readerTts.ttsActive.value) readerTts.stopTts(false);
  },
  { flush: "sync" },
);
watch(readerPageIndex, () => {
  readerTts.ttsSelectedText.value = "";
});
function updateTtsSelection(): void { const selected = selectedReaderText(); if (selected) readerTts.ttsSelectedText.value = selected; }

const ttsError = computed(() => readerTts.ttsError.value || ttsSelectionError.value);

const readerDictionary = useReaderDictionary({
  viewport: readerViewport, chapter: currentChapter, visible: readerVisible,
  getReaderLoadRevision: () => readerLoadRevision,
  getRestoreRevision, isBackupRestoreInProgress, errorText,
});

function setReaderMediaElement(element: unknown): void {
  if (!(element instanceof HTMLMediaElement) && !(element instanceof NativeReaderAudio)) { readerMediaBinding.clearReaderMediaBinding(true); return; }
  if (readerMediaElement === element) return;
  const content = readingContent.value;
  if (content?.kind !== "media") return;
  readerMediaBinding.bindReaderMediaElement(element);
  readerVideoQualityIndex.value = Math.max(0, readerVideoQualityChoices.value.findIndex(option => option.selected));
  const choice = readerVideoQualityChoices.value[readerVideoQualityIndex.value];
  readerMediaBinding.loadReaderMediaSource(element, choice?.src || content.src, choice?.format ? choice.format === "hls" : content.format === "hls",
    readerMediaBinding.getBindingRevision(), readerLoadRevision);
}

function openReaderDirectory(): void {
  readerDirectoryOpen.value = true;
  readerControlsOpen.value = false;
  readerMenuOpen.value = true;
}

async function downloadReaderBook(): Promise<void> {
  const book = readingBook.value;
  if (!book || readerBookDownloadBusy.value || shelfBatchRecoveryRequired.value || isBackupRestoreInProgress()) return;
  const chapterCount = Math.max(book.chapterCount, book.chapters.length);
  if (chapterCount < 1) {
    notify("这本书没有可缓存的章节。", "error");
    return;
  }
  readerBookDownloadBusy.value = true;
  try {
    await Promise.all(book.chapters.filter(chapter => !chapter.volume).map(chapter => mainText(book.id, chapter.id, true)));
    if (isBackupRestoreInProgress()) return;
    notify("全书章节缓存完成。");
  } catch (error) {
    notify(`无法缓存全书章节：${errorText(error)}`, "error");
  } finally {
    readerBookDownloadBusy.value = false;
  }
}

function openReaderControls(mode: ReaderControlsMode = "settings"): void {
  readerControlsMode.value = mode;
  readerControlsOpen.value = true;
  readerDirectoryOpen.value = false;
  if (mode === "tts") {
    updateTtsSelection();
    readerTts.refreshTtsVoices();
  } else if (mode === "search") {
    void nextTick(() => document.getElementById("reader-find-input")?.focus());
  }
}

async function openHttpTtsSettings(): Promise<void> {
  await closeReader();
  if (!readerVisible.value) {
    clearOpenedBook();
    openOtherSettings();
  }
}

return {
  continuousChapters: readerChapterWindow.continuousChapters,
  previewChapters: readerChapterWindow.previewChapters,
  setReaderChapterPreview: readerChapterWindow.setPreview,
  ...readerTts, ...readerTtsActions, ...readerDictionary,
  ...readerBookmarksAndHistory, ...readerMediaBinding, ...readerNavigationActions, ...readerDisplay,
  readerVisible, readingBook, readingChapterIndex, readingContent, readingPdfPage, activePdfZoom,
  readingContentType, readingImagePage, readingMediaChapter, readingVideoChapter, readerViewport, readerPageIndex, readerPageCount,
  readerControlsOpen, readerControlsMode, readerDirectoryOpen, readerMenuOpen,
  readerFindQuery, readerFindMatches, readerFindIndex, readerBusy, readerProgressDirty, progressSaveState,
  readerBookDownloadBusy, readerChapterError,
  readerMediaError, readerMediaRetrying, readerMediaPlaying, readerMediaTimeMs, readerMediaDurationMs,
  readerVideoQualityChoices, readerVideoQualityIndex, readerMediaSleepTimerMinutes, comicScaleMode,
  readerChapterLoading, currentChapterTitle, readerChapterCount,
  onReaderSurfaceClick, moveReaderFind, handleReaderFindKeydown, isReaderMediaControlBlocked,
  setReaderMediaElement, toggleReaderMediaPlayback, seekReaderMedia, skipReaderMedia,
  clearReaderMediaSleepTimer, setReaderMediaSleepTimer, onReaderMediaSleepTimerChange, closeReader, handleReaderResize,
  loadReaderChapter, startReading, setReaderPage, turnPage, saveCurrentProgress,
  formatReaderMediaTime, openReaderDirectory,
  downloadReaderBook, openReaderControls, openHttpTtsSettings,
  ttsError, selectedReaderText, clearReaderScrollListeners: readerDisplay.clearReaderScrollListeners,
  getReaderMediaElement: () => readerMediaElement, hasActiveReaderMedia: () => Boolean(readerMediaElement),
  closeReaderAfterRestore: async () => { await closeReader(true); },
};
}
