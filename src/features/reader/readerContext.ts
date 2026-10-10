import { computed, type Component, type ComputedRef } from "vue";
import type { useBookSourceSwitch } from "../sources/useBookSourceSwitch";
import type { BookSourceSwitchContract } from "../sources/sourceContext";
import type { useAppReaderFeatures } from "./useAppReaderFeatures";

type ReaderFeatures = ReturnType<typeof useAppReaderFeatures>;

type ReaderFeatureContext = Pick<
  ReaderFeatures,
  | "activePdfZoom"
  | "bookmarkBusy"
  | "bookmarkEditingId"
  | "bookmarkEditorOpen"
  | "bookmarkNote"
  | "comicScaleMode"
  | "currentBookmark"
  | "currentChapterTitle"
  | "downloadReaderBook"
  | "closeBookmarkEditor"
  | "closeReaderDictionary"
  | "dictionaryBusy"
  | "dictionaryError"
  | "dictionaryHtml"
  | "dictionaryLanguage"
  | "dictionaryOpen"
  | "dictionaryProvider"
  | "dictionarySourceUrl"
  | "dictionaryTitle"
  | "dictionaryWord"
  | "editCurrentBookmarkNote"
  | "filteredReaderChapters"
  | "handleReaderFindKeydown"
  | "httpTtsAudioResource"
  | "httpTtsConfigs"
  | "isReaderMediaControlBlocked"
  | "jumpReaderChapterSlider"
  | "loadReaderChapter"
  | "lookupReaderDictionary"
  | "moveReaderFind"
  | "onHttpTtsAudioEnded"
  | "onHttpTtsAudioError"
  | "onHttpTtsAudioPause"
  | "onHttpTtsAudioPlay"
  | "onReaderFrameLoad"
  | "onReaderMediaSleepTimerChange"
  | "setReaderMediaSleepTimer"
  | "onReaderSurfaceClick"
  | "onReaderVideoQualityChange"
  | "openHttpTtsSettings"
  | "openReaderControls"
  | "openReaderDictionary"
  | "openReaderDirectory"
  | "openTtsControls"
  | "pauseSelectedTts"
  | "persistSettings"
  | "prefetchBusy"
  | "progressSaveState"
  | "readerBookDownloadBusy"
  | "readerBusy"
  | "readerChapterCount"
  | "readerChapterError"
  | "readerControlsMode"
  | "readerControlsOpen"
  | "readerDirectoryOpen"
  | "readerDirectoryQuery"
  | "readerErrorChapterIndex"
  | "readerFindIndex"
  | "readerFindMatches"
  | "readerFindQuery"
  | "readerFrame"
  | "readerMediaError"
  | "readerMediaRetrying"
  | "readerMediaPlaying"
  | "readerVideoPlaybackRate"
  | "setReaderVideoPlaybackRate"
  | "readerMediaTimeMs"
  | "readerMediaDurationMs"
  | "toggleReaderMediaPlayback"
  | "seekReaderMedia"
  | "skipReaderMedia"
  | "formatReaderMediaTime"
  | "getReaderMediaElement"
  | "readerMediaSleepTimerMinutes"
  | "readerMenuOpen"
  | "readerPageCount"
  | "readerPageIndex"
  | "readerSourceRefreshBusy"
  | "readerVideoQualityChoices"
  | "readerVideoQualityIndex"
  | "readerVisible"
  | "readingBook"
  | "readingChapterHtml"
  | "readingChapterIndex"
  | "readingImagePage"
  | "readingMediaChapter"
  | "readingPdfInitialPassword"
  | "readingPdfPage"
  | "readingVideoChapter"
  | "refreshCurrentReaderChapter"
  | "resumeSelectedTts"
  | "retryTtsAvailability"
  | "saveSettingsBusy"
  | "selectReaderDirectoryChapter"
  | "selectTtsEngine"
  | "setHttpTtsAudioElement"
  | "setReaderTheme"
  | "setTtsRate"
  | "settings"
  | "shelfBatchRecoveryRequired"
  | "startSelectedTextTts"
  | "startSelectedTts"
  | "stopSelectedTts"
  | "toggleCurrentBookmark"
  | "toggleReaderNightTheme"
  | "ttsAvailability"
  | "ttsCanStart"
  | "ttsControlCanStart"
  | "ttsControlError"
  | "ttsControlPlayback"
  | "ttsControlProgress"
  | "ttsControlStatus"
  | "ttsEngineSelection"
  | "ttsError"
  | "ttsSelectedText"
  | "ttsVoices"
  | "turnPage"
  | "updateReaderSetting"
  | "useSelectedReaderText"
  | "saveBookmark"
>;

type ReaderChapterLoadingContext = Pick<
  ReaderFeatures["readerChapterLoading"],
  | "retryReaderMedia"
>;

