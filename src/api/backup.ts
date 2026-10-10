import { command } from "./ipc";
import type { BackupResponse, WebDavBackupConfig, WebDavBackupUploadResponse, RestoreBackupResponse, WebDavBackupRestoreResponse } from "./types";

export const createBackupFromPicker = () => command<BackupResponse>("create_backup_from_picker");

export const restoreBackupFromPicker = () => command<RestoreBackupResponse>("restore_backup_from_picker");

export const getWebDavBackupConfig = () => command<WebDavBackupConfig>("get_webdav_backup_config");

export const saveWebDavBackupConfig = (url: string, username: string, password: string, clearPassword = false) =>
  command<WebDavBackupConfig>("save_webdav_backup_config", { url, username, password, clearPassword });

export const saveWebDavAutoBackupInterval = (intervalHours: number | null) =>
  command<WebDavBackupConfig>("save_webdav_auto_backup_interval", { intervalHours });

export const uploadWebDavBackup = () => command<WebDavBackupUploadResponse>("upload_webdav_backup");

export const restoreWebDavBackup = () => command<WebDavBackupRestoreResponse>("restore_webdav_backup");
