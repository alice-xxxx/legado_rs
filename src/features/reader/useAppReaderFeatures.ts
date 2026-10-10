import { computed, nextTick, onScopeDispose, ref, watch, type Ref } from "vue";
import type { UnlistenFn } from "@tauri-apps/api/event";
import type { AppSettingsResource, AppTask, BookResource, DisplayReplacementRule, ResourceDescriptor } from "../../api/types";
import { readResource as appReadResource } from "../../api/resources";
import { refreshChapterContent as appRefreshChapterContent } from "../../api/books";
import { startChapterDownload as appStartChapterDownload } from "../../api/tasks";
import { deferSettingsSaveDuringRestore as deferBackupSettingsSaveDuringRestore, type BackupBusy } from "../settings/backupState";
import type { PdfPageResource } from "./displayHtml";
import type { ReaderPendingFragmentState, ReaderTouchStartState } from "./useReaderPageInteraction";
import { useReaderDisplay } from "./useReaderDisplay";
import { useReaderTts } from "./useReaderTts";
import { useReaderTtsActions } from "./useReaderTtsActions";
import { useReaderSettingsActions } from "./useReaderSettingsActions";
import { useReaderChapterLoading } from "./useReaderChapterLoading";
import { useReaderMedia } from "./useReaderMedia";
import { useReaderNavigationActions } from "./useReaderNavigationActions";
import { useReaderDictionary } from "./useReaderDictionary";
import { useReaderBookmarksAndHistory } from "./useReaderBookmarksAndHistory";
import { useReaderPageInteraction } from "./useReaderPageInteraction";
import { useReaderMediaBinding, type ReaderVideoQualityChoice } from "./useReaderMediaBinding";
import { readerHttpTtsConfigs, registerReaderTtsConfigSelectionWriter } from "./readerTtsConfigService";

type ReaderControlsMode = "settings" | "tts" | "appearance" | "search";

interface AppReaderFeatureDependencies {
  settings: Ref<AppSettingsResource>;
  replacementRules: Ref<DisplayReplacementRule[]>;
  openedBook: Ref<BookResource | null>;
  shelfBatchRecoveryRequired: Ref<boolean>;
  backupBusy: Ref<BackupBusy>;
  pdfImportPasswords: Map<string, { password: string; timer: ReturnType<typeof setTimeout> }>;
  getRestoreRevision: () => number;
  notify: (message: string, kind?: "success" | "error") => void;
  errorText: (error: unknown) => string;
  normalizeSettings: (settings: AppSettingsResource) => AppSettingsResource;
  lockChapterCacheRecovery: (title: string, message: string) => void;
  refreshChapterContent: typeof appRefreshChapterContent;
  startChapterDownload: typeof appStartChapterDownload;
  refreshTasks: (descriptor?: ResourceDescriptor) => Promise<void>;
  reusedCatalogTaskNotice: (task: AppTask) => string;
  openOtherSettings: () => void;
  showShelfBehindReader: (book: BookResource) => Promise<void>;
  readResource?: typeof appReadResource;
}

