import { onBeforeUnmount, ref, type ComputedRef, type Ref } from "vue";
import { createShelfGroup, deleteShelfGroup, renameShelfGroup } from "../../api/shelf";
import { type ResourceDescriptor } from "../../api/types";

interface ShelfGroupActionsOptions {
  groups: ComputedRef<string[]>;
  activeGroup: Ref<string>;
  selectionMode: Readonly<Ref<boolean>>;
  batchBusy: Readonly<Ref<boolean>>;
  refreshShelfFromDescriptor: (descriptor: ResourceDescriptor) => Promise<void>;
  getRestoreRevision: () => number;
  toggleBookSelection: (bookId: string) => void;
  openBookInfo: (bookId: string) => void;
  continueReading: (bookId: string) => void;
  notify: (message: string, kind?: "success" | "error") => void;
  errorText: (error: unknown) => string;
}

/** 管理书架分组和封面长按交互。 */
export function useShelfGroupActions(options: ShelfGroupActionsOptions) {
  const {
    groups,
    activeGroup,
    selectionMode,
    batchBusy,
    refreshShelfFromDescriptor,
    getRestoreRevision,
    toggleBookSelection,
    openBookInfo,
    continueReading,
    notify,
    errorText,
  } = options;
  const groupMutationBusy = ref(false);
  let longPressTimer: ReturnType<typeof setTimeout> | null = null;
  let longPressPointer: { bookId: string; pointerId: number; x: number; y: number } | null = null;
  let suppressedClickBookId: string | null = null;
  let suppressedClickTimer: ReturnType<typeof setTimeout> | null = null;

  async function createGroup(groupName: string): Promise<void> {
    const normalized = groupName.trim();
    if (!normalized || groupMutationBusy.value) return;
    if (groups.value.includes(normalized)) {
      notify("这个分组已经存在。", "error");
      return;
    }
    const restoreRevision = getRestoreRevision();
    groupMutationBusy.value = true;
    try {
      const resource = await createShelfGroup(normalized);
      if (restoreRevision !== getRestoreRevision()) return;
      await refreshShelfFromDescriptor(resource);
      if (restoreRevision === getRestoreRevision()) notify("书架分组已创建，空分组也会保留。");
    } catch (error) {
      if (restoreRevision === getRestoreRevision()) notify(`创建分组失败：${errorText(error)}`, "error");
    } finally {
      groupMutationBusy.value = false;
    }
  }

  async function renameGroup(oldName: string, newName: string): Promise<void> {
    const normalized = newName.trim();
    if (!normalized || groupMutationBusy.value) return;
    const restoreRevision = getRestoreRevision();
    groupMutationBusy.value = true;
    try {
      const resource = await renameShelfGroup(oldName, normalized);
      if (restoreRevision !== getRestoreRevision()) return;
      await refreshShelfFromDescriptor(resource);
      if (restoreRevision !== getRestoreRevision()) return;
      if (activeGroup.value === oldName) activeGroup.value = normalized;
      notify("分组名称已更新。");
    } catch (error) {
      if (restoreRevision === getRestoreRevision()) notify(`分组重命名失败：${errorText(error)}`, "error");
    } finally {
      groupMutationBusy.value = false;
    }
  }

  async function removeShelfGroup(groupName: string): Promise<void> {
    if (groupMutationBusy.value) return;
    const restoreRevision = getRestoreRevision();
    groupMutationBusy.value = true;
    try {
      const resource = await deleteShelfGroup(groupName);
      if (restoreRevision !== getRestoreRevision()) return;
      await refreshShelfFromDescriptor(resource);
      if (restoreRevision !== getRestoreRevision()) return;
      if (activeGroup.value === groupName) activeGroup.value = "";
      notify("分组已删除，书籍仍保留在书架。");
    } catch (error) {
      if (restoreRevision === getRestoreRevision()) notify(`删除分组失败：${errorText(error)}`, "error");
    } finally {
      groupMutationBusy.value = false;
    }
  }

  function clearSuppressedClick(): void {
    if (suppressedClickTimer) clearTimeout(suppressedClickTimer);
    suppressedClickTimer = null;
    suppressedClickBookId = null;
  }

  function cancelShelfCoverLongPress(): void {
    if (longPressTimer) clearTimeout(longPressTimer);
    longPressTimer = null;
    longPressPointer = null;
  }

  function startShelfCoverLongPress(bookId: string, event: PointerEvent): void {
    if ((event.pointerType === "mouse" && event.button !== 0) || selectionMode.value || batchBusy.value) return;
    cancelShelfCoverLongPress();
    clearSuppressedClick();
    longPressPointer = { bookId, pointerId: event.pointerId, x: event.clientX, y: event.clientY };
    longPressTimer = setTimeout(() => {
      if (longPressPointer?.pointerId !== event.pointerId) return;
      longPressTimer = null;
      suppressedClickBookId = bookId;
      suppressedClickTimer = setTimeout(clearSuppressedClick, 1000);
      openBookInfo(bookId);
    }, 550);
  }

  function trackShelfCoverLongPress(event: PointerEvent): void {
    const pointer = longPressPointer;
    if (!pointer || pointer.pointerId !== event.pointerId || !longPressTimer) return;
    if (Math.hypot(event.clientX - pointer.x, event.clientY - pointer.y) > 10) cancelShelfCoverLongPress();
  }

  function finishShelfCoverLongPress(event: PointerEvent): void {
    if (longPressPointer?.pointerId !== event.pointerId) return;
    if (longPressTimer) clearTimeout(longPressTimer);
    longPressTimer = null;
    longPressPointer = null;
  }

  function handleShelfCoverPointerLeave(event: PointerEvent): void {
    if (event.pointerType === "mouse" && longPressTimer) cancelShelfCoverLongPress();
  }

  function handleShelfCoverClick(bookId: string): void {
    if (suppressedClickBookId === bookId) {
      clearSuppressedClick();
      return;
    }
    if (selectionMode.value) toggleBookSelection(bookId);
    else continueReading(bookId);
  }

  function dispose(): void {
    cancelShelfCoverLongPress();
    clearSuppressedClick();
  }

  onBeforeUnmount(dispose);

  return {
    groupMutationBusy,
    createGroup,
    renameGroup,
    removeShelfGroup,
    startShelfCoverLongPress,
    trackShelfCoverLongPress,
    finishShelfCoverLongPress,
    cancelShelfCoverLongPress,
    handleShelfCoverPointerLeave,
    handleShelfCoverClick,
    dispose,
  };
}
