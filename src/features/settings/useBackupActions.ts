import { ref } from "vue";
import { createBackupFromPicker, getWebDavBackupConfig, restoreBackupFromPicker, restoreWebDavBackup, saveWebDavBackupConfig, uploadWebDavBackup } from "../../api/backup";
import { readResource } from "../../api/resources";
import type { AppSettingsResource, BackupResponse, ResourceDescriptor, RestoreBackupResponse, WebDavBackupConfig, WebDavBackupRestoreResponse, WebDavBackupUploadResponse } from "../../api/types";
import { getReaderBackupSession } from "../reader/readerBackupSession";
import { backupBusy, beginBackupRestore, finishBackupRestore } from "./backupState";
import { lockShelfBatchRecovery, shelfBatchRecoveryRequired } from "../../app/recoveryState";
import { notify } from "../../app/notifications";
import { normalizeSettings } from "./settingsDefaults";

export type { BackupBusy } from "./backupState";
type RestoreOutcome = "notCommitted" | "committed" | "recovery";
type RestoreResult = Exclude<RestoreBackupResponse, { cancelled: true }> | WebDavBackupRestoreResponse;

function errorText(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

/** Backup screen owns backup actions and updates its settings snapshot directly. */
export function useBackupActions() {
  const session = getReaderBackupSession();
  const webDavBackupConfig = ref<WebDavBackupConfig | null>(null);
  const webDavConfigLoading = ref(false);
  const webDavBackupError = ref("");
  let settingsSnapshotRevision = 0;

  function cancelPendingSettingsSave(): boolean {
    const hadPendingSettingsSave = session.pendingSettingsSave.value !== null;
    if (session.pendingSettingsSave.value) clearTimeout(session.pendingSettingsSave.value);
    session.pendingSettingsSave.value = null;
    return hadPendingSettingsSave;
  }

  async function applyBackupSettingsSnapshot(descriptor: ResourceDescriptor): Promise<void> {
    const revision = ++settingsSnapshotRevision;
    const settings = normalizeSettings(await readResource<AppSettingsResource>(descriptor));
    if (revision === settingsSnapshotRevision) session.settings.value = settings;
  }

  async function prepareBackupContents(): Promise<void> {
    cancelPendingSettingsSave();
    if (session.saveSettingsBusy.value) throw new Error("阅读设置仍在保存，请稍后再创建备份。");
    if (session.hasActiveReaderMedia()) {
      if (!await session.pauseReaderMediaAndSave()) throw new Error("媒体播放位置尚未保存，备份已取消；请稍后重试。");
    } else if (session.readingBook.value && session.readerProgressDirty.value) {
      await session.saveCurrentProgress();
      if (session.readerProgressDirty.value) throw new Error("阅读位置尚未保存，备份已取消；请稍后重试。");
    }
    if (!await session.persistSettings()) throw new Error("阅读设置未能保存，备份已取消。");
  }

  async function lockRestoreRecovery(title: string, message: string): Promise<void> {
    lockShelfBatchRecovery(title, message);
    try {
      await session.closeReaderAfterRestore();
    } catch {
      // 恢复锁优先；即使阅读器清理失败，也不能继续使用旧阅读状态。
    }
  }

  async function consumeRestoreOutcome(result: RestoreResult, source: string): Promise<RestoreOutcome> {
    const warning = result.warning?.trim() ?? "";
    if (result.restored === false && result.commitState === "notCommitted" && !result.recoveryRequired) {
      notify(`${source}未恢复，当前数据和阅读器保持不变。${warning || "可以重试。"}`, "error");
      return "notCommitted";
    }
    if (result.restored !== true || result.commitState !== "committed" || result.recoveryRequired !== false) {
      await lockRestoreRecovery(
        "备份恢复需要重启完成",
        `${source}恢复未能安全完成或提交状态未知。旧阅读器已关闭，本会话已暂停书架修改；请重启应用。${warning ? ` ${warning}` : ""}`,
      );
      notify(`${source}恢复需要重启应用完成；旧阅读器已关闭。`, "error");
      return "recovery";
    }
    if (!result.bootstrap) {
      await lockRestoreRecovery(
        "备份恢复状态未能载入",
        `${source}恢复已提交，但没有收到应用状态快照。旧阅读器已关闭，本会话已暂停书架修改；请重启应用检查。`,
      );
      notify(`${source}恢复已提交，但应用状态快照缺失；请重启应用检查。`, "error");
      return "recovery";
    }
    try {
      await applyBackupSettingsSnapshot(result.bootstrap.settings);
    } catch (error) {
      // The backup commit is already durable, but the frontend may now hold a
      // mix of old and restored descriptors. Do not allow new writes until a
      // fresh process has reconstructed the complete resource state.
      await lockRestoreRecovery(
        "备份恢复后的状态载入失败",
        `${source}数据已提交，但前端无法载入恢复后的设置：${errorText(error)}。已停止本会话的书架修改；请重启应用检查。`,
      );
      notify(`${source}数据已提交，但恢复后的设置载入失败；请重启应用完成恢复。`, "error");
      return "recovery";
    }
    if (warning) notify(`${source}已恢复，但${warning}`, "error");
    else notify(`${source}已恢复，应用数据正在更新。`);
    return "committed";
  }

  async function createBackup(): Promise<void> {
    if (backupBusy.value || session.saveSettingsBusy.value || shelfBatchRecoveryRequired.value) return;
    backupBusy.value = "create";
    try {
      await prepareBackupContents();
      const result: BackupResponse = await createBackupFromPicker();
      if (result.cancelled) return;
      if (!result.backup) throw new Error("备份没有完成。");
      if (result.settings) {
        try {
          await applyBackupSettingsSnapshot(result.settings);
        } catch (error) {
          notify(`备份文件已保存，但最近备份时间无法读取：${errorText(error)}`, "error");
          return;
        }
      }
      if (result.warning) {
        notify(result.warning, "error");
        return;
      }
      notify("备份文件已保存。请妥善保管其中的书源与登录状态。");
    } catch (error) {
      notify(`创建备份失败：${errorText(error)}`, "error");
    } finally {
      backupBusy.value = null;
    }
  }

  async function restoreBackup(): Promise<void> {
    if (backupBusy.value || session.saveSettingsBusy.value || shelfBatchRecoveryRequired.value) return;
    if (!window.confirm("恢复备份会替换当前书架、阅读数据、书源和登录状态。要继续选择备份文件吗？")) return;
    const hadPendingSettingsSave = cancelPendingSettingsSave();
    beginBackupRestore();
    let restoreDidNotCommit = false;
    try {
      const result: RestoreBackupResponse = await restoreBackupFromPicker();
      if ("cancelled" in result) {
        restoreDidNotCommit = true;
        return;
      }
      const outcome = await consumeRestoreOutcome(result, "本地备份");
      if (outcome === "notCommitted") restoreDidNotCommit = true;
    } catch (error) {
      await lockRestoreRecovery(
        "备份恢复状态未确认",
        `没有收到本地备份的结构化恢复结果，无法确认数据是否已替换。旧阅读器已关闭，本会话已暂停书架修改；请重启应用检查。应用返回信息：${errorText(error)}`,
      );
      notify("备份恢复状态未知，旧阅读器已关闭；请重启应用检查。", "error");
    } finally {
      backupBusy.value = null;
      const settingsSaveDeferredByRestore = finishBackupRestore();
      if (restoreDidNotCommit && (hadPendingSettingsSave || settingsSaveDeferredByRestore)) session.scheduleSettingsSave();
    }
  }

  async function reloadWebDavBackupConfig(): Promise<void> {
    if (backupBusy.value || shelfBatchRecoveryRequired.value || webDavConfigLoading.value) return;
    webDavConfigLoading.value = true;
    webDavBackupError.value = "";
    try {
      webDavBackupConfig.value = await getWebDavBackupConfig();
    } catch (error) {
      webDavBackupError.value = `读取 WebDAV 配置失败：${errorText(error)}`;
    } finally {
      webDavConfigLoading.value = false;
    }
  }

  async function saveWebDavConfig(input: { url: string; username: string; password: string }): Promise<void> {
    if (backupBusy.value || shelfBatchRecoveryRequired.value || webDavConfigLoading.value) return;
    backupBusy.value = "webdav-save";
    webDavBackupError.value = "";
    try {
      webDavBackupConfig.value = await saveWebDavBackupConfig(input.url, input.username, input.password, false);
      notify("WebDAV 连接设置已保存。空密码会保留同一目录和用户名下已保存的密码。");
    } catch (error) {
      webDavBackupError.value = `保存 WebDAV 配置失败：${errorText(error)}`;
      notify(webDavBackupError.value, "error");
    } finally {
      backupBusy.value = null;
    }
  }

  async function clearWebDavPassword(): Promise<void> {
    const config = webDavBackupConfig.value;
    if (backupBusy.value || shelfBatchRecoveryRequired.value || webDavConfigLoading.value || !config?.hasPassword) return;
    if (!window.confirm("清除已保存的 WebDAV 密码？之后上传或恢复可能需要重新配置密码。")) return;
    backupBusy.value = "webdav-save";
    webDavBackupError.value = "";
    try {
      webDavBackupConfig.value = await saveWebDavBackupConfig(config.url, config.username, "", true);
      notify("WebDAV 密码已清除。");
    } catch (error) {
      webDavBackupError.value = `清除 WebDAV 密码失败：${errorText(error)}`;
      notify(webDavBackupError.value, "error");
    } finally {
      backupBusy.value = null;
    }
  }

  async function uploadWebDavBackupManually(): Promise<void> {
    if (backupBusy.value || session.saveSettingsBusy.value || shelfBatchRecoveryRequired.value
      || webDavConfigLoading.value || !webDavBackupConfig.value?.url) return;
    backupBusy.value = "webdav-upload";
    webDavBackupError.value = "";
    try {
      await prepareBackupContents();
      const result: WebDavBackupUploadResponse = await uploadWebDavBackup();
      if (!result.uploaded) throw new Error("备份没有上传完成。");
      let metadataReadError = "";
      if (result.settings) {
        try {
          await applyBackupSettingsSnapshot(result.settings);
        } catch (error) {
          metadataReadError = `最近备份时间无法读取：${errorText(error)}`;
        }
      }
      const warning = [result.warning?.trim(), metadataReadError].filter(Boolean).join(" ");
      if (warning) {
        webDavBackupError.value = `备份已上传，但${warning}`;
        notify(webDavBackupError.value, "error");
      } else {
        notify("WebDAV 备份已上传至 legado-rs-backup.zip。");
      }
    } catch (error) {
      webDavBackupError.value = `WebDAV 上传失败：${errorText(error)}`;
      notify(webDavBackupError.value, "error");
    } finally {
      backupBusy.value = null;
    }
  }

  async function restoreWebDavBackupManually(): Promise<void> {
    const config = webDavBackupConfig.value;
    if (backupBusy.value || session.saveSettingsBusy.value || shelfBatchRecoveryRequired.value
      || webDavConfigLoading.value || !config?.url) return;
    if (!window.confirm("将使用 WebDAV 目录中的 legado-rs-backup.zip 替换当前本地书架、阅读数据、书源和设置。恢复成功后会重新载入应用数据。要继续吗？")) return;
    const hadPendingSettingsSave = cancelPendingSettingsSave();
    beginBackupRestore();
    webDavBackupError.value = "";
    let restoreDidNotCommit = false;
    try {
      const result: WebDavBackupRestoreResponse = await restoreWebDavBackup();
      const outcome = await consumeRestoreOutcome(result, "WebDAV 备份");
      if (outcome === "notCommitted") {
        restoreDidNotCommit = true;
        webDavBackupError.value = `WebDAV 备份未恢复：${result.warning || "当前数据和阅读器保持不变，可以重试。"}`;
      } else {
        if (outcome === "committed" && result.warning?.trim()) {
          webDavBackupError.value = `数据已恢复，但${result.warning.trim()}`;
        } else if (outcome === "recovery") {
          webDavBackupError.value = `WebDAV 恢复需要重启应用检查：${result.warning || "恢复状态未知。"}`;
        }
      }
    } catch (error) {
      const message = errorText(error);
      await lockRestoreRecovery(
        "WebDAV 恢复状态未确认",
        `没有收到 WebDAV 备份的结构化恢复结果，无法确认数据是否已替换。旧阅读器已关闭，本会话已暂停书架修改；请重启应用检查。应用返回信息：${message}`,
      );
      webDavBackupError.value = `WebDAV 恢复状态未知：${message}。旧阅读器已关闭，请重启应用检查。`;
      notify(webDavBackupError.value, "error");
    } finally {
      backupBusy.value = null;
      const settingsSaveDeferredByRestore = finishBackupRestore();
      if (restoreDidNotCommit && (hadPendingSettingsSave || settingsSaveDeferredByRestore)) session.scheduleSettingsSave();
    }
  }

  return {
    backupBusy,
    applyBackupSettingsSnapshot,
    webDavBackupConfig,
    webDavConfigLoading,
    webDavBackupError,
    prepareBackupContents,
    createBackup,
    restoreBackup,
    reloadWebDavBackupConfig,
    saveWebDavConfig,
    clearWebDavPassword,
    uploadWebDavBackupManually,
    restoreWebDavBackupManually,
  };
}