/** 组合阅读器状态和阅读相关功能。 */
export function useAppReaderFeatures(options: AppReaderFeatureDependencies) {
const { settings, replacementRules, openedBook, shelfBatchRecoveryRequired, backupBusy,
  pdfImportPasswords, getRestoreRevision, notify, errorText,
  normalizeSettings, lockChapterCacheRecovery, refreshChapterContent,
  startChapterDownload, refreshTasks, reusedCatalogTaskNotice, openOtherSettings } = options;
const readResource: typeof appReadResource = options.readResource ?? appReadResource;
const unlisteners: UnlistenFn[] = [];
const readerVisible = ref(false);
const readingBook = ref<BookResource | null>(null);
const readingChapterIndex = ref(0);
const readingChapterHtml = ref("");
const readingChapterRaw = ref("");
const readingPdfPage = ref<PdfPageResource | null>(null);
const readingPdfZoomOverride = ref<PdfPageResource["defaultZoom"] | null>(null);
const activePdfZoom = computed(() => readingPdfZoomOverride.value ?? readingPdfPage.value?.defaultZoom ?? "page-fit");
const readingPdfInitialPassword = ref("");
const readingImagePage = ref(false);
const readingMediaChapter = ref(false);
const readingVideoChapter = ref(false);
const readerMediaTimeMs = ref(0);
const readerMediaDurationMs = ref(0);
const readerMediaPlaying = ref(false);
const readerVideoPlaybackRate = ref(settings.value.reader.videoPlaybackRate);
const readerMediaError = ref("");
const readerMediaRetrying = ref(false);
const readerVideoQualityChoices = ref<ReaderVideoQualityChoice[]>([]);
const readerVideoQualityIndex = ref(0);
const comicImageScrollTop = ref(0);
const comicScaleMode = ref<"fit-width" | "actual-size">("fit-width");
const readerFrame = ref<HTMLIFrameElement | null>(null);
const readerPageIndex = ref(0);
const readerPageCount = ref(1);
const readerControlsOpen = ref(false);
let readerTouchStart: ReaderTouchStartState | null = null;
let readerPendingFragment: ReaderPendingFragmentState | null = null;
let readerPageInteraction!: ReturnType<typeof useReaderPageInteraction>;
let readerDisplay!: ReturnType<typeof useReaderDisplay>;
let readerSettingsActions!: ReturnType<typeof useReaderSettingsActions>;
let readerChapterLoading!: ReturnType<typeof useReaderChapterLoading>;
const readerControlsMode = ref<ReaderControlsMode>("settings");
const readerDirectoryOpen = ref(false);
const readerDirectoryQuery = ref("");
const readerSourceRefreshBusy = ref(false);
const readerMenuOpen = ref(true);
// The prototype opens with navigation visible. A center tap can still hide it for
// immersive reading; entering a new session restores discoverable controls.
watch(readerVisible, visible => {
  if (visible) readerMenuOpen.value = true;
});
const readerFindQuery = ref("");
const readerFindMatches = ref<HTMLElement[]>([]);
const readerFindIndex = ref(-1);
const readerBusy = ref(false);
const readerChapterError = ref("");
const readerErrorChapterIndex = ref<number | null>(null);
const readerProgressDirty = ref(false);
const progressSaveState = ref<"idle" | "saving" | "saved" | "error">("idle");
const prefetchBusy = ref(false);
const readerBookDownloadBusy = ref(false);
const saveSettingsBusy = ref(false);
const readerBackgroundBusy = ref(false);
const readerBackgroundError = ref("");
const pendingSettingsSave = ref<ReturnType<typeof setTimeout> | null>(null);
let progressRevision = 0;
let readerLoadRevision = 0;
let readerMediaElement: HTMLMediaElement | null = null;
watch(() => settings.value.reader.videoPlaybackRate, (rate) => {
  readerVideoPlaybackRate.value = rate;
  const media = readerMediaElement;
  if (media && readingVideoChapter.value && Math.abs(media.playbackRate - rate) > 0.001) {
    try { media.playbackRate = rate; } catch { /* Native playback controls may reject rate changes. */ }
  }
});
let readerClickSuppressedUntil = 0;
let scheduleReaderMediaProgressSaveAction = (_bindingRevision: number, _loadRevision: number): void => {};
let pauseReaderMediaAndSaveAction = async (): Promise<boolean> => true;
let clearReaderMediaQualityResumeHandlerAction = (_media: HTMLMediaElement): void => {};
let getReaderMediaBindingRevisionAction = (): number => 0;
let readerMediaSleepTimer: ReturnType<typeof setTimeout> | null = null;
let readerMediaSleepTimerGeneration = 0;
const readerMediaSleepTimerMinutes = ref<0 | 15 | 30 | 60>(0);
const currentChapter = computed(() => readingBook.value?.chapters?.find((chapter) => chapter.index === readingChapterIndex.value) ?? null);
const currentChapterTitle = computed(() => currentChapter.value?.title ?? `第${readingChapterIndex.value + 1}章`);
const readerChapterCount = computed(() => Math.max(1, readingBook.value?.chapterCount ?? readingBook.value?.chapters?.length ?? 1));
const readerSettings = computed(() => settings.value.reader);
const readerCurrentOffset = computed(() => readingMediaChapter.value
  ? readerMediaTimeMs.value
  : readingImagePage.value ? comicImageScrollTop.value : readerPageIndex.value);
const filteredReaderChapters = computed(() => {
  const chapters = readingBook.value?.chapters ?? [];
  const query = readerDirectoryQuery.value.trim().toLocaleLowerCase("zh-CN");
  return query ? chapters.filter((chapter) => chapter.title.toLocaleLowerCase("zh-CN").includes(query)) : chapters;
});
let saveCurrentProgressAction: () => Promise<void> = async () => {};
let pauseReaderMediaAndSave: () => Promise<boolean> = async () => true;
let clearReaderMediaBinding: (pause?: boolean) => void = () => {};
let clearReaderScrollListeners: () => void = () => {};
let clearReaderFindMatches: () => void = () => {};
let applyPendingReaderFragment: () => void = () => {};
let applyComicScrollPosition: () => void = () => {};
let measureReaderPages: (preserveRatio?: boolean) => void = () => {};
let beginReadingSession: () => void = () => {};
let finishReadingSession: () => Promise<void> = async () => {};
let stopTts: (showNotice?: boolean) => void = () => {};
let stopHttpTtsAudio: (clearSelection?: boolean) => void = () => {};
let stopTtsForDisplayChange: () => void = () => {};
let updateTtsSelection: () => void = () => {};
let refreshTtsVoices: () => void = () => {};
let setupTts: () => void = () => {};
function isBackupRestoreInProgress(): boolean {
  return backupBusy.value === "restore";
}

function isReaderMediaControlBlocked(): boolean {
  return isBackupRestoreInProgress() || shelfBatchRecoveryRequired.value || readerBusy.value || readerMediaRetrying.value;
}

/** Existing iframe media element remains the single player and progress owner. */
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
    media.currentTime = Math.max(0, Math.min(seconds, maximum));
    // The existing seeked listener owns durable progress saving.
  } catch (error) {
    readerMediaError.value = "无法跳转播放位置：" + errorText(error);
  }
}

function skipReaderMedia(seconds: number): void {
  if (readerMediaElement) seekReaderMedia(readerMediaElement.currentTime + seconds);
}

function setReaderVideoPlaybackRate(rate: number): void {
  if (!readingVideoChapter.value || isReaderMediaControlBlocked()
    || !Number.isFinite(rate) || rate < 0.5 || rate > 3) return;
  const media = readerMediaElement;
  if (!media) return;
  try {
    media.playbackRate = rate;
    readerVideoPlaybackRate.value = media.playbackRate;
    readerSettingsActions.updateReaderSetting("videoPlaybackRate", readerVideoPlaybackRate.value);
  } catch (error) {
    readerMediaError.value = "当前视频无法调整播放速度：" + errorText(error);
  }
}


function getReaderCurrentOffset(): number {
  return readerCurrentOffset.value;
}

function showShelfBehindReader(book: BookResource): Promise<void> { return options.showShelfBehindReader(book); }

