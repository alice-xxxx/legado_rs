import { createApp, type Component } from "vue";
import { emit } from "@tauri-apps/api/event";
import { appBootstrap } from "./api/core";
import router from "./router";
import { notify } from "./app/notifications";
import RoutesRoot from "./app/RoutesRoot.vue";
import ExternalImportHost from "./features/import/ExternalImportHost.vue";
import ReaderHost from "./features/reader/ReaderHost.vue";
import GlobalNoticeHost from "./app/GlobalNoticeHost.vue";
import RecoveryRuntime from "./app/RecoveryRuntime.vue";
import { loadSettings } from "./features/settings/settingsStore";
import { initializeTheme } from "./features/settings/themeState";
import { loadHistory } from "./features/settings/historyStore";
import { loadConfigurations } from "./features/settings/configurationStores";
import { initializeBookStore } from "./features/books/bookDetailState";
import { loadShelf } from "./features/shelf/shelfStore";
import { loadSources } from "./features/sources/sourcesStore";
import { initializeTaskStore, disposeTaskStore } from "./features/tasks/tasksStore";
import "./style.css";

const mountedApps: Array<{ unmount: () => void }> = [];
let startupDisposed = false;
const disposeTheme = initializeTheme();
const preventIosZoomGesture = (event: Event) => event.preventDefault();
window.addEventListener("gesturestart", preventIosZoomGesture, { passive: false });
window.addEventListener("gesturechange", preventIosZoomGesture, { passive: false });

function limit(value: string, maxLength: number): string {
  return value.length > maxLength ? `${value.slice(0, maxLength)}…` : value;
}

function asText(value: unknown): string {
  try {
    return value instanceof Error ? value.message : String(value);
  } catch {
    return "Unprintable frontend error";
  }
}

function mountRoot(component: Component, target: string, name: string, useRouter = false): void {
  const app = createApp(component);
  if (useRouter) app.use(router);

  app.config.errorHandler = (error, _instance, info) => {
    console.error(`[Vue runtime error: ${name}]`, error, info);
    if (import.meta.env.DEV) {
      void emit("frontend-runtime-error", {
        source: limit(`${name}:vue`, 64),
        message: limit(asText(error), 4096),
        stack: limit(error instanceof Error ? error.stack ?? "" : "", 8192),
        info: limit(info, 1024),
      }).catch(() => {});
    }
    notify(`界面出现错误：${asText(error)}`, "error");
  };

  app.mount(target);
  mountedApps.push(app);
}

const onUnhandledRejection = (event: PromiseRejectionEvent) => {
  console.error("[Unhandled Promise Rejection]", event.reason);
  if (!import.meta.env.DEV) return;
  void emit("frontend-runtime-error", {
    source: "unhandledrejection",
    message: limit(asText(event.reason), 4096),
    stack: limit(event.reason instanceof Error ? event.reason.stack ?? "" : "", 8192),
    info: "",
  }).catch(() => {});
};
window.addEventListener("unhandledrejection", onUnhandledRejection);
const removeRouterError = router.onError(error => {
  console.error("Route loading failed", error);
  notify(`页面载入失败：${asText(error)}`, "error");
});

// Recovery listening starts immediately; settings initialize before session hosts and pages.
mountRoot(RecoveryRuntime, "#recovery-runtime", "recovery");

void appBootstrap().then((bootstrap) => Promise.all([
  loadSettings(bootstrap.settings),
  loadHistory(bootstrap.history),
  loadShelf(bootstrap.shelf),
  loadSources(bootstrap.sources),
  loadConfigurations(bootstrap),
  initializeBookStore(bootstrap.shelf),
  initializeTaskStore(),
])).then(() => {
  if (startupDisposed) return;
  // The import host installs its session before ReaderHost consumes it.
  mountRoot(ExternalImportHost, "#external-import-host", "external-import");
  // ReaderHost installs the reader session used by notices and route pages.
  mountRoot(ReaderHost, "#reader-host", "reader", true);
  mountRoot(GlobalNoticeHost, "#notice-host", "notices");
  mountRoot(RoutesRoot, "#app", "routes", true);
}).catch((error: unknown) => {
  console.error("App initialization failed", error);
  if (startupDisposed) return;
  // Show a failure only after an actual error; do not hide the UI behind a timer or reload it automatically.
  const target = document.getElementById("app");
  if (!target) return;
  const panel = document.createElement("div");
  panel.className = "safe-area-top";
  panel.style.padding = "24px";
  panel.setAttribute("role", "alert");
  const message = document.createElement("p");
  message.textContent = `启动失败：${asText(error)}`;
  const retry = document.createElement("button");
  retry.type = "button";
  retry.textContent = "重试";
  retry.addEventListener("click", () => window.location.reload());
  panel.append(message, retry);
  target.replaceChildren(panel);
});

if (import.meta.hot) {
  import.meta.hot.dispose(() => {
    startupDisposed = true;
    disposeTheme();
    disposeTaskStore();
    window.removeEventListener("gesturestart", preventIosZoomGesture);
    window.removeEventListener("gesturechange", preventIosZoomGesture);
    window.removeEventListener("unhandledrejection", onUnhandledRejection);
    removeRouterError();
    for (const app of mountedApps.splice(0).reverse()) app.unmount();
  });
}
