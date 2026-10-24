import type { BackGestureEvent } from "tauri-plugin-navigation-api";

/** Move the current local page to reveal its live parent underneath. */
export function createLocalBackGesture(getTarget: () => {
  element: HTMLElement;
  back: () => void | Promise<void>;
} | null) {
  let active: { animation: Animation; back: () => void | Promise<void> } | null = null;

  function cancel(): void {
    active?.animation.cancel();
    active = null;
  }

  async function handle(event: BackGestureEvent): Promise<void> {
    if (event.phase === "start") {
      cancel();
      const target = getTarget();
      if (!target) return;
      const animation = target.element.animate([
        { transform: "translateX(0)", boxShadow: "-8px 0 24px rgb(0 0 0 / 16%)" },
        { transform: "translateX(100%)", boxShadow: "-8px 0 24px rgb(0 0 0 / 16%)" },
      ], { duration: 220, fill: "both" });
      animation.pause();
      animation.currentTime = 0;
      active = { animation, back: target.back };
    }
    const current = active;
    if (!current) return;
    current.animation.currentTime = Math.max(0, Math.min(1, event.progress)) * 220;
    if (event.phase !== "end") return;
    current.animation.playbackRate = event.commit ? 1 : -1;
    current.animation.play();
    try {
      await current.animation.finished;
      if (active === current && event.commit) await current.back();
    } catch (error) {
      if (!(error instanceof DOMException && error.name === "AbortError")) throw error;
    } finally {
      if (active === current) cancel();
    }
  }

  return { handle, cancel };
}