function selectedReaderText(): string {
  return readerFrame.value?.contentDocument?.getSelection()?.toString().trim() ?? "";
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
    if (readerVisible.value && readingBook.value?.id === bookId && readingMediaChapter.value) await pauseReaderMediaAndSave();
  }, minutes * 60_000);
}

function onReaderMediaSleepTimerChange(event: Event): void {
  setReaderMediaSleepTimer(Number((event.target as HTMLSelectElement).value));
}

async function closeReader(discardProgress = false): Promise<void> {
  if (isBackupRestoreInProgress() && !discardProgress) return;
  stopTts(false);
  stopHttpTtsAudio(false);
  ttsSelectedText.value = "";
  readerPendingFragment = null;
  if (!discardProgress) {
    if (readerMediaElement && !(await pauseReaderMediaAndSave())) return;
    await saveCurrentProgressAction();
    if (readerProgressDirty.value) return;
    await finishReadingSession();
  } else {
    readerProgressDirty.value = false;
    progressRevision += 1;
  }
  readerLoadRevision += 1;
  clearReaderMediaBinding(true);
  clearReaderScrollListeners();
  clearReaderFindMatches();
  readerFindQuery.value = "";
  readerVisible.value = false;
  readerControlsOpen.value = false;
  readerDirectoryOpen.value = false;
  readerChapterError.value = "";
  readerErrorChapterIndex.value = null;
  clearReaderMediaSleepTimer();
  readingChapterHtml.value = "";
  readingChapterRaw.value = "";
  readingPdfPage.value = null;
  readingPdfZoomOverride.value = null;
  readingPdfInitialPassword.value = "";
  readingImagePage.value = false;
  readingMediaChapter.value = false;
  readingVideoChapter.value = false;
  readerFrame.value = null;
  readerBusy.value = false;
  if (discardProgress) {
    readingBook.value = null;
    readingChapterIndex.value = 0;
  }
}

function handleReaderResize(): void {
  measureReaderPages(true);
}

function cancelPendingPdfImportOnUnmount(): void {
  for (const entry of pdfImportPasswords.values()) clearTimeout(entry.timer);
  pdfImportPasswords.clear();
  readingPdfInitialPassword.value = "";
}

let readerNavigationActions!: ReturnType<typeof useReaderNavigationActions>;
let readerMediaBinding!: ReturnType<typeof useReaderMediaBinding>;
let readerMediaPlayback!: ReturnType<typeof useReaderMedia>;
let readerTts!: ReturnType<typeof useReaderTts>;
let readerTtsActions!: ReturnType<typeof useReaderTtsActions>;
let readerBookmarksAndHistory!: ReturnType<typeof useReaderBookmarksAndHistory>;
let deferSettingsSaveDuringRestore = deferBackupSettingsSaveDuringRestore;

function loadReaderChapter(index: number, mediaOffset?: number, fragment?: string): Promise<boolean> {
  return readerChapterLoading.loadReaderChapter(index, mediaOffset, fragment);
}

function startReading(book: BookResource, index?: number, offset?: number): Promise<void> {
  return readerChapterLoading.startReading(book, index, offset);
}

function setReaderPage(index: number): void { readerNavigationActions.setReaderPage(index); }
function turnPage(direction: -1 | 1): void { readerNavigationActions.turnPage(direction); }
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
    book: readingBook,
    chapterTitle: currentChapterTitle,
    getCurrentElement: () => readerMediaElement,
    isControlBlocked: isReaderMediaControlBlocked,
    error: readerMediaError,
  },
  changeChapter,
  pauseAndSave: () => pauseReaderMediaAndSaveAction(),
  clearQualityResumeHandler: (media) => clearReaderMediaQualityResumeHandlerAction(media),
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
    videoPlaybackRate: readerVideoPlaybackRate,
    error: readerMediaError,
    progressDirty: readerProgressDirty,
    progressSaveState,
    qualityChoices: readerVideoQualityChoices,
    qualityIndex: readerVideoQualityIndex,
    getLoadRevision: () => readerLoadRevision,
    getMediaElement: () => readerMediaElement,
    setMediaElement: (media) => { readerMediaElement = media; },
    getAudioPlaybackRate: () => settings.value.reader.audioPlaybackRate,
    incrementProgressRevision: () => { progressRevision += 1; },
  },
  actions: {
    changeChapter,
    saveCurrentProgress,
    isControlBlocked: isReaderMediaControlBlocked,
    updateAudioPlaybackRate: (rate) => updateReaderSetting("audioPlaybackRate", rate),
    updateVideoPlaybackRate: (rate) => updateReaderSetting("videoPlaybackRate", rate),
    notify,
    errorText,
    formatMediaTime: formatReaderMediaTime,
  },
  mediaPlayback: readerMediaPlayback,
});
pauseReaderMediaAndSaveAction = readerMediaBinding.pauseReaderMediaAndSave;
pauseReaderMediaAndSave = readerMediaBinding.pauseReaderMediaAndSave;
clearReaderMediaQualityResumeHandlerAction = readerMediaBinding.clearQualityResumeHandler;
scheduleReaderMediaProgressSaveAction = readerMediaBinding.scheduleReaderMediaProgressSave;
getReaderMediaBindingRevisionAction = readerMediaBinding.getBindingRevision;
clearReaderMediaBinding = readerMediaBinding.clearReaderMediaBinding;

