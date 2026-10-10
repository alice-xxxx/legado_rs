import { computed, ref, type Ref } from "vue";
import { isTauri } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { getCurrent, onOpenUrl } from "@tauri-apps/plugin-deep-link";
import { cancelPendingPdfImport, discardExternalFile, importExternalFile, listPendingExternalFiles } from "../../api/books";
import { type LocalBookImportResponse, type PendingExternalFile } from "../../api/types";
import router, { routeNames } from "../../router";
import type { ReaderSession } from "../reader/readerSession";

type PendingImportLink = { kind: "source" | "rules"; url: string };

interface ExternalImportOptions {
  unlisteners: UnlistenFn[];
  bootstrapped: Readonly<Ref<boolean>>;
  backupBusy: Readonly<Ref<unknown>>;
  recoveryRequired: Readonly<Ref<boolean>>;
  getRestoreRevision: () => number;
  getReaderSession: () => ReaderSession;
  localBookImportBusy: Ref<boolean>;
  externalFileRequestBusy: Ref<boolean>;
  openPdfPasswordPrompt: (importToken: string) => void;
  acceptLocalBookImport: (response: LocalBookImportResponse) => Promise<void>;
  sourceImportOpen: Readonly<Ref<boolean>>;
  openSourceImport: (url: string) => Promise<void>;
  pdfPasswordPromptOpen: Readonly<Ref<boolean>>;
  notify: (message: string, kind?: "success" | "error") => void;
  errorText: (error: unknown) => string;
}

