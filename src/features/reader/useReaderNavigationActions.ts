import { openedBook } from "../books/bookDetailState";
import type { Ref } from "vue";
import { clamp } from "./readerContent";
import type { PdfPageResource } from "./readerContent";

type ReaderNavigationOptions = {
  comicScaleMode: Ref<"fit-width" | "actual-size">;
  readingPdfZoomOverride: Ref<PdfPageResource["defaultZoom"] | null>;
  readingPdfInitialPassword: Ref<string>;
  pdfImportPasswords: Map<string, { password: string; timer: ReturnType<typeof setTimeout> }>;
  readerPageIndex: Ref<number>;
  readerPageCount: Readonly<Ref<number>>;
  readerVisible: Readonly<Ref<boolean>>;
  readerDirectoryOpen: Ref<boolean>;
  readerBusy: Readonly<Ref<boolean>>;
  readingImagePage: Readonly<Ref<boolean>>;
  readingMediaChapter: Readonly<Ref<boolean>>;
  readingPdfPage: Readonly<Ref<PdfPageResource | null>>;
  readingChapterIndex: Ref<number>;
  readerProgressDirty: Ref<boolean>;
  progressSaveState: Ref<"idle" | "saving" | "saved" | "error">;
  readerMediaRetrying: Readonly<Ref<boolean>>;
  shelfBatchRecoveryRequired: Readonly<Ref<boolean>>;
  getProgressRevision: () => number;
  advanceProgressRevision: () => void;
  isBackupRestoreInProgress: () => boolean;
  scrollReaderFrameToPage: (animate?: boolean, direction?: -1 | 1) => void;
  loadReaderChapter: (index: number) => Promise<boolean>;
  notifyCurrentReading: () => Promise<void>;
  notify: (message: string, kind?: "success" | "error") => void;
  errorText: (error: unknown) => string;
};

/** 管理章节翻页、章节跳转和阅读进度保存，进度版本由阅读器状态统一递增。 */
export function useReaderNavigationActions(options: ReaderNavigationOptions) {

  function setComicScaleMode(mode: "fit-width" | "actual-size"): void {
    if (options.comicScaleMode.value === mode) return;
    options.comicScaleMode.value = mode;
  }

  function setPdfZoom(mode: PdfPageResource["defaultZoom"]): void {
    options.readingPdfZoomOverride.value = mode;
  }

  function consumeImportedPdfPassword(): void {
    const bookId = openedBook.value?.id;
    if (bookId) {
      const entry = options.pdfImportPasswords.get(bookId);
      if (entry) clearTimeout(entry.timer);
      options.pdfImportPasswords.delete(bookId);
    }
    options.readingPdfInitialPassword.value = "";
  }

  function setReaderPage(index: number, animate = false, direction: -1 | 1 = 1): void {
    if (!options.readerVisible.value || options.readerBusy.value || options.readingImagePage.value
      || options.readingMediaChapter.value || options.readingPdfPage.value) return;
    const next = Math.round(clamp(index, 0, options.readerPageCount.value - 1));
    if (next === options.readerPageIndex.value) return;
    options.readerPageIndex.value = next;
    options.readerProgressDirty.value = true;
    options.advanceProgressRevision();
    options.progressSaveState.value = "idle";
    options.scrollReaderFrameToPage(animate, direction);
    void saveCurrentProgress();
  }

  function turnPage(direction: -1 | 1): void {
    if (!options.readerVisible.value || options.readerBusy.value) return;
    if (options.readingImagePage.value || options.readingMediaChapter.value || options.readingPdfPage.value) {
      void changeChapter(direction);
      return;
    }
    const next = options.readerPageIndex.value + direction;
    if (next >= 0 && next < options.readerPageCount.value) {
      setReaderPage(next, true, direction);
      return;
    }
    if (direction > 0 && options.readerPageIndex.value === options.readerPageCount.value - 1) void changeChapter(1);
    else if (direction < 0 && options.readerPageIndex.value === 0) void changeChapter(-1);
  }

  async function selectReaderChapter(index: number): Promise<boolean> {
    if (!options.readerVisible.value || options.readerBusy.value || options.readerMediaRetrying.value
      || options.shelfBatchRecoveryRequired.value || options.isBackupRestoreInProgress()
      || !openedBook.value?.chapters.some(chapter => chapter.index === index)) return false;
    if (index !== options.readingChapterIndex.value && !(await options.loadReaderChapter(index))) return false;
    options.readerDirectoryOpen.value = false;
    return true;
  }

  async function jumpReaderChapter(event: Event): Promise<void> {
    const input = event.target as HTMLInputElement;
    const chapterNumber = input.type === "number";
    const index = Number(input.value) - (chapterNumber ? 1 : 0);
    const count = openedBook.value?.chapters.length ?? 0;
    if (Number.isInteger(index) && index >= 0 && index < count) await selectReaderChapter(index);
    else options.notify(`请输入 1 到 ${count} 之间的章号。`, "error");
    input.value = String(options.readingChapterIndex.value + (chapterNumber ? 1 : 0));
  }

  async function changeChapter(direction: -1 | 1): Promise<void> {
    if (options.readerBusy.value || options.readerMediaRetrying.value) return;
    const nextIndex = options.readingChapterIndex.value + direction;
    const count = openedBook.value?.chapterCount ?? openedBook.value?.chapters.length ?? 0;
    if (nextIndex < 0) {
      options.notify("已经是第一章。");
      return;
    }
    if (nextIndex >= count) {
      options.notify("已经到达源提供的最后一章。", "error");
      return;
    }
    await selectReaderChapter(nextIndex);
  }

  async function saveCurrentProgress(): Promise<void> {
    const book = openedBook.value;
    if (!book || !options.readerProgressDirty.value || options.isBackupRestoreInProgress()) return;
    const revision = options.getProgressRevision();
    const isCurrent = () => openedBook.value?.id === book.id && options.getProgressRevision() === revision;
    options.progressSaveState.value = "saving";
    try {
      await options.notifyCurrentReading();
      if (isCurrent()) { options.readerProgressDirty.value = false; options.progressSaveState.value = "saved"; }
    } catch (error) {
      if (isCurrent()) { options.progressSaveState.value = "error"; options.notify('阅读进度保存失败：' + options.errorText(error), "error"); }
    }
  }

  return {
    setComicScaleMode,
    setPdfZoom,
    consumeImportedPdfPassword,
    setReaderPage,
    turnPage,
    selectReaderChapter,
    jumpReaderChapter,
    changeChapter,
    saveCurrentProgress,
  };
}
