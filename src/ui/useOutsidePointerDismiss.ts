import { onBeforeUnmount, onMounted } from "vue";

export function useOutsidePointerDismiss(
  isOpen: () => boolean,
  containsTarget: (target: EventTarget | null) => boolean,
  dismiss: () => void,
): void {
  const onPointerDown = (event: PointerEvent): void => {
    if (!isOpen() || containsTarget(event.target)) return;
    dismiss();
  };

  onMounted(() => document.addEventListener("pointerdown", onPointerDown));
  onBeforeUnmount(() => document.removeEventListener("pointerdown", onPointerDown));
}