type ReaderNavigationContext = Pick<
  ReaderFeatures["readerNavigationActions"],
  | "consumeImportedPdfPassword"
  | "jumpReaderChapter"
  | "setComicScaleMode"
  | "setPdfZoom"
>;

type ReaderSourceSwitchContext = Pick<
  ReturnType<typeof useBookSourceSwitch>,
  | "bookSourceSwitchBusy"
  | "openBookSourceSwitch"
>;

/** The exact reader feature state and actions consumed by the reader UI. */
export type ReaderContextFeatures = ReaderFeatureContext & {
  readerChapterLoading: ReaderChapterLoadingContext;
  readerNavigationActions: ReaderNavigationContext;
};

export interface ReaderContextOptions {
  closeReader: (discardProgress?: boolean) => Promise<void>;
  BackButton: Component;
  PdfReaderPage: Component;
  bookSourceSwitch: Pick<BookSourceSwitchContract, "bookSourceSwitchBusy" | "openBookSourceSwitch">;
}

/**
 * Reader-only contract. Keep this list aligned with what AppReader actually
 * consumes so adding a reader feature does not silently widen the reader API.
 */
export type AppReaderContext =
  & ReaderFeatureContext
  & ReaderChapterLoadingContext
  & ReaderNavigationContext
  & ReaderSourceSwitchContext
  & {
    closeReader: (discardProgress?: boolean) => Promise<void>;
    BackButton: Component;
    PdfReaderPage: Component;
    readerPositionDescription: ComputedRef<string>;
  };