readerDisplay = useReaderDisplay({
  reader: {
    frame: readerFrame,
    visible: readerVisible,
    busy: readerBusy,
    book: readingBook,
    chapterIndex: readingChapterIndex,
    chapterTitle: currentChapterTitle,
    rawHtml: readingChapterRaw,
    html: readingChapterHtml,
    settings,
    replacementRules,
    imagePage: readingImagePage,
    mediaChapter: readingMediaChapter,
    videoChapter: readingVideoChapter,
    pdfPage: readingPdfPage,
    pageIndex: readerPageIndex,
    pageCount: readerPageCount,
    progressDirty: readerProgressDirty,
    progressSaveState,
    comicScrollTop: comicImageScrollTop,
    comicScaleMode,
    mediaError: readerMediaError,
    videoQualityChoices: readerVideoQualityChoices,
    videoQualityIndex: readerVideoQualityIndex,
    getLoadRevision: () => readerLoadRevision,
    getPendingFragment: () => readerPendingFragment,
    setPendingFragment: (value) => { readerPendingFragment = value; },
    getTouchStart: () => readerTouchStart,
    setTouchStart: (value) => { readerTouchStart = value; },
    getClickSuppressedUntil: () => readerClickSuppressedUntil,
    setClickSuppressedUntil: (value) => { readerClickSuppressedUntil = value; },
  },
  media: { binding: readerMediaBinding, playback: readerMediaPlayback },
  getPageInteraction: () => readerPageInteraction,
  actions: {
    setPage: setReaderPage,
    turnPage,
    closeReader,
    incrementProgressRevision: () => { progressRevision += 1; },
    stopHttpTtsAudio: (clearSelection) => stopHttpTtsAudio(clearSelection),
    clearTtsHighlight: () => readerTtsActions.clearTtsHighlight(),
    updateTtsSelection: () => updateTtsSelection(),
    notify,
    errorText,
  },
});
const { frameBinding, rebuildReaderDisplay, scrollReaderFrameToPage, onReaderFrameScroll,
  onReaderFrameKeydown, shouldIgnoreReaderKeydown, onReaderFrameTouchStart, onReaderFrameTouchMove,
  onReaderFrameTouchEnd, onReaderFrameLoad } = readerDisplay;
({ measureReaderPages, applyComicScrollPosition } = readerDisplay);

