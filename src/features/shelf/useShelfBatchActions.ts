import { moreInformation } from "../../api/moreInformation";
import { mainText } from "../../api/mainText";
import { readBook } from "../../api/database";
import { computed, ref, watch, type ComputedRef, type Ref } from "vue";
import { removeLibraryContent, setContentGroups, refreshContentLibrary } from "../books/contentLibraryState";
import { shelfBooks as shelf } from "./shelfBooks";
import type { ShelfResource } from "../../api/types";

type ShelfBook = ShelfResource["books"][number];
type ShelfBatchRemovalTarget = Pick<ShelfBook, "id" | "title" | "author">;

export interface ShelfCacheResult {
  completed: Array<{ id: string; title: string }>;
  failed: Array<{ id: string; title: string; message: string }>;
  skipped: number;
}

interface ShelfBatchActionsOptions {
  visibleBooks: ComputedRef<ShelfResource["books"]>;
  groups: ComputedRef<string[]>;
  recoveryRequired: Ref<boolean>;
  recoveryTitle: Ref<string>;
  recoveryMessage: Ref<string>;
  isShelfScreen: () => boolean;
  getRestoreRevision: () => number;
  notify: (message: string, kind?: "success" | "error") => void;
  errorText: (error: unknown) => string;
}

/** 管理书架多选、正文缓存、批量分组和移除。 */
export function useShelfBatchActions(options: ShelfBatchActionsOptions) {
  const {
    visibleBooks,
    groups,
    recoveryRequired,
    isShelfScreen,
    getRestoreRevision,
    notify,
    errorText,
  } = options;

  const selectionMode = ref(false);
  const selectedBookIds = ref<string[]>([]);
  const batchBusy = ref(false);
  const batchError = ref("");
  const cacheResult = ref<ShelfCacheResult | null>(null);
  const groupsOpen = ref(false);
  const groupBookIds = ref<string[]>([]);
  const groupDraft = ref<string[]>([]);
  const clearGroups = ref(false);
  const newGroup = ref("");
  const removeConfirmOpen = ref(false);
  const removalTargets = ref<ShelfBatchRemovalTarget[]>([]);
  const selectedVisibleBookIds = computed(() => {
    const selected = new Set(selectedBookIds.value);
    return visibleBooks.value.filter((book) => selected.has(book.id)).map((book) => book.id);
  });
  const selectedVisibleBooks = computed(() => {
    const selected = new Set(selectedVisibleBookIds.value);
    return visibleBooks.value.filter((book) => selected.has(book.id));
  });
  const selectedLocalBooks = computed(() => selectedVisibleBooks.value.filter((book) => book.id.startsWith("local-")));
  const allVisibleBooksSelected = computed(() =>
    visibleBooks.value.length > 0 && visibleBooks.value.every((book) => selectedBookIds.value.includes(book.id)));

  // 分组或筛选变化后只保留当前可见书籍；批量请求期间保留原选择快照。
  watch(visibleBooks, (books) => {
    if (!selectionMode.value || batchBusy.value) return;
    const visibleIds = new Set(books.map((book) => book.id));
    selectedBookIds.value = selectedBookIds.value.filter((id) => visibleIds.has(id));
  }, { flush: "sync" });

  function enterSelectionMode(): void {
    selectionMode.value = true;
    selectedBookIds.value = [];
    batchError.value = "";
  }

  function exitSelectionMode(): void {
    if (batchBusy.value) return;
    selectionMode.value = false;
    selectedBookIds.value = [];
    groupsOpen.value = false;
    groupBookIds.value = [];
    groupDraft.value = [];
    clearGroups.value = false;
    newGroup.value = "";
    batchError.value = "";
  }

  function toggleBookSelection(bookId: string): void {
    if (!selectionMode.value || batchBusy.value || !visibleBooks.value.some((book) => book.id === bookId)) return;
    selectedBookIds.value = selectedBookIds.value.includes(bookId)
      ? selectedBookIds.value.filter((id) => id !== bookId)
      : [...selectedBookIds.value, bookId];
    batchError.value = "";
  }

  function selectVisibleBooks(): void {
    if (batchBusy.value) return;
    selectedBookIds.value = [...new Set([...selectedBookIds.value, ...visibleBooks.value.map((book) => book.id)])];
    batchError.value = "";
  }

  function clearVisibleBookSelection(): void {
    if (batchBusy.value) return;
    const visibleIds = new Set(visibleBooks.value.map((book) => book.id));
    selectedBookIds.value = selectedBookIds.value.filter((id) => !visibleIds.has(id));
    batchError.value = "";
  }

  /** 每章直接调用统一正文接口，完整缓存资源，不创建下载任务。 */
  async function cacheSelectedBooks(): Promise<void> {
    const selected = [...selectedVisibleBooks.value];
    if (!selectionMode.value || !selected.length || selected.length > 256 || batchBusy.value || recoveryRequired.value) return;
    const revision = getRestoreRevision();
    const result: ShelfCacheResult = { completed: [], failed: [], skipped: 0 };
    batchBusy.value = true;
    batchError.value = "";
    cacheResult.value = result;
    try {
      await Promise.all(selected.map(async target => {
        try {
          const book = await readBook(target.id);
          if (revision !== getRestoreRevision()) return;
          if (!book.sourceId) { result.skipped++; return; }
          const chapters = await Promise.allSettled(book.chapters.filter(chapter => !chapter.volume)
            .map(chapter => mainText(book.id, chapter.id, true)));
          if (revision !== getRestoreRevision()) return;
          const failed = chapters.find(chapter => chapter.status === "rejected");
          if (failed?.status === "rejected") throw failed.reason;
          result.completed.push(target);
        } catch (error) {
          if (revision === getRestoreRevision()) result.failed.push({ ...target, message: errorText(error) });
        } finally {
          if (revision === getRestoreRevision()) cacheResult.value = { ...result };
        }
      }));
      if (revision === getRestoreRevision()) notify(`缓存完成 ${result.completed.length} 本，失败 ${result.failed.length} 本，本地书跳过 ${result.skipped} 本。`, result.failed.length ? "error" : "success");
    } finally {
      batchBusy.value = false;
    }
  }

  /** 批量刷新直接并发调用统一详情接口，不创建后台任务。 */
  async function refreshSelectedInformation(): Promise<void> {
    const selected = [...selectedVisibleBooks.value];
    if (!selectionMode.value || !selected.length || selected.length > 256 || batchBusy.value || recoveryRequired.value) return;
    const revision = getRestoreRevision();
    batchBusy.value = true;
    batchError.value = "";
    cacheResult.value = null;
    let updated = 0, skipped = 0;
    try {
      const results = await Promise.allSettled(selected.map(async target => {
        const book = await readBook(target.id);
        if (revision !== getRestoreRevision()) return;
        if (!book.sourceId || !book.bookUrl) { skipped++; return; }
        await moreInformation(book.sourceId, book.bookUrl);
        if (revision !== getRestoreRevision()) return;
        updated++;
        await refreshContentLibrary();
      }));
      if (revision !== getRestoreRevision()) return;
      const errors = results.flatMap((result, index) => result.status === "rejected"
        ? [`《${selected[index].title}》：${errorText(result.reason)}`] : []);
      batchError.value = errors.join("；");
      notify(`已更新 ${updated} 本，无书源跳过 ${skipped} 本，失败 ${errors.length} 本。`, errors.length ? "error" : "success");
    } finally {
      batchBusy.value = false;
    }
  }

  function openRemovalConfirm(): void {
    const targets = selectedVisibleBooks.value.map((book) => ({ id: book.id, title: book.title, author: book.author }));
    if (!targets.length || targets.length > 256 || batchBusy.value || recoveryRequired.value) return;
    removalTargets.value = targets;
    removeConfirmOpen.value = true;
  }

  function cancelRemoval(): void {
    if (batchBusy.value) return;
    removeConfirmOpen.value = false;
    removalTargets.value = [];
  }

  function submitRemoval(): void {
    const bookIds = new Set(removalTargets.value.map(book => book.id));
    if (!bookIds.size || batchBusy.value || recoveryRequired.value) return;
    removeLibraryContent(bookIds);
    selectedBookIds.value = selectedBookIds.value.filter(id => !bookIds.has(id));
    removalTargets.value = [];
    removeConfirmOpen.value = false;
    notify(`已从书架移除 ${bookIds.size} 本书，缓存和阅读记录保留。`);
  }

  function openGroups(): void {
    const bookIds = [...selectedVisibleBookIds.value];
    if (!bookIds.length || bookIds.length > 256 || batchBusy.value || recoveryRequired.value) return;
    groupBookIds.value = bookIds;
    const selectedBooks = shelf.value.books.filter((book) => bookIds.includes(book.id));
    groupDraft.value = groups.value.filter((group) => selectedBooks.every((book) => (book.groups ?? []).includes(group)));
    clearGroups.value = false;
    newGroup.value = "";
    batchError.value = "";
    groupsOpen.value = true;
  }

  function toggleGroup(group: string): void {
    if (batchBusy.value || recoveryRequired.value || clearGroups.value) return;
    groupDraft.value = groupDraft.value.includes(group)
      ? groupDraft.value.filter((entry) => entry !== group)
      : [...groupDraft.value, group];
    batchError.value = "";
  }

  function addGroup(): void {
    if (batchBusy.value || recoveryRequired.value) return;
    const group = newGroup.value.trim();
    if (!group) return;
    if (!groupDraft.value.includes(group)) groupDraft.value = [...groupDraft.value, group];
    clearGroups.value = false;
    newGroup.value = "";
    batchError.value = "";
  }

  function setClearGroups(clear: boolean): void {
    if (batchBusy.value || recoveryRequired.value) return;
    clearGroups.value = clear;
    if (clear) groupDraft.value = [];
    batchError.value = "";
  }

  function closeGroups(): void {
    if (batchBusy.value) return;
    groupsOpen.value = false;
    groupBookIds.value = [];
    groupDraft.value = [];
    clearGroups.value = false;
    newGroup.value = "";
  }

  function submitGroups(): void {
    const bookIds = [...groupBookIds.value];
    if (!bookIds.length || batchBusy.value || recoveryRequired.value || (!groupDraft.value.length && !clearGroups.value)) return;
    const nextGroups = clearGroups.value ? [] : [...groupDraft.value];
    setContentGroups(bookIds, nextGroups);
    selectedBookIds.value = selectedBookIds.value.filter((id) => !bookIds.includes(id));
    closeGroups();
    notify(`已为 ${bookIds.length} 本书替换分组。`);
    if (!isShelfScreen()) exitSelectionMode();
  }

  return {
    selectionMode,
    selectedBookIds,
    selectedVisibleBookIds,
    selectedVisibleBooks,
    selectedLocalBooks,
    allVisibleBooksSelected,
    batchBusy,
    batchError,
    cacheResult,
    groupsOpen,
    groupBookIds,
    groupDraft,
    clearGroups,
    newGroup,
    removeConfirmOpen,
    removalTargets,
    enterSelectionMode,
    exitSelectionMode,
    toggleBookSelection,
    selectVisibleBooks,
    clearVisibleBookSelection,
    cacheSelectedBooks,
    refreshSelectedInformation,
    openRemovalConfirm,
    cancelRemoval,
    submitRemoval,
    openGroups,
    toggleGroup,
    addGroup,
    setClearGroups,
    closeGroups,
    submitGroups,
  };
}