/** 管理系统文件打开请求与应用导入链接的接收、确认和跳转。 */
export function useExternalImportActions(options: ExternalImportOptions) {
  const pendingImportLinks = ref<PendingImportLink[]>([]);
  const importLinkOpening = ref(false);
  const pendingExternalFiles = ref<PendingExternalFile[]>([]);
  const externalFileRequestBusy = options.externalFileRequestBusy;
  let externalFilesInitialReadPending = false;
  const initialHandledExternalFiles = new Set<string>();
  let disposed = false;

  const externalFileRequestBlocked = computed(() => externalFileRequestBusy.value || options.localBookImportBusy.value
    || options.pdfPasswordPromptOpen.value || !options.bootstrapped.value || !!options.backupBusy.value
    || options.recoveryRequired.value);
  const importLinkBlocked = computed(() => importLinkOpening.value || !options.bootstrapped.value || !!options.backupBusy.value
    || options.recoveryRequired.value || options.sourceImportOpen.value);

  async function closeReaderForImport(restoreRevision: number): Promise<boolean> {
    const reader = options.getReaderSession();
    if (reader.readerBusy.value) return false;
    if (reader.readerVisible.value) {
      await reader.closeReader();
      if (reader.readerVisible.value || restoreRevision !== options.getRestoreRevision()) return false;
    }
    return restoreRevision === options.getRestoreRevision() && !reader.readerBusy.value;
  }

  function receiveExternalFile(file: PendingExternalFile): void {
    if (disposed || initialHandledExternalFiles.has(file.token)
      || pendingExternalFiles.value.some((entry) => entry.token === file.token)) return;
    if (pendingExternalFiles.value.length < 8) pendingExternalFiles.value.push(file);
  }

  async function setupExternalFiles(): Promise<void> {
    if (!isTauri()) return;
    externalFilesInitialReadPending = true;
    try {
      const unlisten = await listen<PendingExternalFile>("external-file-opened", (event) => receiveExternalFile(event.payload));
      if (disposed) {
        unlisten();
        return;
      }
      options.unlisteners.push(unlisten);
      const result = await listPendingExternalFiles();
      if (!disposed) for (const file of result.files) receiveExternalFile(file);
    } catch (error) {
      if (!disposed) options.notify(`读取外部文件请求失败：${options.errorText(error)}`, "error");
    } finally {
      externalFilesInitialReadPending = false;
      initialHandledExternalFiles.clear();
    }
  }

  function removeExternalFileRequest(token: string): void {
    pendingExternalFiles.value = pendingExternalFiles.value.filter((file) => file.token !== token);
    if (externalFilesInitialReadPending) initialHandledExternalFiles.add(token);
  }

  async function acceptExternalFileRequest(): Promise<void> {
    const file = pendingExternalFiles.value[0];
    if (!file || externalFileRequestBlocked.value) return;
    const restoreRevision = options.getRestoreRevision();
    externalFileRequestBusy.value = true;
    try {
      if (!await closeReaderForImport(restoreRevision)) return;
      if (restoreRevision !== options.getRestoreRevision() || options.backupBusy.value || options.recoveryRequired.value) return;
      options.localBookImportBusy.value = true;
      const response = await importExternalFile(file.token);
      if (restoreRevision !== options.getRestoreRevision()) {
        if (response.importToken) void cancelPendingPdfImport(response.importToken).catch(() => {});
        return;
      }
      removeExternalFileRequest(file.token);
      if (response.passwordRequired) {
        if (!response.importToken) throw new Error("无法继续打开加密文件，请重新打开文件。");
        options.openPdfPasswordPrompt(response.importToken);
        return;
      }
      if (!response.cancelled) {
        await router.push({ name: routeNames.shelf });
        if (restoreRevision !== options.getRestoreRevision()) return;
        await options.acceptLocalBookImport(response);
      }
    } catch (error) {
      if (restoreRevision !== options.getRestoreRevision()) return;
      options.notify(`导入外部文件失败：${options.errorText(error)}`, "error");
      try {
        const refreshed = await listPendingExternalFiles();
        if (!refreshed.files.some((entry) => entry.token === file.token)) removeExternalFileRequest(file.token);
      } catch {
        removeExternalFileRequest(file.token);
        options.notify("文件请求状态无法确认，请从系统重新打开文件。", "error");
      }
    } finally {
      options.localBookImportBusy.value = false;
      externalFileRequestBusy.value = false;
    }
  }

  async function ignoreExternalFileRequest(): Promise<void> {
    const file = pendingExternalFiles.value[0];
    if (!file || externalFileRequestBusy.value || options.recoveryRequired.value) return;
    const restoreRevision = options.getRestoreRevision();
    externalFileRequestBusy.value = true;
    try {
      await discardExternalFile(file.token);
      if (restoreRevision === options.getRestoreRevision()) removeExternalFileRequest(file.token);
    } catch (error) {
      if (restoreRevision === options.getRestoreRevision()) {
        options.notify(`取消文件请求失败：${options.errorText(error)}`, "error");
      }
    } finally {
      externalFileRequestBusy.value = false;
    }
  }

  function receiveImportLinks(urls: string[]): void {
    for (const raw of urls) {
      try {
        if (raw.length > 8192) throw new Error("导入链接过长。");
        const link = new URL(raw);
        if (link.protocol !== "legado-rs:") continue;
        if (link.username || link.password || (link.pathname && link.pathname !== "/") || link.hash
          || (link.hostname !== "import-source" && link.hostname !== "import-rules")
          || link.searchParams.getAll("url").length !== 1) throw new Error("不支持这个导入链接。");
        const target = link.searchParams.get("url")!;
        if (target.length > 4096) throw new Error("导入地址过长。");
        const targetUrl = new URL(target);
        if (!/^https?:$/.test(targetUrl.protocol) || targetUrl.username || targetUrl.password) {
          throw new Error("导入地址必须是 HTTP 或 HTTPS 链接，且不能包含用户名和密码。");
        }
        const kind = link.hostname === "import-source" ? "source" : "rules";
        if (pendingImportLinks.value.some((entry) => entry.kind === kind && entry.url === target)) continue;
        if (pendingImportLinks.value.length >= 8) throw new Error("待处理的导入链接过多，请先处理已有链接。");
        pendingImportLinks.value.push({ kind, url: target });
      } catch (error) {
        options.notify(`无法打开导入链接：${options.errorText(error)}`, "error");
      }
    }
  }

  async function setupImportLinks(): Promise<void> {
    if (!isTauri()) return;
    try {
      const unlisten = await onOpenUrl((urls) => {
        if (!disposed) receiveImportLinks(urls);
      });
      if (disposed) {
        unlisten();
        return;
      }
      options.unlisteners.push(unlisten);
      const current = await getCurrent();
      if (!disposed && current) receiveImportLinks(current);
    } catch (error) {
      if (!disposed) options.notify(`导入链接接收失败：${options.errorText(error)}`, "error");
    }
  }

  async function openPendingImportLink(): Promise<void> {
    const pending = pendingImportLinks.value[0];
    if (!pending || importLinkBlocked.value) return;
    const restoreRevision = options.getRestoreRevision();
    importLinkOpening.value = true;
    try {
      if (!await closeReaderForImport(restoreRevision)) return;
      if (restoreRevision !== options.getRestoreRevision() || pendingImportLinks.value[0] !== pending || options.recoveryRequired.value || options.backupBusy.value) return;
      if (pending.kind === "source") {
        if (options.sourceImportOpen.value) return;
        await options.openSourceImport(pending.url);
        if (restoreRevision !== options.getRestoreRevision()) return;
      } else {
        await router.push({ name: routeNames.settingsRules, query: { importUrl: pending.url } });
        if (restoreRevision !== options.getRestoreRevision()) return;
      }
      pendingImportLinks.value.shift();
    } catch (error) {
      if (restoreRevision === options.getRestoreRevision()) {
        options.notify(`Unable to open import confirmation page: ${options.errorText(error)}`, "error");
      }
    } finally {
      importLinkOpening.value = false;
    }
  }

  function dispose(): void {
    disposed = true;
  }

  return {
    pendingImportLinks,
    importLinkOpening,
    pendingExternalFiles,
    externalFileRequestBusy,
    externalFileRequestBlocked,
    importLinkBlocked,
    receiveExternalFile,
    setupExternalFiles,
    removeExternalFileRequest,
    acceptExternalFileRequest,
    ignoreExternalFileRequest,
    receiveImportLinks,
    setupImportLinks,
    openPendingImportLink,
    dispose,
  };
}
