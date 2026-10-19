import { mainText } from "../../api/mainText";
import { ref, type Ref } from "vue";
import { type ResourceDescriptor, type SearchBookResult, type BookResource, type SourceMetadata } from "../../api/types";
import { search, releaseSearchResults } from "../../api/search";
import { presentSearchResults } from "../discovery/searchPresentation";
import { moreInformation } from "../../api/moreInformation";
import { readMoreInformation } from "../../api/database";
import { readTextResource } from "../../api/resources";
import { sources, readSourceDefinitionJson } from "./sourcesStore";
import { readSourceEditorDraft } from "./sourceEditorDraft";
import { shelfBatchRecoveryRequired } from "../../app/recoveryState";

interface SourceDebugDependencies {
  /** 书源列表中当前选中的行。 */
  selectedSourceRows: Ref<string[]>;
  sourceBatchBusy: Ref<boolean>;
  sourceExportBusy: Ref<boolean>;
  searchBusy: Readonly<Ref<boolean>>;
  searchResultBatchBusy: Ref<boolean>;
  notify: (message: string, kind?: "success" | "error") => void;
  errorText: (error: unknown) => string;
}

/** 处理单个书源的搜索、详情和章节预览调试流程。 */
export function useSourceDebugActions({
  selectedSourceRows,
  sourceBatchBusy,
  sourceExportBusy,
  searchBusy,
  searchResultBatchBusy,
  notify,
  errorText,
}: SourceDebugDependencies) {
  const sourceDebugTarget = ref<SourceMetadata | null>(null);
  const sourceDebugResults = ref<SearchBookResult[]>([]);
  const sourceDebugSearched = ref(false);
  const sourceDebugErrors = ref<Array<{ sourceId?: string; message: string }>>([]);
  const sourceDebugSelectedResult = ref<SearchBookResult | null>(null);
  const sourceDebugBook = ref<BookResource | null>(null);
  const sourceDebugChapters = ref<Array<{ index: number; title: string }>>([]);
  const sourceDebugChapterResource = ref<ResourceDescriptor | null>(null);
  const sourceDebugChapterText = ref("");
  const sourceDebugSelectedChapterIndex = ref<number | null>(null);
  const sourceDebugStage = ref<"result" | "source" | "detail" | "catalog" | "chapter" | null>(null);
  const sourceDebugError = ref("");
  const sourceDebugKeyword = ref("");
  const sourceDebugKeywordLoading = ref(false);
  const sourceDebugBusy = ref(false);
  let information: ReturnType<typeof moreInformation> | null = null;
  let text: ReturnType<typeof mainText> | null = null;
  let sourceDebugGeneration = 0;
  let searchId: string | null = null;

  function isCurrentSourceDebug(generation: number, sourceId: string): boolean {
    return sourceDebugGeneration === generation && sourceDebugTarget.value?.id === sourceId;
  }

  function releaseSourceDebugPreview(): void {
    sourceDebugChapterResource.value = null;
    sourceDebugChapterText.value = "";
  }

  function resetSourceDebugInspection(): void {
    releaseSourceDebugPreview();
    sourceDebugSelectedResult.value = null;
    sourceDebugBook.value = null;
    sourceDebugChapters.value = [];
    sourceDebugSelectedChapterIndex.value = null;
    sourceDebugError.value = "";
  }

  async function loadSourceDebugKeyword(source: SourceMetadata, generation: number): Promise<void> {
    sourceDebugKeyword.value = "我的";
    sourceDebugKeywordLoading.value = true;
    try {
      const json = await readSourceDefinitionJson(source.id);
      const draft = readSourceEditorDraft(json, [{ key: "ruleSearch.checkKeyWord", label: "校验关键词", kind: "text" }], source.enabled);
      if (isCurrentSourceDebug(generation, source.id)) sourceDebugKeyword.value = draft["ruleSearch.checkKeyWord"]?.trim() || "我的";
    } catch {
      // The optional keyword must not prevent testing if its saved definition cannot be read.
    } finally {
      if (isCurrentSourceDebug(generation, source.id)) sourceDebugKeywordLoading.value = false;
    }
  }

  function openSelectedSourceDebug(): void {
    if (selectedSourceRows.value.length !== 1 || sourceBatchBusy.value || sourceExportBusy.value || searchBusy.value) return;
    const source = sources.value.find((entry) => entry.id === selectedSourceRows.value[0]);
    if (!source) {
      notify("所选源已不存在，请刷新列表后重试。", "error");
      return;
    }
    if (source.isRss === true) {
      notify("RSS 源请从订阅文章页面调试。", "error");
      return;
    }
    if (!source.enabled) {
      notify("请先启用这个源，再运行单源搜索。", "error");
      return;
    }

    sourceDebugGeneration += 1;
    releaseSourceDebugPreview();
    sourceDebugTarget.value = source;
    sourceDebugResults.value = [];
    sourceDebugSearched.value = false;
    sourceDebugErrors.value = [];
    sourceDebugStage.value = null;
    resetSourceDebugInspection();
    void loadSourceDebugKeyword(source, sourceDebugGeneration);
  }

  function closeSourceDebug(): void {
    void text?.cancel().catch(console.error);
    if (sourceDebugBusy.value) void information?.cancel().catch(console.error);
    if (searchId) void releaseSearchResults(searchId).catch(console.error);
    searchId = null;
    sourceDebugGeneration += 1;
    sourceDebugBusy.value = false;
    releaseSourceDebugPreview();
    sourceDebugTarget.value = null;
    sourceDebugKeyword.value = "";
    sourceDebugKeywordLoading.value = false;
    sourceDebugResults.value = [];
    sourceDebugSearched.value = false;
    sourceDebugErrors.value = [];
    resetSourceDebugInspection();
    sourceDebugStage.value = null;
  }

  async function startSourceDebugSearch(): Promise<void> {
    const source = sourceDebugTarget.value;
    const keyword = sourceDebugKeyword.value.trim();
    if (!source || sourceDebugBusy.value || sourceDebugKeywordLoading.value || shelfBatchRecoveryRequired.value) return;
    if (!keyword) {
      notify("请输入要测试的书名或关键词。", "error");
      return;
    }
    const currentSource = sources.value.find((entry) => entry.id === source.id);
    if (!currentSource || !currentSource.enabled || currentSource.isRss === true) {
      notify("这个源当前不可用于书籍搜索，请确认它仍已启用。", "error");
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
      if (searchId) void releaseSearchResults(searchId).catch(console.error);
      const id = await search([currentSource.id], keyword, 1, "none", update => {
        if (!isCurrentSourceDebug(generation, currentSource.id)) {
          void releaseSearchResults(update.searchId).catch(console.error);
          return;
        }
        searchId = update.searchId;
        sourceDebugResults.value = presentSearchResults(update.results, keyword);
        if (update.error) sourceDebugErrors.value.push({ sourceId: update.sourceId, message: update.error });
        sourceDebugSearched.value = true;

      });
      if (!isCurrentSourceDebug(generation, currentSource.id)) {
        await releaseSearchResults(id);
        return;
      }
      searchId = id;
      sourceDebugBusy.value = false;
      sourceDebugStage.value = null;
    } catch (error) {
      if (!isCurrentSourceDebug(generation, currentSource.id)) return;
      sourceDebugStage.value = sourceDebugResults.value.length ? "catalog" : "result";
      sourceDebugError.value = errorText(error);
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
      information = moreInformation(result.sourceId, result.bookUrl);
      await information;
      if (!isCurrentSourceDebug(generation, source.id)) return;
      const book = await readMoreInformation(result.sourceId, result.bookUrl);
      if (!isCurrentSourceDebug(generation, source.id)) return;
      sourceDebugBook.value = book;
      sourceDebugChapters.value = book.chapters;
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
      const saved = sourceDebugBook.value;
      if (!saved) return;
      const target = saved.chapters[chapter.index];
      if (!target) return;
      text = mainText(saved.id, target.id);
      await text;
      if (!isCurrentSourceDebug(generation, source.id)) return;
      const book = await readMoreInformation(result.sourceId, result.bookUrl);
      if (!isCurrentSourceDebug(generation, source.id)) return;
      const chapterResource = book.chapters[chapter.index]?.resource ?? null;
      sourceDebugChapterResource.value = chapterResource;
      sourceDebugChapterText.value = "";
      if (chapterResource) {
        const body = await readTextResource(chapterResource);
        if (!isCurrentSourceDebug(generation, source.id)) {
          return;
        }
        sourceDebugChapterText.value = chapterResource.format === "html"
          ? new DOMParser().parseFromString(body, "text/html").body.textContent ?? "" : body;
      }
      sourceDebugStage.value = "chapter";
    } catch (error) {
      if (!isCurrentSourceDebug(generation, source.id)) return;
      // 读取失败时清空本地预览，已保存的章节仍由统一缓存管理。
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
    sourceDebugKeywordLoading,
    sourceDebugBusy,
    openSelectedSourceDebug,
    closeSourceDebug,
    startSourceDebugSearch,
    inspectSourceDebugResult,
    previewSourceDebugChapter,
  };
}
