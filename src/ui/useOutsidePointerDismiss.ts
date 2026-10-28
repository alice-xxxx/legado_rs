import { onBeforeUnmount, onMounted, watch } from "vue";

const openMenus = new Set<(event: KeyboardEvent) => void>();

export function popoverOwner(target: EventTarget | null): HTMLElement | null {
  const id = target instanceof Element ? target.closest<HTMLElement>("[data-popover-owner]")?.dataset.popoverOwner : undefined;
  return id ? document.getElementById(id) : null;
}

export function useOutsidePointerDismiss(
  isOpen: () => boolean,
  containsTarget: (target: EventTarget | null) => boolean,
  dismiss: () => void,
): void {
  let returnFocus: HTMLElement | null = null;
  watch(isOpen, open => {
    if (open) openMenus.add(onKeydown);
    else openMenus.delete(onKeydown);
    if (open) returnFocus = document.activeElement instanceof HTMLElement ? document.activeElement : null;
  });
  const onPointerDown = (event: PointerEvent): void => {
    if (!isOpen() || containsTarget(event.target) || containsTarget(popoverOwner(event.target))) return;
    dismiss();
  };

  const onKeydown = (event: KeyboardEvent): void => {
    if ([...openMenus].pop() !== onKeydown) return;
    if (event.key !== "Escape" || event.defaultPrevented || !isOpen()) return;
    event.preventDefault();
    event.stopPropagation();
    dismiss();
    if (returnFocus?.isConnected && !returnFocus.matches("input,textarea,select,[contenteditable]")) returnFocus.focus({ preventScroll: true });
  };
  onMounted(() => {
    if (isOpen()) openMenus.add(onKeydown);
    document.addEventListener("pointerdown", onPointerDown);
    document.addEventListener("keydown", onKeydown, true);
  });
  onBeforeUnmount(() => {
    openMenus.delete(onKeydown);
    document.removeEventListener("pointerdown", onPointerDown);
    document.removeEventListener("keydown", onKeydown, true);
  });
}