readerPageInteraction = useReaderPageInteraction({
  reader: {
    frame: readerFrame,
    book: readingBook,
    currentChapterSource: computed(() => currentChapter.value?.resource?.src),
    chapterIndex: readingChapterIndex,
    pageCount: readerPageCount,
    visible: readerVisible,
    busy: readerBusy,
    mediaChapter: readingMediaChapter,
    imagePage: readingImagePage,
    pdfPage: readingPdfPage,
    sourceRefreshBusy: readerSourceRefreshBusy,
    settings: readerSettings,
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
  frameBinding,
  events: {
    onScroll: onReaderFrameScroll,
    onSelectionChange: updateTtsSelection,
    onKeydown: onReaderFrameKeydown,
    onTouchStart: onReaderFrameTouchStart,
    onTouchMove: onReaderFrameTouchMove,
    onTouchEnd: onReaderFrameTouchEnd,
  },
  actions: { setPage: setReaderPage, turnPage, loadChapter: loadReaderChapter },
});
const { onReaderFrameTouchCancel, readerSegmentForElement, readerSegmentForRect,
  onReaderFrameClick, onReaderSurfaceClick, rebuildReaderFind, moveReaderFind,
  handleReaderFindKeydown } = readerPageInteraction;
watch(readerFindQuery, () => rebuildReaderFind(true));
({ applyPendingReaderFragment, clearReaderFindMatches, clearReaderScrollListeners } = readerPageInteraction);

readerChapterLoading = useReaderChapterLoading({
  reader: {
    book: readingBook,
    openedBook,
    chapterIndex: readingChapterIndex,
    chapterHtml: readingChapterHtml,
    chapterRaw: readingChapterRaw,
    pdfPage: readingPdfPage,
    pdfZoomOverride: readingPdfZoomOverride,
    pdfInitialPassword: readingPdfInitialPassword,
    imagePage: readingImagePage,
    mediaChapter: readingMediaChapter,
    videoChapter: readingVideoChapter,
    mediaTimeMs: readerMediaTimeMs,
    mediaDurationMs: readerMediaDurationMs,
    mediaError: readerMediaError,
    mediaRetrying: readerMediaRetrying,
    pageIndex: readerPageIndex,
    pageCount: readerPageCount,
    comicScrollTop: comicImageScrollTop,
    comicScaleMode,
    visible: readerVisible,
    controlsOpen: readerControlsOpen,
    directoryOpen: readerDirectoryOpen,
    controlsMode: readerControlsMode,
    menuOpen: readerMenuOpen,
    busy: readerBusy,
    progressDirty: readerProgressDirty,
    progressSaveState,
    prefetchBusy,
    shelfBatchRecoveryRequired,
    pdfImportPasswords,
    settings,
    replacementRules,
    currentChapterTitle,
    getMediaElement: () => readerMediaElement,
    getLoadRevision: () => readerLoadRevision,
    advanceLoadRevision: () => { readerLoadRevision += 1; return readerLoadRevision; },
    advanceProgressRevision: () => { progressRevision += 1; },
    getTtsPlayback: () => readerTts?.ttsPlayback.value ?? "idle",
    getTtsExpectedChapterKey: () => readerTts?.ttsExpectedChapterKey.value ?? null,
    setPendingFragment: (value) => { readerPendingFragment = value; },
    resetClickSuppression: () => { readerClickSuppressedUntil = 0; },
    chapterError: readerChapterError,
    errorChapterIndex: readerErrorChapterIndex,
  },
  actions: {
    showShelfBehindReader,
    stopTts: (showNotice) => stopTts(showNotice),
    pauseReaderMediaAndSave: () => pauseReaderMediaAndSave(),
    clearReaderMediaBinding: (pause) => clearReaderMediaBinding(pause),
    clearReaderFindMatches: () => clearReaderFindMatches(),
    clearReaderScrollListeners: () => clearReaderScrollListeners(),
    applyPendingReaderFragment: () => applyPendingReaderFragment(),
    applyComicScrollPosition: () => applyComicScrollPosition(),
    scrollReaderFrameToPage: () => scrollReaderFrameToPage(),
    measureReaderPages: () => measureReaderPages(),
    beginReadingSession: () => beginReadingSession(),
    isBackupRestoreInProgress,
    notify,
    errorText,
  },
});

readerNavigationActions = useReaderNavigationActions({
  comicScaleMode, readingPdfZoomOverride, readingPdfInitialPassword, pdfImportPasswords,
  readerPageIndex, readerPageCount, readerVisible, readerBusy, readingImagePage, readingMediaChapter,
  readingPdfPage, readingBook, readingChapterIndex, readerProgressDirty, progressSaveState,
  readerMediaTimeMs, comicImageScrollTop, readerMediaRetrying, shelfBatchRecoveryRequired,
  currentChapterId: () => currentChapter.value?.id ?? null,
  getReaderMediaElement: () => readerMediaElement,
  getReaderMediaRevision: () => getReaderMediaBindingRevisionAction(),
  getReaderLoadRevision: () => readerLoadRevision,
  getProgressRevision: () => progressRevision,
  advanceProgressRevision: () => { progressRevision += 1; },
  isBackupRestoreInProgress,
  rebuildReaderDisplay: () => rebuildReaderDisplay(),
  scrollReaderFrameToPage: () => scrollReaderFrameToPage(),
  loadReaderChapter,
  scheduleReaderMediaProgressSave: (bindingRevision, loadRevision) => scheduleReaderMediaProgressSaveAction(bindingRevision, loadRevision),
  notify,
  errorText,
});
saveCurrentProgressAction = readerNavigationActions.saveCurrentProgress;

readerSettingsActions = useReaderSettingsActions({
  settings,
  saveSettingsBusy,
  readerBackgroundBusy,
  readerBackgroundError,
  pendingSettingsSave,
  readerMediaError,
  readerMediaElement: () => readerMediaElement,
  readingVideoChapter,
  backupBusy: () => backupBusy.value !== null,
  recoveryRequired: shelfBatchRecoveryRequired,
  deferSettingsSaveDuringRestore: () => deferSettingsSaveDuringRestore(),
  normalizeSettings,
  rebuildReaderDisplay: () => rebuildReaderDisplay(),
  notify,
  errorText,
});
const { setReaderTheme, applyAppearanceSettings, updateReaderSetting,
  updateSourceHttpTimeout, scheduleSettingsSave, persistSettings, chooseReaderBackgroundImage,
  clearReaderBackground } = readerSettingsActions;

readerBookmarksAndHistory = useReaderBookmarksAndHistory({
  readingBook,
  readingChapterIndex,
  getReaderCurrentOffset,
  isBackupRestoreInProgress,
  getRestoreRevision,
  startReading,
  notify,
  errorText,
});
const { bookmarks, readingHistory, readingStatisticsPeriod, readingStatisticsData, readingStatisticsBusy,
  readingStatisticsError, bookmarkEditorOpen, bookmarkNote, bookmarkEditingId, bookmarkBusy,
  bookmarkDeleteBatchBusy, bookmarkDeleteBatchResult, currentBookmark, refreshBookmarksAndReadingHistory,
  refreshCurrentReadingStatistics, changeReadingStatisticsPeriod, continueReading, toggleCurrentBookmark,
  editCurrentBookmarkNote, closeBookmarkEditor, beginReadingSession: beginReadingSessionAction,
  finishReadingSession: finishReadingSessionAction, openBookmark, saveBookmark, removeBookmark, removeBookmarks,
  clearReadingHistoryData, removeBookHistory } = readerBookmarksAndHistory;
beginReadingSession = beginReadingSessionAction;
finishReadingSession = finishReadingSessionAction;

const ttsSelectionError = ref("");
readerTtsActions = useReaderTtsActions({
  reader: { frame: readerFrame, visible: readerVisible, mediaChapter: readingMediaChapter,
    imagePage: readingImagePage, pdfPage: readingPdfPage, getLoadRevision: () => readerLoadRevision },
  selection: { getCurrentText: selectedReaderText, rememberedText: computed(() => readerTts.ttsSelectedText.value), error: ttsSelectionError },
  playback: { start: (selectedText) => readerTts.startTts(selectedText) },
  display: { openControls: () => openReaderControls("tts"), setPage: setReaderPage,
    segmentForElement: readerSegmentForElement, segmentForRect: readerSegmentForRect,
    scrollToPage: () => scrollReaderFrameToPage() },
});
readerTts = useReaderTts({
  reader: { settings: readerSettings, visible: readerVisible, busy: readerBusy, mediaChapter: readingMediaChapter,
    pdfPage: readingPdfPage, imagePage: readingImagePage, book: readingBook, chapterIndex: readingChapterIndex,
    getLoadRevision: () => readerLoadRevision },
  httpTtsConfigs: readerHttpTtsConfigs,
  loadSegments: readerTtsActions.waitForReaderTtsSegments,
  changeChapter,
  updateReaderSetting,
  highlightChunk: readerTtsActions.highlightTtsChunk,
  clearHighlight: readerTtsActions.clearTtsHighlight,
  errorText,
});
const { ttsAvailability, ttsVoices, ttsPlayback, ttsChunkProgress, ttsSelectedText, ttsSelectionPlayback,
  ttsNotice, ttsExpectedChapterKey, httpTtsExpectedChapterKey, setHttpTtsAudioElement, httpTtsAudioResource,
  httpTtsAudioBusy, httpTtsAudioActive, httpTtsAudioPaused, httpTtsAudioError, httpTtsAudioNotice, httpTtsChunkProgress: httpTtsChunkProgressState,
  ttsCanStart, ttsEngineSelection, ttsControlPlayback, ttsControlCanStart, ttsControlStatus, ttsControlError, ttsControlProgress,
  ttsStatusMessage, retryTtsAvailability, selectTtsVoice, selectTtsEngine, setTtsRate, selectHttpTtsConfig,
  startSelectedTts, pauseSelectedTts, resumeSelectedTts, stopSelectedTts, startHttpTtsAudio,
  startTts, pauseTts, resumeTts, onHttpTtsAudioPlay, onHttpTtsAudioPause, onHttpTtsAudioEnded, onHttpTtsAudioError } = readerTts;
onScopeDispose(registerReaderTtsConfigSelectionWriter(selectHttpTtsConfig));
stopTts = readerTts.stopTts;
stopHttpTtsAudio = readerTts.stopHttpTtsAudio;
stopTtsForDisplayChange = readerTts.stopTtsForDisplayChange;
setupTts = readerTts.setupTts;
refreshTtsVoices = readerTts.refreshTtsVoices;
watch(
  () => ({
    visible: readerVisible.value,
    bookId: readingBook.value?.id ?? "",
    chapterIndex: currentChapter.value?.index ?? readingChapterIndex.value,
    chapterId: currentChapter.value?.id ?? "",
    recovering: shelfBatchRecoveryRequired.value,
    restoring: backupBusy.value === "restore",
  }),
  (current) => {
    readerTts.ttsSelectedText.value = "";
    if (!current.visible || current.recovering || current.restoring) {
      readerTts.stopTts(false);
      readerTts.stopHttpTtsAudio(false);
      return;
    }
    const key = `${current.bookId}:${current.chapterIndex}`;
    if (readerTts.httpTtsExpectedChapterKey.value === key) readerTts.httpTtsExpectedChapterKey.value = null;
    else readerTts.stopHttpTtsAudio(false);
    if (readerTts.ttsExpectedChapterKey.value === key) {
      readerTts.ttsExpectedChapterKey.value = null;
      return;
    }
    if (readerTts.ttsPlayback.value !== "idle") readerTts.stopTts(false);
  },
  { flush: "sync" },
);
watch(
  () => [
    readerSettings.value.fontSizePx,
    readerSettings.value.lineHeight,
    readerSettings.value.fontFamily,
    readerSettings.value.textColor,
    readerSettings.value.backgroundColor,
    readerSettings.value.textAlign,
    readerSettings.value.theme,
    readerSettings.value.verticalScroll,
  ] as const,
  () => {
    if (readerVisible.value && readingChapterHtml.value) {
      readerTts.stopTtsForDisplayChange();
      readerDisplay.rebuildReaderDisplay();
    }
  },
  { flush: "sync" },
);
watch(replacementRules, () => {
  if (readerVisible.value && readingChapterHtml.value) {
    readerTts.stopTtsForDisplayChange();
    readerDisplay.rebuildReaderDisplay();
  }
}, { deep: true, flush: "sync" });
watch(readerPageIndex, () => {
  readerTts.ttsSelectedText.value = "";
});
updateTtsSelection = () => { const selected = selectedReaderText(); if (selected) readerTts.ttsSelectedText.value = selected; };
const { openTtsControls, clearTtsHighlight, highlightTtsChunk, waitForReaderTtsSegments,
  startSelectedTextTts } = readerTtsActions;
const ttsError = computed(() => readerTts.ttsError.value || ttsSelectionError.value);

const readerDictionary = useReaderDictionary({
  frame: readerFrame, book: readingBook, chapter: currentChapter, visible: readerVisible,
  settings, replacementRules, getReaderLoadRevision: () => readerLoadRevision,
  getRestoreRevision, isBackupRestoreInProgress, errorText,
});
const { dictionaryOpen, dictionaryWord, dictionaryLanguage, dictionaryBusy, dictionaryError,
  dictionaryTitle, dictionaryProvider, dictionarySourceUrl, dictionaryHtml, openReaderDictionary,
  useSelectedReaderText, lookupReaderDictionary, closeReaderDictionary } = readerDictionary;

const { bindReaderMediaElement, clearQualityResumeHandler, flushReaderMediaProgress,
  onReaderVideoQualityChange, scheduleReaderMediaProgressSave, selectReaderVideoQuality,
  syncReaderMediaPosition } = readerMediaBinding;
scheduleReaderMediaProgressSaveAction = scheduleReaderMediaProgressSave;

function openReaderDirectory(): void {
  readerDirectoryQuery.value = "";
  readerDirectoryOpen.value = true;
  readerControlsOpen.value = false;
  readerMenuOpen.value = true;
}

async function selectReaderDirectoryChapter(index: number): Promise<void> {
  if (readerBusy.value || index < 0 || index >= readerChapterCount.value) return;
  if (index === readingChapterIndex.value) {
    readerDirectoryOpen.value = false;
    return;
  }
  if (await loadReaderChapter(index)) readerDirectoryOpen.value = false;
}

async function jumpReaderChapterSlider(event: Event): Promise<void> {
  const index = Number((event.target as HTMLInputElement).value);
  if (Number.isInteger(index) && index >= 0 && index < readerChapterCount.value && index !== readingChapterIndex.value) {
    await loadReaderChapter(index);
  }
}

async function refreshCurrentReaderChapter(): Promise<void> {
  const book = readingBook.value;
  const chapterIndex = readerErrorChapterIndex.value ?? readingChapterIndex.value;
  const chapter = book?.chapters.find((entry) => entry.index === chapterIndex);
  if (!book || !chapter || !readerVisible.value || readerBusy.value || readerSourceRefreshBusy.value
    || shelfBatchRecoveryRequired.value || isBackupRestoreInProgress()) return;
  if (!book.sourceId || readingMediaChapter.value || readingImagePage.value || readingPdfPage.value) {
    notify("当前章节不支持从书源重新获取。", "error");
    return;
  }
  if (!(await pauseReaderMediaAndSave())) {
    notify("阅读位置尚未保存，无法安全更新章节缓存。", "error");
    return;
  }
  const offset = chapterIndex === readingChapterIndex.value ? readerCurrentOffset.value : 0;
  await saveCurrentProgress();
  if (readerProgressDirty.value) {
    notify("阅读位置尚未保存，无法安全更新章节缓存。", "error");
    return;
  }

  const loadRevision = readerLoadRevision;
  const isCurrentChapter = () => readerVisible.value && readingBook.value?.id === book.id
    && readingChapterIndex.value === chapterIndex && readerLoadRevision === loadRevision
    && !isBackupRestoreInProgress();
  if (!isCurrentChapter()) return;
  readerSourceRefreshBusy.value = true;
  readerBusy.value = true;
  try {
    const response = await refreshChapterContent(book.id, chapter.id);
    if (response.commitState !== "committed" || response.recoveryRequired) {
      const detail = response.error?.trim() || response.warning?.trim() || "事务未提交，原章节缓存仍保留。";
      if (response.commitState === "indeterminate" || response.recoveryRequired) {
        lockChapterCacheRecovery("章节刷新状态未知", `《${book.title}》第 ${chapterIndex + 1} 章刷新状态未知。请重启应用完成缓存恢复。${detail}`);
        notify("章节刷新状态未知；请重启应用后再继续。", "error");
      } else {
        notify(`未能从书源更新当前章节：${detail}`, "error");
      }
      return;
    }
    if (!isCurrentChapter()) return;
    if (!response.book) throw new Error("书源已返回刷新结果，但没有返回更新后的书籍资源。");
    const updatedBook = await readResource<BookResource>(response.book);
    if (updatedBook.id !== book.id) throw new Error("刷新后返回的书籍与当前阅读书籍不匹配。");
    if (!isCurrentChapter()) return;
    readingBook.value = updatedBook;
    if (openedBook.value?.id === book.id) openedBook.value = updatedBook;
    if (!(await loadReaderChapter(chapterIndex))) return;
    if (!readerVisible.value || readingBook.value?.id !== book.id) return;
    if (offset) setReaderPage(offset);
    notify("已从书源重新获取当前章节并更新缓存。");
  } catch (error) {
    if (isCurrentChapter()) notify(`从书源刷新当前章节失败：${errorText(error)}`, "error");
  } finally {
    readerSourceRefreshBusy.value = false;
    if (isCurrentChapter()) readerBusy.value = false;
  }
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
    const response = await startChapterDownload(book.id, 0, chapterCount);
    try {
      await refreshTasks(response.resource);
    } catch (error) {
      notify(`${response.reused ? "已有全书缓存任务" : "全书缓存任务已加入"}，但任务列表刷新失败：${errorText(error)}`, "error");
      return;
    }
    notify(response.reused ? reusedCatalogTaskNotice(response.task) : "全书章节缓存任务已加入。");
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
    refreshTtsVoices();
  } else if (mode === "search") {
    void nextTick(() => document.getElementById("reader-find-input")?.focus());
  }
}

