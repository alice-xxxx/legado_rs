import { onBeforeUnmount, type ComputedRef, type Ref } from "vue";
import { contentLibrary } from "../books/contentLibraryState";

interface ShelfGroupActionsOptions {
  groups: ComputedRef<string[]>;
  activeGroup: Ref<string>;
  selectionMode: Readonly<Ref<boolean>>;
  batchBusy: Readonly<Ref<boolean>>;
  toggleBookSelection: (bookId: string) => void;
  openBookInfo: (bookId: string) => void;
  continueReading: (bookId: string) => void;
  notify: (message: string, kind?: "success" | "error") => void;
}

/** 管理书架分组和封面长按交互。 */
export function useShelfGroupActions(options: ShelfGroupActionsOptions) {
  const {
    groups,
    activeGroup,
    selectionMode,
    batchBusy,
    toggleBookSelection,
    openBookInfo,
    continueReading,
    notify,
  } = options;
  let longPressTimer: ReturnType<typeof setTimeout> | null = null;
  let longPressPointer: { bookId: string; pointerId: number; x: number; y: number } | null = null;
  let suppressedClickBookId: string | null = null;
  let suppressedClickTimer: ReturnType<typeof setTimeout> | null = null;

  function createGroup(groupName: string): void {
    const normalized = groupName.trim();
    if (!normalized || batchBusy.value) return;
    if (groups.value.includes(normalized)) {
      notify("这个分组已经存在。", "error");
      return;
    }
    (contentLibrary.value.groups ??= []).push({ name: normalized, id: [] });
    notify("书架分组已创建，空分组也会保留。");
  }

  function renameGroup(oldName: string, newName: string): void {
    const normalized = newName.trim();
    if (!normalized || batchBusy.value) return;
    const group = contentLibrary.value.groups?.find(group => group.name === oldName);
    if (!group) return;
    const duplicate = contentLibrary.value.groups?.find(group => group.name === normalized && group.name !== oldName);
    if (duplicate && duplicate !== group) {
      duplicate.id = [...new Set([...duplicate.id, ...group.id])];
      contentLibrary.value.groups = contentLibrary.value.groups!.filter(entry => entry !== group);
    } else group.name = normalized;
    if (activeGroup.value === oldName) activeGroup.value = normalized;
    notify("分组名称已更新。");
  }

  function removeShelfGroup(groupName: string): void {
    if (batchBusy.value) return;
    contentLibrary.value.groups = contentLibrary.value.groups?.filter(group => group.name !== groupName);
    if (activeGroup.value === groupName) activeGroup.value = "";
    notify("分组已删除，书籍仍保留在书架。");
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
