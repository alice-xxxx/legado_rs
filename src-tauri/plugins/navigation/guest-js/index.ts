import { onBackButtonPress } from "@tauri-apps/api/app";
import { addPluginListener, invoke, isTauri } from "@tauri-apps/api/core";

export type Unlisten = () => void | Promise<void>;
export interface BackGestureEvent {
  phase: "start" | "move" | "end";
  progress: number;
  commit?: boolean;
}
export interface BackRequest {
  source: "system" | "keyboard";
  canGoBack?: boolean;
}

function isIOS(): boolean {
  return /iPhone|iPad|iPod/.test(navigator.userAgent)
    || /Macintosh/.test(navigator.userAgent) && navigator.maxTouchPoints > 1;
}

/** Whether this host exposes a native WebView history gesture switch. */
export function supportsHistoryGestures(): boolean {
  return isTauri() && !/Android/.test(navigator.userAgent);
}

/** Returns false when the host has no native history gesture switch. */
export function setNavigationGestures(enabled: boolean, local = false): Promise<boolean> {
  return !supportsHistoryGestures() ? Promise.resolve(false)
    : invoke("plugin:navigation|set_gestures", { enabled, local });
}

/** Interactive local-page progress is currently supplied by iOS. */
export async function onBackGesture(handler: (event: BackGestureEvent) => void): Promise<Unlisten> {
  if (!isTauri() || !isIOS()) return () => {};
  const listener = await addPluginListener("navigation", "backGesture", handler);
  return () => listener.unregister();
}

/** Android system Back and desktop/tablet keyboard Back share one consumer callback. */
export async function onBackRequested(handler: (event: BackRequest) => void): Promise<Unlisten> {
  const listener = isTauri() && /Android/.test(navigator.userAgent)
    ? await onBackButtonPress(event => handler({ ...event, source: "system" })) : undefined;
  const apple = /Macintosh|iPhone|iPad|iPod/.test(navigator.userAgent);
  const onKey = (event: KeyboardEvent) => {
    if (event.defaultPrevented || event.repeat || event.isComposing) return;
    const back = apple
      ? event.metaKey && !event.altKey && !event.ctrlKey && !event.shiftKey && event.key === "["
      : event.altKey && !event.ctrlKey && !event.metaKey && !event.shiftKey && event.key === "ArrowLeft";
    if (back) {
      event.preventDefault();
      handler({ source: "keyboard" });
    }
  };
  window.addEventListener("keydown", onKey);
  return async () => {
    window.removeEventListener("keydown", onKey);
    await listener?.unregister();
  };
}