function toggleReaderNightTheme(): void {
  setReaderTheme(settings.value.reader.theme === "dark" ? "paper" : "dark");
}

async function openHttpTtsSettings(): Promise<void> {
  await closeReader();
  if (!readerVisible.value) {
    openedBook.value = null;
    openOtherSettings();
  }
}



return {
  settings, replacementRules, openedBook, shelfBatchRecoveryRequired, backupBusy, pdfImportPasswords,
  httpTtsConfigs: readerHttpTtsConfigs,
  getRestoreRevision, notify, errorText, normalizeSettings, lockChapterCacheRecovery, refreshChapterContent, startChapterDownload, refreshTasks,
  reusedCatalogTaskNotice, openOtherSettings, frameBinding, rebuildReaderDisplay, scrollReaderFrameToPage, onReaderFrameScroll, onReaderFrameKeydown, shouldIgnoreReaderKeydown,
  onReaderFrameTouchStart, onReaderFrameTouchMove, onReaderFrameTouchEnd, onReaderFrameLoad, onReaderFrameTouchCancel, readerSegmentForElement, readerSegmentForRect, onReaderFrameClick,
  onReaderSurfaceClick, rebuildReaderFind, moveReaderFind, handleReaderFindKeydown, setReaderTheme, applyAppearanceSettings, updateReaderSetting,
  updateSourceHttpTimeout, scheduleSettingsSave, persistSettings, chooseReaderBackgroundImage, clearReaderBackground, bookmarks, readingHistory, readingStatisticsPeriod,
  readingStatisticsData, readingStatisticsBusy, readingStatisticsError, bookmarkEditorOpen, bookmarkNote, bookmarkEditingId, bookmarkBusy, bookmarkDeleteBatchBusy,
  bookmarkDeleteBatchResult, currentBookmark, refreshBookmarksAndReadingHistory, refreshCurrentReadingStatistics, changeReadingStatisticsPeriod, continueReading, toggleCurrentBookmark, editCurrentBookmarkNote,
  closeBookmarkEditor, beginReadingSessionAction, finishReadingSessionAction, openBookmark, saveBookmark, removeBookmark, removeBookmarks, clearReadingHistoryData,
  removeBookHistory, ttsAvailability, ttsVoices, ttsPlayback, ttsChunkProgress, ttsSelectedText, ttsSelectionPlayback, ttsNotice,
  ttsExpectedChapterKey, httpTtsExpectedChapterKey, setHttpTtsAudioElement, httpTtsAudioResource, httpTtsAudioBusy, httpTtsAudioActive, httpTtsAudioPaused, httpTtsAudioError, httpTtsAudioNotice,
  httpTtsChunkProgressState, ttsCanStart, ttsEngineSelection, ttsControlPlayback, ttsControlCanStart, ttsControlStatus, ttsControlError, ttsControlProgress,
  ttsStatusMessage, retryTtsAvailability, selectTtsVoice, selectTtsEngine, setTtsRate, selectHttpTtsConfig, startSelectedTts, pauseSelectedTts,
  resumeSelectedTts, stopSelectedTts, startHttpTtsAudio, startTts, pauseTts, resumeTts, onHttpTtsAudioPlay, onHttpTtsAudioPause, onHttpTtsAudioEnded, onHttpTtsAudioError,
  openTtsControls, clearTtsHighlight, highlightTtsChunk,
  waitForReaderTtsSegments, startSelectedTextTts, dictionaryOpen, dictionaryWord, dictionaryLanguage, dictionaryBusy, dictionaryError, dictionaryTitle,
  dictionaryProvider, dictionarySourceUrl, dictionaryHtml, openReaderDictionary, useSelectedReaderText, lookupReaderDictionary, closeReaderDictionary, bindReaderMediaElement,
  clearQualityResumeHandler, flushReaderMediaProgress, onReaderVideoQualityChange, scheduleReaderMediaProgressSave, selectReaderVideoQuality, syncReaderMediaPosition, readResource,
  unlisteners, readerVisible, readingBook, readingChapterIndex, readingChapterHtml, readingChapterRaw, readingPdfPage, readingPdfZoomOverride,
  activePdfZoom, readingPdfInitialPassword, readingImagePage, readingMediaChapter, readingVideoChapter, readerMediaTimeMs, readerMediaDurationMs, readerMediaError,
  readerMediaRetrying, readerMediaPlaying, readerVideoPlaybackRate, setReaderVideoPlaybackRate, toggleReaderMediaPlayback, seekReaderMedia, skipReaderMedia, readerVideoQualityChoices, readerVideoQualityIndex, comicImageScrollTop, comicScaleMode, readerFrame, readerPageIndex, readerPageCount,
  readerControlsOpen, readerTouchStart, readerPendingFragment, readerPageInteraction, readerDisplay, readerSettingsActions, readerChapterLoading, readerControlsMode,
  readerDirectoryOpen, readerDirectoryQuery, readerSourceRefreshBusy, readerMenuOpen, readerFindQuery, readerFindMatches, readerFindIndex, readerBusy,
  readerProgressDirty, progressSaveState, prefetchBusy, readerBookDownloadBusy, readerChapterError, readerErrorChapterIndex, saveSettingsBusy, readerBackgroundBusy, readerBackgroundError, pendingSettingsSave,
  progressRevision, readerLoadRevision, getReaderLoadRevision: () => readerLoadRevision, readerMediaElement, readerClickSuppressedUntil, scheduleReaderMediaProgressSaveAction, pauseReaderMediaAndSaveAction, clearReaderMediaQualityResumeHandlerAction, getReaderMediaBindingRevisionAction,
  readerMediaSleepTimer, readerMediaSleepTimerGeneration, readerMediaSleepTimerMinutes, currentChapter, currentChapterTitle, readerChapterCount, readerSettings, readerCurrentOffset,
  filteredReaderChapters, saveCurrentProgressAction, pauseReaderMediaAndSave, clearReaderMediaBinding, clearReaderScrollListeners, clearReaderFindMatches, applyPendingReaderFragment, applyComicScrollPosition,
  measureReaderPages, beginReadingSession, finishReadingSession, stopTts, stopHttpTtsAudio, stopTtsForDisplayChange, updateTtsSelection, refreshTtsVoices,
  setupTts, readerNavigationActions, readerMediaBinding, readerMediaPlayback, readerTts, readerTtsActions, readerBookmarksAndHistory, deferSettingsSaveDuringRestore,
  ttsSelectionError, ttsError, readerDictionary, isBackupRestoreInProgress, isReaderMediaControlBlocked, getReaderCurrentOffset, showShelfBehindReader, selectedReaderText,
  clearReaderMediaSleepTimer, setReaderMediaSleepTimer, onReaderMediaSleepTimerChange, closeReader, handleReaderResize, cancelPendingPdfImportOnUnmount, loadReaderChapter, startReading,
  setReaderPage, turnPage, changeChapter, saveCurrentProgress, formatReaderMediaTime, openReaderDirectory, selectReaderDirectoryChapter, jumpReaderChapterSlider,
  refreshCurrentReaderChapter, downloadReaderBook, openReaderControls, toggleReaderNightTheme, openHttpTtsSettings, getReaderMediaElement: () => readerMediaElement, hasActiveReaderMedia: () => Boolean(readerMediaElement), closeReaderAfterRestore: async () => { await closeReader(true); },
};
}
