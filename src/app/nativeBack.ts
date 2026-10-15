import { onBackButtonPress, exit } from "@tauri-apps/api/app";
import { invoke, isTauri } from "@tauri-apps/api/core";
import type { PluginListener } from "@tauri-apps/api/core";
import { watch, type Ref } from "vue";
import router, { routeNames } from "../router";

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

function topBackLayer(): HTMLElement | undefined {
  const layers = Array.from(document.querySelectorAll<HTMLElement>(
    "[data-back-layer],.modal-backdrop,.overlay-backdrop,[aria-modal='true']",
  )).reverse();
  return layers.find(element => {
    if (!visible(element)) return false;
    const rect = element.getBoundingClientRect();
    const top = document.elementFromPoint(
      Math.max(0, Math.min(innerWidth - 1, rect.left + rect.width / 2)),
      Math.max(0, Math.min(innerHeight - 1, rect.top + rect.height / 2)),
    );
    return !!top && element.contains(top);
  });
}

function hasVisibleBackLayer(): boolean {
  return Array.from(document.querySelectorAll<HTMLElement>(
    "[data-back-layer],.modal-backdrop,.overlay-backdrop,[aria-modal='true']",
  )).some(visible);
}

/** Only local layers belong here. Ordinary pages use the router's real history. */
export function localBackAction(): BackAction | null {
  const layer = topBackLayer();
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
  if (!isTauri()) return () => {};
  let disposed = false, handling = false;
  let listener: PluginListener | undefined;
  let removeGestureSync: (() => void) | undefined;
  if (isIos()) {
    let lastRequested: boolean | undefined;
    let pending = Promise.resolve();
    const syncGestures = () => {
      const route = router.currentRoute.value;
      const hasLocalPage = Array.from(document.querySelectorAll<HTMLButtonElement>("button[data-back-local]"))
        .some(visible);
      const sourceEditorEntry = route.name === routeNames.sources && Boolean(route.query.sourceEdit);
      const isTab = route.name === routeNames.home || route.name === routeNames.shelf
        || route.name === routeNames.settings;
      // A WebKit swipe previews its real history item. Local dialogs and pages
      // without a matching entry must not expose an unrelated screen beneath.
      const enabled = (Boolean(route.query[readerQuery]) || !isTab && route.name != null)
        && !hasVisibleBackLayer() && (!hasLocalPage || sourceEditorEntry);
      if (enabled === lastRequested) return;
      lastRequested = enabled;
      pending = pending.catch(() => {}).then(() => invoke("set_navigation_gestures", { enabled }))
        .then(() => {}).catch(error => console.error("iOS navigation gesture update failed:", error));
    };
    let queued = false;
    const queueGestureSync = () => {
      if (queued) return;
      queued = true;
      queueMicrotask(() => { queued = false; if (!disposed) syncGestures(); });
    };
    const observer = new MutationObserver(queueGestureSync);
    observer.observe(document.body, {
      childList: true, subtree: true, attributes: true, attributeFilter: ["style", "hidden", "inert"],
    });
    queueGestureSync();
    const removeAfterEach = router.afterEach((_to, _from, failure) => {
      if (!failure) queueGestureSync();
    });
    removeGestureSync = () => { observer.disconnect(); removeAfterEach(); };
  }
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
    // The mobile source editor has its own history entry. Let its route guard
    // handle the draft and reveal the source list captured by WKWebView.
    if (from.name === routeNames.sources && from.query.sourceEdit
      && from.path === to.path && from.query.sourceEdit !== to.query.sourceEdit
      && !hasVisibleBackLayer()) return;
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
        const route = router.currentRoute.value;
        if (route.name === routeNames.sources && route.query.sourceEdit && !hasVisibleBackLayer()) {
          const query = { ...route.query };
          delete query.sourceEdit;
          delete query.sourceImport;
          delete query.importUrl;
          const list = { name: routeNames.sources, query };
          if (router.options.history.state.back === router.resolve(list).fullPath) router.back();
          else void router.replace(list);
        } else if (action) await action.run();
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
  return () => { disposed = true; removeGuard(); removeGestureSync?.(); void listener?.unregister(); };
}