/** Assemble the reader display contract at the reader UI boundary. */
export function createAppReaderContext(
  features: ReaderContextFeatures,
  options: ReaderContextOptions,
): AppReaderContext {
  return {
    activePdfZoom: features.activePdfZoom,
    bookmarkBusy: features.bookmarkBusy,
    bookmarkEditingId: features.bookmarkEditingId,
    bookmarkEditorOpen: features.bookmarkEditorOpen,
    bookmarkNote: features.bookmarkNote,
    comicScaleMode: features.comicScaleMode,
    closeBookmarkEditor: features.closeBookmarkEditor,
    closeReaderDictionary: features.closeReaderDictionary,
    currentBookmark: features.currentBookmark,
    currentChapterTitle: features.currentChapterTitle,
    dictionaryBusy: features.dictionaryBusy,
    dictionaryError: features.dictionaryError,
    dictionaryHtml: features.dictionaryHtml,
    dictionaryLanguage: features.dictionaryLanguage,
    dictionaryOpen: features.dictionaryOpen,
    dictionaryProvider: features.dictionaryProvider,
    dictionarySourceUrl: features.dictionarySourceUrl,
    dictionaryTitle: features.dictionaryTitle,
    dictionaryWord: features.dictionaryWord,
    downloadReaderBook: features.downloadReaderBook,
    editCurrentBookmarkNote: features.editCurrentBookmarkNote,
    filteredReaderChapters: features.filteredReaderChapters,
    handleReaderFindKeydown: features.handleReaderFindKeydown,
    httpTtsAudioResource: features.httpTtsAudioResource,
    httpTtsConfigs: features.httpTtsConfigs,
    isReaderMediaControlBlocked: features.isReaderMediaControlBlocked,
    jumpReaderChapterSlider: features.jumpReaderChapterSlider,
    loadReaderChapter: features.loadReaderChapter,
    lookupReaderDictionary: features.lookupReaderDictionary,
    moveReaderFind: features.moveReaderFind,
    onHttpTtsAudioEnded: features.onHttpTtsAudioEnded,
    onHttpTtsAudioError: features.onHttpTtsAudioError,
    onHttpTtsAudioPause: features.onHttpTtsAudioPause,
    onHttpTtsAudioPlay: features.onHttpTtsAudioPlay,
    onReaderFrameLoad: features.onReaderFrameLoad,
    onReaderMediaSleepTimerChange: features.onReaderMediaSleepTimerChange,
    setReaderMediaSleepTimer: features.setReaderMediaSleepTimer,
    onReaderSurfaceClick: features.onReaderSurfaceClick,
    onReaderVideoQualityChange: features.onReaderVideoQualityChange,
    openHttpTtsSettings: features.openHttpTtsSettings,
    openReaderControls: features.openReaderControls,
    openReaderDictionary: features.openReaderDictionary,
    openReaderDirectory: features.openReaderDirectory,
    openTtsControls: features.openTtsControls,
    pauseSelectedTts: features.pauseSelectedTts,
    persistSettings: features.persistSettings,
    prefetchBusy: features.prefetchBusy,
    progressSaveState: features.progressSaveState,
    readerBookDownloadBusy: features.readerBookDownloadBusy,
    readerBusy: features.readerBusy,
    readerChapterCount: features.readerChapterCount,
    readerChapterError: features.readerChapterError,
    readerControlsMode: features.readerControlsMode,
    readerControlsOpen: features.readerControlsOpen,
    readerDirectoryOpen: features.readerDirectoryOpen,
    readerDirectoryQuery: features.readerDirectoryQuery,
    readerErrorChapterIndex: features.readerErrorChapterIndex,
    readerFindIndex: features.readerFindIndex,
    readerFindMatches: features.readerFindMatches,
    readerFindQuery: features.readerFindQuery,
    readerFrame: features.readerFrame,
    readerMediaError: features.readerMediaError,
    readerMediaRetrying: features.readerMediaRetrying,
    readerMediaPlaying: features.readerMediaPlaying,
    readerVideoPlaybackRate: features.readerVideoPlaybackRate,
    setReaderVideoPlaybackRate: features.setReaderVideoPlaybackRate,
    readerMediaTimeMs: features.readerMediaTimeMs,
    readerMediaDurationMs: features.readerMediaDurationMs,
    toggleReaderMediaPlayback: features.toggleReaderMediaPlayback,
    seekReaderMedia: features.seekReaderMedia,
    skipReaderMedia: features.skipReaderMedia,
    formatReaderMediaTime: features.formatReaderMediaTime,
    getReaderMediaElement: features.getReaderMediaElement,
    readerMediaSleepTimerMinutes: features.readerMediaSleepTimerMinutes,
    readerMenuOpen: features.readerMenuOpen,
    readerPageCount: features.readerPageCount,
    readerPageIndex: features.readerPageIndex,
    readerPositionDescription: computed(() => features.readingMediaChapter.value
      ? features.formatReaderMediaTime(features.readerMediaTimeMs.value)
      : `第 ${features.readerPageIndex.value + 1} / ${features.readerPageCount.value} 页`),
    readerSourceRefreshBusy: features.readerSourceRefreshBusy,
    readerVideoQualityChoices: features.readerVideoQualityChoices,
    readerVideoQualityIndex: features.readerVideoQualityIndex,
    readerVisible: features.readerVisible,
    readingBook: features.readingBook,
    readingChapterHtml: features.readingChapterHtml,
    readingChapterIndex: features.readingChapterIndex,
    readingImagePage: features.readingImagePage,
    readingMediaChapter: features.readingMediaChapter,
    readingPdfInitialPassword: features.readingPdfInitialPassword,
    readingPdfPage: features.readingPdfPage,
    readingVideoChapter: features.readingVideoChapter,
    refreshCurrentReaderChapter: features.refreshCurrentReaderChapter,
    resumeSelectedTts: features.resumeSelectedTts,
    retryTtsAvailability: features.retryTtsAvailability,
    saveSettingsBusy: features.saveSettingsBusy,
    saveBookmark: features.saveBookmark,
    selectReaderDirectoryChapter: features.selectReaderDirectoryChapter,
    selectTtsEngine: features.selectTtsEngine,
    setHttpTtsAudioElement: features.setHttpTtsAudioElement,
    setReaderTheme: features.setReaderTheme,
    setTtsRate: features.setTtsRate,
    settings: features.settings,
    shelfBatchRecoveryRequired: features.shelfBatchRecoveryRequired,
    startSelectedTextTts: features.startSelectedTextTts,
    startSelectedTts: features.startSelectedTts,
    stopSelectedTts: features.stopSelectedTts,
    toggleCurrentBookmark: features.toggleCurrentBookmark,
    toggleReaderNightTheme: features.toggleReaderNightTheme,
    ttsAvailability: features.ttsAvailability,
    ttsCanStart: features.ttsCanStart,
    ttsControlCanStart: features.ttsControlCanStart,
    ttsControlError: features.ttsControlError,
    ttsControlPlayback: features.ttsControlPlayback,
    ttsControlProgress: features.ttsControlProgress,
    ttsControlStatus: features.ttsControlStatus,
    ttsEngineSelection: features.ttsEngineSelection,
    ttsError: features.ttsError,
    ttsSelectedText: features.ttsSelectedText,
    ttsVoices: features.ttsVoices,
    turnPage: features.turnPage,
    updateReaderSetting: features.updateReaderSetting,
    useSelectedReaderText: features.useSelectedReaderText,
    retryReaderMedia: features.readerChapterLoading.retryReaderMedia,
    consumeImportedPdfPassword: features.readerNavigationActions.consumeImportedPdfPassword,
    jumpReaderChapter: features.readerNavigationActions.jumpReaderChapter,
    setComicScaleMode: features.readerNavigationActions.setComicScaleMode,
    setPdfZoom: features.readerNavigationActions.setPdfZoom,
    bookSourceSwitchBusy: computed(() => options.bookSourceSwitch.bookSourceSwitchBusy),
    openBookSourceSwitch: options.bookSourceSwitch.openBookSourceSwitch,
    closeReader: options.closeReader,
    BackButton: options.BackButton,
    PdfReaderPage: options.PdfReaderPage,
  };
}
