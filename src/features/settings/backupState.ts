import { ref } from "vue";

export type BackupBusy = "create" | "restore" | "webdav-upload" | null;

/** Shared backup lock used by backup actions and screens that must pause mutations. */
export const backupBusy = ref<BackupBusy>(null);

export function beginBackupRestore(): void {
  backupBusy.value = "restore";
}

export function finishBackupRestore(): void {
  backupBusy.value = null;
}
