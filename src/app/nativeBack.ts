import { exit } from "@tauri-apps/api/app";
import { isTauri } from "@tauri-apps/api/core";
import { onBackGesture, onBackRequested, setNavigationGestures, supportsHistoryGestures, type Unlisten } from "tauri-plugin-navigation-api";
import { watch, type Ref } from "vue";
import router, { routeNames } from "../router";
import { createLocalBackGesture } from "./localBackGesture";

interface BackAction { run: () => void | Promise<void>; }
let readerAction: { isOpen: () => boolean; menuOpen: () => boolean; action: BackAction } | null = null;
const readerQuery = "_reader";

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
  menuOpen: Readonly<Ref<boolean>>;
  close: () => Promise<void>;
}): () => void {
  const session = { isOpen: () => options.visible.value, menuOpen: () => options.menuOpen.value,
    action: { run: async () => {
      // Reading consumes Back so an edge swipe cannot also turn a page and exit.
      // Subpages are handled by topBackLayer; the menu itself exits the reader.
      if (options.menuOpen.value) await options.close();
    } } };
  readerAction = session;
  // Give WebKit a history entry to preview the page beneath the reader.
  const stop = watch(options.visible, async open => {
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
  const initial = router.currentRoute.value;
  if (initial.query[readerQuery] && !readerAction?.isOpen()) {
    const query = { ...initial.query };
    delete query[readerQuery];
    void router.replace({ path: initial.path, query, hash: initial.hash });
  }
  let disposed = false, handling = false;
  let removeBackListener: Unlisten | undefined;
  let removeGestureListener: Unlisten | undefined;
  let removeGestureSync: (() => void) | undefined;
  let prepareTabGesture: (() => Promise<void>) | undefined;
  const isTabRoute = (name: unknown) => name === routeNames.home
    || name === routeNames.shelf || name === routeNames.settings;
  if (supportsHistoryGestures()) {
    let lastRequested: string | undefined;
    let pending = Promise.resolve();
    let tabPushPending = false;
    const requestGestureState = (enabled: boolean, local = false): Promise<void> => {
      const state = `${enabled}:${local}`;
      if (state === lastRequested) return pending;
      lastRequested = state;
      pending = pending.then(async () => {
        if (disposed || lastRequested !== state) return;
        try {
          await setNavigationGestures(enabled, local);
        } catch (error) {
          if (lastRequested === state) lastRequested = undefined;
          console.error("Navigation gesture update failed:", error);
        }
      });
      return pending;
    };
    const syncGestures = () => {
      if (tabPushPending) return;
      const route = router.currentRoute.value;
      if (readerAction?.isOpen()) {
        const layer = topBackLayer();
        // The menu uses WebKit's real history preview of the library. Local
        // subpages slide over the still-mounted reading menu underneath.
        void requestGestureState(readerAction.menuOpen() || !!layer, !!layer);
        return;
      }
      const hasLocalPage = Array.from(document.querySelectorAll<HTMLButtonElement>("button[data-back-local]"))
        .some(visible);
      const sourceEditorEntry = route.name === routeNames.sources && Boolean(route.query.sourceEdit);
      // A WebKit swipe previews its real history item. Local dialogs and pages
      // without a matching entry must not expose an unrelated screen beneath.
      const enabled = typeof router.options.history.state.back === "string"
        && (Boolean(route.query[readerQuery]) || !isTabRoute(route.name) && route.name != null)
        && !hasVisibleBackLayer() && (!hasLocalPage || sourceEditorEntry);
      void requestGestureState(enabled);
    };
    prepareTabGesture = async () => {
      // Tab roots keep the native swipe disabled because their history may
      // point at another tab. Enable it before pushing a child route so
      // WKWebView captures the tab page as that entry's swipe-back preview.
      tabPushPending = true;
      lastRequested = undefined;
      await requestGestureState(true);
    };
    let queued = false;
    const queueGestureSync = () => {
      if (queued) return;
      queued = true;
      queueMicrotask(() => { queued = false; if (!disposed) syncGestures(); });
    };
    const controls = "[data-back-layer],.modal-backdrop,.overlay-backdrop,[aria-modal='true'],button[data-back-local],[data-reader-menu-open]";
    const affectsControls = (node: Node) => node instanceof Element
      && (node.matches(controls) || !!node.querySelector(controls));
    const observer = new MutationObserver(records => {
      if (records.some(record => record.type === "attributes" ? affectsControls(record.target)
        : [...record.addedNodes, ...record.removedNodes].some(affectsControls))) queueGestureSync();
    });
    observer.observe(document.body, {
      childList: true, subtree: true, attributes: true, attributeFilter: ["class", "style", "hidden", "inert", "data-reader-menu-open"],
    });
    queueGestureSync();
    const removeAfterEach = router.afterEach((_to, _from, failure) => {
      tabPushPending = false;
      if (failure) lastRequested = undefined;
      queueGestureSync();
    });
    const localGesture = createLocalBackGesture(() => {
      const layer = topBackLayer();
      if (!readerAction?.isOpen() || !layer) return null;
      const action = localBackAction();
      if (!action) return null;
      return { element: layer.closest<HTMLElement>("[data-back-layer],.modal-backdrop,.overlay-backdrop") ?? layer,
        back: action.run };
    });
    removeGestureSync = () => { observer.disconnect(); removeAfterEach(); localGesture.cancel(); prepareTabGesture = undefined; };
    void onBackGesture(async event => {
      if (disposed || !readerAction?.isOpen()) return;
      if (event.phase === "start") {
        if (handling) return;
        handling = true;
      } else if (!handling) return;
      try { await localGesture.handle(event); }
      catch (error) { localGesture.cancel(); handling = false; console.error("Reader back gesture failed:", error); }
      finally { if (event.phase === "end") handling = false; }
    }).then(unlisten => {
      if (disposed) void unlisten();
      else removeGestureListener = unlisten;
    }).catch(error => console.error("Reader back gesture registration failed:", error));
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
    if (!popping && isTabRoute(from.name) && !isTabRoute(to.name)) {
      await prepareTabGesture?.();
    }
    if (!popping) return;
    // The mobile source editor has its own history entry. Let its route guard
    // handle the draft and reveal the source list captured by WKWebView.
    if (from.name === routeNames.sources && from.query.sourceEdit
      && from.path === to.path && from.query.sourceEdit !== to.query.sourceEdit
      && !hasVisibleBackLayer() && !readerAction?.isOpen()) return;
    const action = localBackAction();
    if (!action) return;
    await action.run();
    const closesReaderEntry = from.query[readerQuery] && !to.query[readerQuery]
      && from.path === to.path && !readerAction?.isOpen();
    return closesReaderEntry ? undefined : false;
  });

  if (isTauri()) {
    void onBackRequested(async ({ canGoBack, source }) => {
      if (disposed || handling) return;
      handling = true;
      try {
        const action = localBackAction();
        const route = router.currentRoute.value;
        if (route.name === routeNames.sources && route.query.sourceEdit && !hasVisibleBackLayer() && !readerAction?.isOpen()) {
          const query = { ...route.query };
          delete query.sourceEdit;
          delete query.sourceImport;
          delete query.importUrl;
          const list = { name: routeNames.sources, query };
          if (router.options.history.state.back === router.resolve(list).fullPath) router.back();
          else void router.replace(list);
        } else if (action) await action.run();
        else if (canGoBack !== false && typeof router.options.history.state.back === "string") router.back();
        else {
          const pageBack = Array.from(document.querySelectorAll<HTMLButtonElement>("button[data-back-action]"))
            .reverse().find(visible);
          if (pageBack) buttonAction(pageBack).run();
          else if (source === "system") await exit(0);
        }
      } finally { handling = false; }
    }).then(unlisten => {
      if (disposed) void unlisten();
      else removeBackListener = unlisten;
    }).catch(error => console.error("Native back registration failed:", error));
  }
  return () => { disposed = true; removeGuard(); removeGestureSync?.(); void removeBackListener?.(); void removeGestureListener?.(); };
}
