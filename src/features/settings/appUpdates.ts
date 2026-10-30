import { computed, ref, shallowRef } from "vue";
import { Channel, isTauri } from "@tauri-apps/api/core";
import { command } from "../../api/ipc";
import { errorText, notify } from "../../app/notifications";
import { backupBusy } from "./backupState";
import { shelfBatchRecoveryRequired } from "../../app/recoveryState";
import { settings } from "./settingsStore";

export interface AppUpdate { version: string; notes: string; page: string; kind: "desktop" | "android" | "ios"; }
interface AppInfo { version: string; system: string; arch: string; }
interface Progress { phase: "downloading" | "installing"; downloaded: number; total: number | null; }

export const currentVersion = ref("");
export const currentPlatform = ref("");
export const appUpdate = shallowRef<AppUpdate | null>(null);
export const updateDialogOpen = ref(false);
export const updatePhase = ref<"idle" | "checking" | "downloading" | "installing" | "restart" | "permission" | "installer">("idle");
export const updateProgress = ref<Progress | null>(null);
export const updateError = ref("");
export const updateBusy = computed(() => ["checking", "downloading", "installing"].includes(updatePhase.value));
export const updateBlocked = computed(() => updateBusy.value || backupBusy.value !== null || shelfBatchRecoveryRequired.value);
export const updateAction = computed(() => updatePhase.value === "restart" ? "重启完成更新"
  : updatePhase.value === "permission" ? "继续安装"
    : appUpdate.value?.kind === "ios" ? "前往下载" : "下载并更新");

let automaticActive = false;

export async function checkAppUpdate(manual = true): Promise<void> {
  if (updateBlocked.value || updatePhase.value === "restart") return;
  updatePhase.value = "checking";
  updateError.value = "";
  try {
    appUpdate.value = await command<AppUpdate | null>("check_app_update");
    if (appUpdate.value) {
      if (manual || (automaticActive && settings.value.autoCheckUpdates !== false
        && settings.value.ignoredUpdateVersion !== appUpdate.value.version)) updateDialogOpen.value = true;
    } else if (manual) notify("当前已是最新发布版本。");
  } catch (error) {
    updateError.value = `检查更新失败：${errorText(error)}`;
    if (manual) notify(updateError.value, "error");
  } finally { updatePhase.value = "idle"; }
}

export function ignoreAppUpdate(): void {
  if (!appUpdate.value || updatePhase.value !== "idle") return;
  settings.value.ignoredUpdateVersion = appUpdate.value.version;
  updateDialogOpen.value = false;
}

export async function installAppUpdate(): Promise<void> {
  if (!appUpdate.value || updateBlocked.value) return;
  updateError.value = "";
  if (updatePhase.value === "restart") {
    try { await command("restart_after_update"); }
    catch (error) { updateError.value = `重启失败：${errorText(error)}`; }
    return;
  }
  updatePhase.value = "downloading";
  updateProgress.value = null;
  const progress = new Channel<Progress>();
  progress.onmessage = value => {
    if (updatePhase.value !== "downloading" && updatePhase.value !== "installing") return;
    updateProgress.value = value;
    updatePhase.value = value.phase;
  };
  try {
    const result = await command<"restart" | "permission" | "installer" | "browser">("install_app_update", { progress });
    updatePhase.value = result === "browser" ? "idle" : result;
    updateDialogOpen.value = result === "restart" || result === "permission";
  } catch (error) {
    updateError.value = `更新失败：${errorText(error)}`;
    updatePhase.value = "idle";
    if (!updateDialogOpen.value) notify(updateError.value, "error");
  }
}

export function initializeAppUpdates(): () => void {
  if (!isTauri()) return () => {};
  automaticActive = true;
  void command<AppInfo>("get_app_info").then(info => {
    currentVersion.value = info.version;
    currentPlatform.value = `${info.system} / ${info.arch}`;
  }).catch(console.error);
  if (import.meta.env.PROD && settings.value.autoCheckUpdates !== false) void checkAppUpdate(false);
  return () => { automaticActive = false; };
}
