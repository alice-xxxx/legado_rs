import { ref, type ComputedRef, type Ref } from "vue";
import { type ResourceDescriptor, type SearchBookResult, type SearchResource, type ChapterRichTextResource, type ChapterTextResource, type SourceDebugResponse, type SourceMetadata } from "../../api/types";
import { clearSourceDebugPreview, debugSearchResult, searchBooks } from "../../api/search";
import { readResource } from "../../api/resources";

interface SourceDebugDependencies {
  /** 当前可供书籍搜索的书源列表。 */
  sources: Ref<SourceMetadata[]>;
  /** 已启用且非 RSS 的候选书源。 */
  bookSourceCandidates: ComputedRef<SourceMetadata[]>;
  /** 书源列表中当前选中的行。 */
  selectedSourceRows: Ref<string[]>;
  sourceBatchBusy: Ref<boolean>;
  sourceExportBusy: Ref<boolean>;
  searchBusy: Readonly<Ref<boolean>>;
  searchResultBatchBusy: Ref<boolean>;
  shelfBatchRecoveryRequired: Readonly<Ref<boolean>>;
  searchBooks: typeof searchBooks;
  debugSearchResult: typeof debugSearchResult;
  clearSourceDebugPreview: typeof clearSourceDebugPreview;
  readResource: typeof readResource;
  notify: (message: string, kind?: "success" | "error") => void;
  errorText: (error: unknown) => string;
}

