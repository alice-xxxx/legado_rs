import { onBackButtonPress, exit } from "@tauri-apps/api/app";
import { isTauri } from "@tauri-apps/api/core";
import type { PluginListener } from "@tauri-apps/api/core";
import { watch, type Ref } from "vue";
import router from "../router";

interface BackAction { run: () => void | Promise<void>; }
let readerAction: { isOpen: () => boolean; action: BackAction } | null = null;
const readerQuery = "_reader";

function isIos(): boolean {
  return /iPhone|iPad|iPod/.test(navigator.userAgent)
    || /Macintosh/.test(navigator.userAgent) && navigator.maxTouchPoints > 1;
}

function visible(element: Element): boolean {
  return element.isConnected && element.getClientRects().length > 0
    && !element.closest("[inert]") && getComputedStyle(element).visibility !== "hidden";
}

function buttonAction(button: HTMLButtonElement | undefined): BackAction {
  // A disabled return control consumes Back without bypassing its busy guard.
  return { run: () => { if (button && !button.disabled && visible(button)) button.click(); } };
}

/** Only local layers belong here. Ordinary pages use the router's real history. */
export function localBackAction(): BackAction | null {
  const layers = Array.from(document.querySelectorAll<HTMLElement>(
    "[data-back-layer],.modal-backdrop,.overlay-backdrop,[aria-modal='true']",
  )).reverse();
  const layer = layers.find(element => {
    if (!visible(element)) return false;
    const rect = element.getBoundingClientRect();
    const top = document.elementFromPoint(
      Math.max(0, Math.min(innerWidth - 1, rect.left + rect.width / 2)),
      Math.max(0, Math.min(innerHeight - 1, rect.top + rect.height / 2)),
    );
    return !!top && element.contains(top);
  });
  if (layer) return buttonAction(Array.from(layer.querySelectorAll<HTMLButtonElement>("button[data-back-action]"))
    .reverse().find(visible));
  if (readerAction?.isOpen()) return readerAction.action;
  const local = Array.from(document.querySelectorAll<HTMLButtonElement>("button[data-back-local]"))
    .reverse().find(visible);
  return local ? buttonAction(local) : null;
}

export function installReaderNativeBack(options: {
  visible: Ref<boolean>;
  close: () => Promise<void>;
}): () => void {
  const session = { isOpen: () => options.visible.value, action: { run: options.close } };
  readerAction = session;
  // WKWebView needs a real history entry to swipe away an overlaid reader,
  // including a book opened from the app's first page.
  const stop = watch(options.visible, async open => {
    if (!isTauri() || !isIos()) return;
    const route = router.currentRoute.value;
    if (open && !route.query[readerQuery]) {
      await router.push({ path: route.path, query: { ...route.query, [readerQuery]: "1" }, hash: route.hash });
    } else if (!open && route.query[readerQuery]
      && router.options.history.state.current === route.fullPath) {
      router.back();
    }
  });
  return () => { stop(); if (readerAction === session) readerAction = null; };
}

export function installNativeBackNavigation(): () => void {
  if (!isTauri() || !(/Android/.test(navigator.userAgent) || isIos())) return () => {};
  let disposed = false, handling = false;
  let listener: PluginListener | undefined;
  const removeGuard = router.beforeEach(async (to, from) => {
    // During a native history traversal the WebView state already names `to`.
    // Vue Router restores that entry when a modal consumes Back or saving fails.
    if (to.query[readerQuery] && !readerAction?.isOpen()) {
      const query = { ...to.query };
      delete query[readerQuery];
      return { path: to.path, query, hash: to.hash, replace: true };
    }
    const popping = to.fullPath !== from.fullPath && router.options.history.state.current === to.fullPath;
    if (!popping) return;
    const action = localBackAction();
    if (!action) return;
    await action.run();
    const closesReaderEntry = from.query[readerQuery] && !to.query[readerQuery]
      && from.path === to.path && !readerAction?.isOpen();
    return closesReaderEntry ? undefined : false;
  });

  if (/Android/.test(navigator.userAgent)) {
    void onBackButtonPress(async ({ canGoBack }) => {
      if (disposed || handling) return;
      handling = true;
      try {
        const action = localBackAction();
        if (action) await action.run();
        else if (canGoBack && typeof router.options.history.state.back === "string") router.back();
        else {
          const pageBack = Array.from(document.querySelectorAll<HTMLButtonElement>("button[data-back-action]"))
            .reverse().find(visible);
          if (pageBack) buttonAction(pageBack).run();
          else await exit(0);
        }
      } finally { handling = false; }
    }).then(value => {
      if (disposed) void value.unregister();
      else listener = value;
    }).catch(error => console.error("Native back registration failed:", error));
  }
  return () => { disposed = true; removeGuard(); void listener?.unregister(); };
}
