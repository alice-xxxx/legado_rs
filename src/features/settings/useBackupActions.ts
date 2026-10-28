import { computed, ref } from "vue";
import { createBackupFromPicker, restoreBackupFromPicker, restoreWebDavBackup, uploadWebDavBackup } from "../../api/backup";
import type { BackupResponse, RestoreBackupResponse, WebDavBackupRestoreResponse, WebDavBackupUploadResponse } from "../../api/types";
import { getReaderBackupSession } from "../reader/readerBackupSession";
import { backupBusy, beginBackupRestore, finishBackupRestore } from "./backupState";
import { lockShelfBatchRecovery, shelfBatchRecoveryRequired } from "../../app/recoveryState";
import { notify } from "../../app/notifications";
import { settings } from "./settingsStore";
import { confirmAction } from "../../app/confirmation";

export type { BackupBusy } from "./backupState";
type RestoreOutcome = "notCommitted" | "committed" | "recovery";
type RestoreResult = Exclude<RestoreBackupResponse, { cancelled: true }> | WebDavBackupRestoreResponse;

function errorText(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

/** Backup screen owns actions; configuration comes from the shared settings. */
export function useBackupActions() {
  const session = getReaderBackupSession();
  const webDavBackupConfig = computed(() => settings.value.webdavBackup);
  const webDavBackupError = ref("");

  async function prepareBackupContents(): Promise<void> {
    if (session.hasActiveReaderMedia()) {
      if (!await session.pauseReaderMediaAndSave()) throw new Error("媒体播放位置尚未保存，备份已取消；请稍后重试。");
    } else if (session.readingBook.value && session.readerProgressDirty.value) {
      await session.saveCurrentProgress();
      if (session.readerProgressDirty.value) throw new Error("阅读位置尚未保存，备份已取消；请稍后重试。");
    }
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
    if (warning) notify(`${source}已恢复，但${warning}`, "error");
    else notify(`${source}已恢复，应用数据正在更新。`);
    return "committed";
  }

  async function createBackup(): Promise<void> {
    if (backupBusy.value || shelfBatchRecoveryRequired.value) return;
    backupBusy.value = "create";
    try {
      await prepareBackupContents();
      const result: BackupResponse = await createBackupFromPicker();
      if (result.cancelled) return;
      if (!result.backup) throw new Error("备份没有完成。");
      if (result.warning) {
        notify(result.warning, "error");
        return;
      }
      notify("备份文件已保存。请妥善保管其中的源与登录状态。");
    } catch (error) {
      notify(`创建备份失败：${errorText(error)}`, "error");
    } finally {
      backupBusy.value = null;
    }
  }

  async function restoreBackup(): Promise<void> {
    if (backupBusy.value || shelfBatchRecoveryRequired.value) return;
    if (!await confirmAction("从备份恢复？", "恢复备份会替换当前书架、阅读数据、源和登录状态。", "选择备份文件")) return;
    if (backupBusy.value || shelfBatchRecoveryRequired.value) return;
    beginBackupRestore();
    try {
      const result: RestoreBackupResponse = await restoreBackupFromPicker();
      if ("cancelled" in result) return;
      const outcome = await consumeRestoreOutcome(result, "本地备份");
      if (outcome === "notCommitted") return;
    } catch (error) {
      await lockRestoreRecovery(
        "备份恢复状态未确认",
        `没有收到本地备份的结构化恢复结果，无法确认数据是否已替换。旧阅读器已关闭，本会话已暂停书架修改；请重启应用检查。应用返回信息：${errorText(error)}`,
      );
      notify("备份恢复状态未知，旧阅读器已关闭；请重启应用检查。", "error");
    } finally {
      finishBackupRestore();
    }
  }

  function saveWebDavConfig(input: { url: string; username: string; password: string }): boolean {
    if (backupBusy.value || shelfBatchRecoveryRequired.value) return false;
    webDavBackupError.value = "";
    try {
      const url = new URL(input.url.trim());
      if (!["http:", "https:"].includes(url.protocol) || !url.hostname || url.username || url.password || url.search || url.hash) {
        throw new Error("请填写不含凭据、查询参数或片段的 HTTP(S) 目录地址。");
      }
      const username = input.username.trim();
      const previous = settings.value.webdavBackup;
      const sameRemote = previous?.url === url.href && previous.username === username;
      settings.value.webdavBackup = {
        ...previous, url: url.href, username,
        password: input.password || (sameRemote ? previous.password : null),
      };
      notify("WebDAV 连接设置已更新。");
      return true;
    } catch (error) {
      webDavBackupError.value = errorText(error);
      return false;
    }
  }

  async function clearWebDavPassword(): Promise<void> {
    const config = settings.value.webdavBackup;
    if (backupBusy.value || shelfBatchRecoveryRequired.value || !config?.password) return;
    if (!await confirmAction("清除 WebDAV 密码？", "下次连接时需要重新输入密码。", "清除密码")) return;
    if (backupBusy.value || shelfBatchRecoveryRequired.value || settings.value.webdavBackup !== config) return;
    config.password = null;
    notify("WebDAV 密码已清除。");
  }

  async function uploadWebDavBackupManually(): Promise<void> {
    if (backupBusy.value || shelfBatchRecoveryRequired.value
      || !webDavBackupConfig.value?.url) return;
    backupBusy.value = "webdav-upload";
    webDavBackupError.value = "";
    try {
      await prepareBackupContents();
      const result: WebDavBackupUploadResponse = await uploadWebDavBackup();
      if (!result.uploaded) throw new Error("备份没有上传完成。");
      const warning = result.warning?.trim() ?? "";
      if (warning) {
        webDavBackupError.value = `备份已上传，但${warning}`;
        notify(webDavBackupError.value, "error");
      } else {
        notify("WebDAV 备份已上传。");
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
    if (backupBusy.value || shelfBatchRecoveryRequired.value
      || !config?.url) return;
    if (!await confirmAction("恢复 WebDAV 备份？", "将替换当前本地书架、阅读数据、源和设置，恢复成功后重新载入应用数据。", "恢复备份")) return;
    if (backupBusy.value || shelfBatchRecoveryRequired.value || webDavBackupConfig.value !== config) return;
    beginBackupRestore();
    webDavBackupError.value = "";
    try {
      const result: WebDavBackupRestoreResponse = await restoreWebDavBackup();
      const outcome = await consumeRestoreOutcome(result, "WebDAV 备份");
      if (outcome === "notCommitted") {
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
      finishBackupRestore();
    }
  }

  return {
    backupBusy,
    webDavBackupConfig,
    webDavBackupError,
    prepareBackupContents,
    createBackup,
    restoreBackup,
    saveWebDavConfig,
    clearWebDavPassword,
    uploadWebDavBackupManually,
    restoreWebDavBackupManually,
  };
}
