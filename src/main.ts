import { createApp, type Component } from "vue";
import { emit } from "@tauri-apps/api/event";
import router from "./router";
import RoutesRoot from "./app/RoutesRoot.vue";
import ExternalImportHost from "./features/import/ExternalImportHost.vue";
import ReaderHost from "./features/reader/ReaderHost.vue";
import GlobalNoticeHost from "./app/GlobalNoticeHost.vue";
import RecoveryRuntime from "./app/RecoveryRuntime.vue";
import "./style.css";

const mountedApps: Array<{ unmount: () => void }> = [];
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

  if (import.meta.env.DEV) {
    const reportRuntimeError = (source: string, error: unknown, info = "") => {
      const payload = {
        source: limit(`${name}:${source}`, 64),
        message: limit(asText(error), 4096),
        stack: limit(error instanceof Error ? error.stack ?? "" : "", 8192),
        info: limit(info, 1024),
      };
      void emit("frontend-runtime-error", payload).catch(() => {});
    };
    app.config.errorHandler = (error, _instance, info) => {
      console.error(`[Vue runtime error: ${name}]`, error, info);
      reportRuntimeError("vue", error, info);
    };
  }

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

// Feature hosts install their narrow session contracts before route setup.
mountRoot(RecoveryRuntime, "#recovery-runtime", "recovery");
mountRoot(ExternalImportHost, "#external-import-host", "external-import");
mountRoot(ReaderHost, "#reader-host", "reader", true);
mountRoot(GlobalNoticeHost, "#notice-host", "notices");
mountRoot(RoutesRoot, "#app", "routes", true);

if (import.meta.hot) {
  import.meta.hot.dispose(() => {
    window.removeEventListener("gesturestart", preventIosZoomGesture);
    window.removeEventListener("gesturechange", preventIosZoomGesture);
    window.removeEventListener("unhandledrejection", onUnhandledRejection);
    for (const app of mountedApps.splice(0).reverse()) app.unmount();
  });
}
