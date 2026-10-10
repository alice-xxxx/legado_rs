import { ref } from "vue";

export type BackupBusy = "create" | "restore" | "webdav-save" | "webdav-upload" | null;

/** Shared backup lock used by backup actions and screens that must pause mutations. */
export const backupBusy = ref<BackupBusy>(null);

let settingsSaveDeferredByRestore = false;

/** Reader settings changes during restore are held until the restore outcome is known. */
export function deferSettingsSaveDuringRestore(): boolean {
  if (backupBusy.value !== "restore") return false;
  settingsSaveDeferredByRestore = true;
  return true;
}

export function beginBackupRestore(): void {
  settingsSaveDeferredByRestore = false;
  backupBusy.value = "restore";
}

/** Returns and clears the deferred-save marker after a restore attempt. */
export function finishBackupRestore(): boolean {
  const deferred = settingsSaveDeferredByRestore;
  settingsSaveDeferredByRestore = false;
  return deferred;
}
