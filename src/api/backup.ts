import { command } from "./ipc";
import type { BackupResponse, WebDavBackupUploadResponse, RestoreBackupResponse, WebDavBackupRestoreResponse } from "./types";

export const createBackupFromPicker = () => command<BackupResponse>("create_backup_from_picker");

export const restoreBackupFromPicker = () => command<RestoreBackupResponse>("restore_backup_from_picker");

export const uploadWebDavBackup = () => command<WebDavBackupUploadResponse>("upload_webdav_backup");

export const restoreWebDavBackup = () => command<WebDavBackupRestoreResponse>("restore_webdav_backup");