/** 处理单个书源的搜索、详情和章节预览调试流程。 */
export function useSourceDebugActions({
  sources,
  bookSourceCandidates,
  selectedSourceRows,
  sourceBatchBusy,
  sourceExportBusy,
  searchBusy,
  searchResultBatchBusy,
  shelfBatchRecoveryRequired,
  searchBooks,
  debugSearchResult,
  clearSourceDebugPreview,
  readResource,
  notify,
  errorText,
}: SourceDebugDependencies) {
  const sourceDebugTarget = ref<SourceMetadata | null>(null);
  const sourceDebugResults = ref<SearchBookResult[]>([]);
  const sourceDebugSearched = ref(false);
  const sourceDebugErrors = ref<Array<{ sourceId?: string; message: string }>>([]);
  const sourceDebugSelectedResult = ref<SearchBookResult | null>(null);
  const sourceDebugBook = ref<SourceDebugResponse["book"] | null>(null);
  const sourceDebugChapters = ref<Array<{ index: number; title: string }>>([]);
  const sourceDebugChapterResource = ref<ResourceDescriptor | null>(null);
  const sourceDebugChapterText = ref("");
  const sourceDebugSelectedChapterIndex = ref<number | null>(null);
  const sourceDebugStage = ref<SourceDebugResponse["stage"] | null>(null);
  const sourceDebugError = ref("");
  const sourceDebugKeyword = ref("");
  const sourceDebugBusy = ref(false);
  let sourceDebugGeneration = 0;

  function isCurrentSourceDebug(generation: number, sourceId: string): boolean {
    return sourceDebugGeneration === generation && sourceDebugTarget.value?.id === sourceId;
  }

  async function cleanupSourceDebugPreview(resourceId: string | undefined): Promise<void> {
    if (!resourceId) return;
    try {
      await clearSourceDebugPreview(resourceId);
    } catch {
      // 清理失败不能覆盖搜索或正文预览的实际结果。
    }
  }

  function releaseSourceDebugPreview(): void {
    const resourceId = sourceDebugChapterResource.value?.resourceId;
    sourceDebugChapterResource.value = null;
    sourceDebugChapterText.value = "";
    if (resourceId) void cleanupSourceDebugPreview(resourceId);
  }

  function resetSourceDebugInspection(): void {
    releaseSourceDebugPreview();
    sourceDebugSelectedResult.value = null;
    sourceDebugBook.value = null;
    sourceDebugChapters.value = [];
    sourceDebugSelectedChapterIndex.value = null;
    sourceDebugError.value = "";
  }

  function openSelectedSourceDebug(): void {
    if (selectedSourceRows.value.length !== 1 || sourceBatchBusy.value || sourceExportBusy.value || searchBusy.value) return;
    const source = sources.value.find((entry) => entry.id === selectedSourceRows.value[0]);
    if (!source) {
      notify("所选书源已不存在，请刷新列表后重试。", "error");
      return;
    }
    if (source.isRss === true) {
      notify("RSS 书源请从订阅文章页面调试。", "error");
      return;
    }
    if (!source.enabled) {
      notify("请先启用这个书源，再运行单源搜索。", "error");
      return;
    }

    sourceDebugGeneration += 1;
    releaseSourceDebugPreview();
    sourceDebugTarget.value = source;
    sourceDebugKeyword.value = "";
    sourceDebugResults.value = [];
    sourceDebugSearched.value = false;
    sourceDebugErrors.value = [];
    sourceDebugStage.value = null;
    resetSourceDebugInspection();
  }

  function selectSourceDebugSource(sourceId: string): void {
    const source = bookSourceCandidates.value.find((entry) => entry.id === sourceId);
    if (!source || sourceDebugBusy.value) return;
    sourceDebugGeneration += 1;
    releaseSourceDebugPreview();
    sourceDebugTarget.value = source;
    sourceDebugResults.value = [];
    sourceDebugSearched.value = false;
    sourceDebugErrors.value = [];
    sourceDebugStage.value = null;
    resetSourceDebugInspection();
  }

  function closeSourceDebug(): void {
    sourceDebugGeneration += 1;
    sourceDebugBusy.value = false;
    releaseSourceDebugPreview();
    sourceDebugTarget.value = null;
    sourceDebugKeyword.value = "";
    sourceDebugResults.value = [];
    sourceDebugSearched.value = false;
    sourceDebugErrors.value = [];
    resetSourceDebugInspection();
    sourceDebugStage.value = null;
  }

  async function startSourceDebugSearch(): Promise<void> {
    const source = sourceDebugTarget.value;
    const keyword = sourceDebugKeyword.value.trim();
    if (!source || sourceDebugBusy.value || shelfBatchRecoveryRequired.value) return;
    if (!keyword) {
      notify("请输入要测试的书名或关键词。", "error");
      return;
    }
    const currentSource = sources.value.find((entry) => entry.id === source.id);
    if (!currentSource || !currentSource.enabled || currentSource.isRss === true) {
      notify("这个书源当前不可用于书籍搜索，请确认它仍已启用。", "error");
      closeSourceDebug();
      return;
    }
    if (searchResultBatchBusy.value) {
      notify("请先等待加入书架操作完成。", "error");
      return;
    }

    const generation = ++sourceDebugGeneration;
    sourceDebugBusy.value = true;
    sourceDebugStage.value = "result";
    sourceDebugError.value = "";
    sourceDebugResults.value = [];
    sourceDebugSearched.value = false;
    sourceDebugErrors.value = [];
    resetSourceDebugInspection();
    try {
      const response = await searchBooks([currentSource.id], keyword, 1);
      if (!isCurrentSourceDebug(generation, currentSource.id)) return;
      sourceDebugStage.value = "catalog";
      const resource = await readResource<SearchResource>(response.resource);
      if (!isCurrentSourceDebug(generation, currentSource.id)) return;
      sourceDebugResults.value = Array.isArray(resource.results) ? resource.results : [];
      sourceDebugErrors.value = Array.isArray(resource.errors) ? resource.errors : response.errors;
      sourceDebugSearched.value = true;
      sourceDebugStage.value = null;
    } catch (error) {
      if (!isCurrentSourceDebug(generation, currentSource.id)) return;
      sourceDebugStage.value = sourceDebugResults.value.length ? "catalog" : "result";
      sourceDebugError.value = errorText(error);
    } finally {
      if (isCurrentSourceDebug(generation, currentSource.id)) sourceDebugBusy.value = false;
    }
  }

  async function inspectSourceDebugResult(result: SearchBookResult): Promise<void> {
    const source = sourceDebugTarget.value;
    if (!source || sourceDebugBusy.value) return;
    releaseSourceDebugPreview();
    const generation = ++sourceDebugGeneration;
    sourceDebugBusy.value = true;
    sourceDebugStage.value = "detail";
    sourceDebugError.value = "";
    sourceDebugSelectedResult.value = result;
    sourceDebugBook.value = null;
    sourceDebugChapters.value = [];
    sourceDebugSelectedChapterIndex.value = null;
    try {
      const response = await debugSearchResult(result.resultId);
      if (!isCurrentSourceDebug(generation, source.id)) return;
      if (!response.ok) {
        sourceDebugStage.value = response.stage ?? "detail";
        sourceDebugError.value = response.error ?? "无法读取书籍详情。";
        return;
      }
      sourceDebugBook.value = response.book ?? null;
      sourceDebugChapters.value = response.chapters ?? [];
      sourceDebugStage.value = "catalog";
    } catch (error) {
      if (!isCurrentSourceDebug(generation, source.id)) return;
      sourceDebugStage.value = "detail";
      sourceDebugError.value = errorText(error);
    } finally {
      if (isCurrentSourceDebug(generation, source.id)) sourceDebugBusy.value = false;
    }
  }

  async function previewSourceDebugChapter(chapter: { index: number; title: string }): Promise<void> {
    const source = sourceDebugTarget.value;
    const result = sourceDebugSelectedResult.value;
    if (!source || !result || sourceDebugBusy.value) return;
    releaseSourceDebugPreview();
    const generation = ++sourceDebugGeneration;
    sourceDebugBusy.value = true;
    sourceDebugStage.value = "chapter";
    sourceDebugError.value = "";
    sourceDebugSelectedChapterIndex.value = chapter.index;
    try {
      const response = await debugSearchResult(result.resultId, chapter.index);
      if (!isCurrentSourceDebug(generation, source.id)) {
        if (response.chapterResource) await cleanupSourceDebugPreview(response.chapterResource.resourceId);
        return;
      }
      if (!response.ok) {
        sourceDebugStage.value = response.stage ?? "chapter";
        sourceDebugError.value = response.error ?? "无法读取章节正文。";
        return;
      }
      const chapterResource = response.chapterResource ?? null;
      sourceDebugChapterResource.value = chapterResource;
      sourceDebugChapterText.value = "";
      if (chapterResource?.format === "json") {
        const document = await readResource<ChapterTextResource | ChapterRichTextResource>(chapterResource);
        if (!isCurrentSourceDebug(generation, source.id)) {
          await cleanupSourceDebugPreview(chapterResource.resourceId);
          return;
        }
        if (document.kind === "text" && typeof document.text === "string") {
          sourceDebugChapterText.value = document.text;
        } else if (document.kind === "richText" && typeof document.markup === "string") {
          const parsed = new DOMParser().parseFromString(document.markup, "text/html");
          sourceDebugChapterText.value = parsed.body.textContent ?? "";
        } else {
          throw new Error("章节调试资源格式无效。");
        }
      }
      sourceDebugStage.value = "chapter";
    } catch (error) {
      if (!isCurrentSourceDebug(generation, source.id)) return;
      // Failed JSON decoding must not leave an empty/stale preview on screen.
      // The backend owns the temporary file; request its disposal as well.
      releaseSourceDebugPreview();
      sourceDebugStage.value = "chapter";
      sourceDebugError.value = errorText(error);
    } finally {
      if (isCurrentSourceDebug(generation, source.id)) sourceDebugBusy.value = false;
    }
  }

  return {
    sourceDebugTarget,
    sourceDebugResults,
    sourceDebugSearched,
    sourceDebugErrors,
    sourceDebugSelectedResult,
    sourceDebugBook,
    sourceDebugChapters,
    sourceDebugChapterResource,
    sourceDebugChapterText,
    sourceDebugSelectedChapterIndex,
    sourceDebugStage,
    sourceDebugError,
    sourceDebugKeyword,
    sourceDebugBusy,
    openSelectedSourceDebug,
    selectSourceDebugSource,
    closeSourceDebug,
    startSourceDebugSearch,
    inspectSourceDebugResult,
    previewSourceDebugChapter,
  };
}
