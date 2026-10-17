import { onBeforeUnmount, onMounted, watch } from "vue";

export function useOutsidePointerDismiss(
  isOpen: () => boolean,
  containsTarget: (target: EventTarget | null) => boolean,
  dismiss: () => void,
): void {
  let returnFocus: HTMLElement | null = null;
  watch(isOpen, open => {
    if (open) returnFocus = document.activeElement instanceof HTMLElement ? document.activeElement : null;
  });
  const onPointerDown = (event: PointerEvent): void => {
    if (!isOpen() || containsTarget(event.target)) return;
    dismiss();
  };

  const onKeydown = (event: KeyboardEvent): void => {
    if (event.key !== "Escape" || event.defaultPrevented || !isOpen()) return;
    event.preventDefault();
    event.stopPropagation();
    dismiss();
    if (returnFocus?.isConnected) returnFocus.focus({ preventScroll: true });
  };
  onMounted(() => {
    document.addEventListener("pointerdown", onPointerDown);
    document.addEventListener("keydown", onKeydown);
  });
  onBeforeUnmount(() => {
    document.removeEventListener("pointerdown", onPointerDown);
    document.removeEventListener("keydown", onKeydown);
  });
}
