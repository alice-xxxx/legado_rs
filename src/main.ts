import { createApp, type Component } from "vue";
import { emit } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import { appBootstrap } from "./api/core";
import router from "./router";
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
let startupComplete = false;
const startupRetryKey = "legado:startup-retry";
const resumeRetryKey = "legado:resume-retry";
let displayCheckTimer: number | undefined;
const disposeTheme = initializeTheme();
const startupCover = document.createElement("div");
startupCover.id = "app-startup-cover";
startupCover.style.cssText = "position:fixed;inset:0;z-index:110;background:#fff;padding:calc(env(safe-area-inset-top) + 24px) 24px 24px;box-sizing:border-box;font:16px -apple-system,BlinkMacSystemFont,sans-serif;color:#444";
startupCover.textContent = "正在载入应用界面…";
document.body.append(startupCover);
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

function renderFailure(target: HTMLElement, titleText: string, message: string): void {
  const panel = document.createElement("div");
  panel.style.cssText = "padding:calc(env(safe-area-inset-top) + 24px) 24px 24px;font:16px -apple-system,BlinkMacSystemFont,sans-serif;color:#333";
  const title = document.createElement("strong");
  title.textContent = titleText;
  const detail = document.createElement("p");
  detail.style.cssText = "line-height:1.5;overflow-wrap:anywhere";
  detail.textContent = limit(message, 300);
  const retry = document.createElement("button");
  retry.type = "button";
  retry.textContent = "重新载入";
  retry.style.cssText = "min-height:44px;padding:0 18px;border:1px solid #aaa;border-radius:8px;background:#fff;color:#222;font:inherit";
  retry.addEventListener("click", () => window.location.reload());
  panel.append(title, detail, retry);
  target.replaceChildren(panel);
}

function showStartupFailure(message: string): void {
  if (startupDisposed || startupComplete) return;
  startupCover.style.padding = "0";
  renderFailure(startupCover, "应用界面载入失败", message);
}

function showAppFailure(message: string): void {
  if (startupDisposed) return;
  let target = document.getElementById("app-failure");
  if (!target) {
    target = document.createElement("div");
    target.id = "app-failure";
    target.setAttribute("role", "alert");
    target.style.cssText = "position:fixed;inset:0;z-index:120;overflow:auto;background:#fff";
    document.body.append(target);
  }
  renderFailure(target, "应用界面出现错误", message);
}

function checkRenderedApp(): void {
  if (startupDisposed || !startupComplete || document.visibilityState !== "visible" || document.getElementById("app-failure")) return;
  if (document.querySelector("#app > *") || document.querySelector("#reader-host > *")) {
    try { sessionStorage.removeItem(resumeRetryKey); } catch { /* Optional retry marker. */ }
    return;
  }
  console.error("App view is empty after becoming visible");
  try {
    if (sessionStorage.getItem(resumeRetryKey) !== "1") {
      sessionStorage.setItem(resumeRetryKey, "1");
      window.location.reload();
      return;
    }
  } catch { /* Storage may be unavailable; keep the visible retry action. */ }
  showAppFailure("页面未能恢复显示。请重新载入应用。");
}

function scheduleDisplayCheck(): void {
  window.clearTimeout(displayCheckTimer);
  if (document.visibilityState === "visible") displayCheckTimer = window.setTimeout(checkRenderedApp, 2_000);
}

window.addEventListener("pageshow", scheduleDisplayCheck);
document.addEventListener("visibilitychange", scheduleDisplayCheck);

function sendFrontendHeartbeat(): void {
  if (!startupDisposed && document.visibilityState === "visible") {
    void invoke("frontend_heartbeat").catch(() => {});
  }
}
window.addEventListener("pageshow", sendFrontendHeartbeat);
document.addEventListener("visibilitychange", sendFrontendHeartbeat);
const heartbeatTimer = window.setInterval(sendFrontendHeartbeat, 5_000);
sendFrontendHeartbeat();

function handleStartupFailure(error: unknown): void {
  console.error("App initialization failed", error);
  if (startupDisposed || startupComplete) return;
  try {
    if (sessionStorage.getItem(startupRetryKey) !== "1") {
      sessionStorage.setItem(startupRetryKey, "1");
      window.setTimeout(() => window.location.reload(), 500);
      return;
    }
  } catch { /* Storage may be unavailable; keep the visible retry action. */ }
  showStartupFailure(asText(error));
}

function mountRoot(component: Component, target: string, name: string, useRouter = false): void {
  const app = createApp(component);
  if (useRouter) app.use(router);

  app.config.errorHandler = (error, _instance, info) => {
    console.error(`[Vue runtime error: ${name}]`, error, info);
    if (import.meta.env.DEV) {
      const payload = {
        source: limit(`${name}:vue`, 64),
        message: limit(asText(error), 4096),
        stack: limit(error instanceof Error ? error.stack ?? "" : "", 8192),
        info: limit(info, 1024),
      };
      void emit("frontend-runtime-error", payload).catch(() => {});
    }
    showAppFailure(`${asText(error)}${info ? ` (${info})` : ""}`);
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
  showAppFailure(asText(error));
});

// Recovery listening starts immediately; settings initialize before session hosts and pages.
mountRoot(RecoveryRuntime, "#recovery-runtime", "recovery");

const startupWatchdog = window.setTimeout(() => showStartupFailure("载入时间过长。请重新载入应用。"), 20_000);
void appBootstrap().then((bootstrap) => Promise.all([
  loadSettings(bootstrap.settings),
  loadHistory(bootstrap.history),
  loadShelf(bootstrap.shelf),
  loadSources(bootstrap.sources),
  loadConfigurations(bootstrap),
  initializeBookStore(bootstrap.shelf),
  initializeTaskStore(),
])).then(async () => {
  if (startupDisposed) return;
  // The import host installs its session before ReaderHost consumes it.
  mountRoot(ExternalImportHost, "#external-import-host", "external-import");
  // ReaderHost installs the reader session used by notices and route pages.
  mountRoot(ReaderHost, "#reader-host", "reader", true);
  mountRoot(GlobalNoticeHost, "#notice-host", "notices");
  mountRoot(RoutesRoot, "#app", "routes", true);
  await router.isReady();
  if (startupDisposed) return;
  startupComplete = true;
  window.clearTimeout(startupWatchdog);
  startupCover.remove();
  try { sessionStorage.removeItem(startupRetryKey); } catch { /* Optional retry marker. */ }
  scheduleDisplayCheck();
}).catch((error: unknown) => {
  window.clearTimeout(startupWatchdog);
  handleStartupFailure(error);
});

if (import.meta.hot) {
  import.meta.hot.dispose(() => {
    startupDisposed = true;
    window.clearTimeout(startupWatchdog);
    window.clearTimeout(displayCheckTimer);
    window.clearInterval(heartbeatTimer);
    startupCover.remove();
    disposeTheme();
    disposeTaskStore();
    window.removeEventListener("gesturestart", preventIosZoomGesture);
    window.removeEventListener("gesturechange", preventIosZoomGesture);
    window.removeEventListener("unhandledrejection", onUnhandledRejection);
    window.removeEventListener("pageshow", scheduleDisplayCheck);
    document.removeEventListener("visibilitychange", scheduleDisplayCheck);
    window.removeEventListener("pageshow", sendFrontendHeartbeat);
    document.removeEventListener("visibilitychange", sendFrontendHeartbeat);
    removeRouterError();
    document.getElementById("app-failure")?.remove();
    for (const app of mountedApps.splice(0).reverse()) app.unmount();
  });
}
