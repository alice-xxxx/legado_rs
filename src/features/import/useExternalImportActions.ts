import { backupBusy } from "../settings/backupState";
import { shelfBatchRecoveryRequired } from "../../app/recoveryState";
import { computed, ref, watch, type Ref } from "vue";
import { isTauri } from "@tauri-apps/api/core";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { getCurrent, onOpenUrl } from "@tauri-apps/plugin-deep-link";
import { type DisplayReplacementRule, type HttpTtsConfigEntry, type TxtTocRule } from "../../api/types";
import { prepareDeepLinkImport } from "../../api/settings";
import { readMoreInformation, readSources, readSourceDefinition } from "../../api/database";
import { moreInformation } from "../../api/moreInformation";
import { libraryType } from "../books/contentLibraryState";
import { importSourceDefinitions } from "../sources/sourcesStore";
import { replacementRuleStore, txtTocRuleStore, httpTtsStore } from "../settings/configurationStores";
import { requestBookDetailOpen, discardBookDetailOpenRequest } from "../books/bookDetailOpenRequest";
import { parseDeepLink, parseImportInput, deepLinkLabels, type ImportKind, type PendingImportLink } from "./deepLinks";
import router, { routeNames } from "../../router";
import type { ReaderSession } from "../reader/readerSession";

type ImportEntry = { key: string; name: string; data: Record<string, unknown>; json?: string };

interface ExternalImportOptions {
  unlisteners: UnlistenFn[];
  bootstrapped: Readonly<Ref<boolean>>;
  getRestoreRevision: () => number;
  getReaderSession: () => ReaderSession;
  localBookImportBusy: Ref<boolean>;
  sourceImportOpen: Readonly<Ref<boolean>>;
  notify: (message: string, kind?: "success" | "error") => void;
  errorText: (error: unknown) => string;
}

/** 管理系统文件打开请求与应用导入链接的接收、确认和跳转。 */
export function useExternalImportActions(options: ExternalImportOptions) {
  const pendingImportLinks = ref<PendingImportLink[]>([]);
  const importLinkOpening = ref(false);
  const activeImportLink = ref<PendingImportLink | null>(null);
  const importDialogOpen = computed(() => activeImportLink.value !== null);
  const importKind = ref<ImportKind | null>(null);
  const importEntries = ref<ImportEntry[]>([]);
  const selectedImportKeys = ref<string[]>([]);
  const importError = ref("");
  const importTitle = computed(() => `导入${deepLinkLabels[importKind.value ?? activeImportLink.value?.kind ?? "auto"]}`);
  let importGeneration = 0;
  let disposed = false;

  const importLinkBlocked = computed(() => importLinkOpening.value || !options.bootstrapped.value || !!backupBusy.value
    || shelfBatchRecoveryRequired.value || options.sourceImportOpen.value || importDialogOpen.value
    || options.localBookImportBusy.value
    || options.getReaderSession().readerBusy.value);

  async function closeReaderForImport(restoreRevision: number): Promise<boolean> {
    const reader = options.getReaderSession();
    if (reader.readerBusy.value) return false;
    if (reader.readerVisible.value) {
      await reader.closeReader();
      if (reader.readerVisible.value || restoreRevision !== options.getRestoreRevision()) return false;
    }
    return restoreRevision === options.getRestoreRevision() && !reader.readerBusy.value;
  }

  function receiveImportLinks(urls: string[]): void {
    for (const raw of urls) {
      try {
        const pending = parseDeepLink(raw);
        if (!pending) continue;
        enqueueImportLink(pending);
      } catch (error) {
        options.notify(`无法打开导入链接：${options.errorText(error)}`, "error");
      }
    }
  }

  function enqueueImportLink(pending: PendingImportLink): void {
    if (disposed || pendingImportLinks.value.some(entry => entry.kind === pending.kind && entry.url === pending.url)) return;
    if (pendingImportLinks.value.length >= 8) throw new Error("待处理的导入链接过多，请先处理已有链接。");
    pendingImportLinks.value.push(pending);
  }

  function requestLinkImport(input: string): void {
    enqueueImportLink(parseImportInput(input));
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
    const generation = ++importGeneration;
    const revision = options.getRestoreRevision();
    const isCurrent = () => !disposed && generation === importGeneration && revision === options.getRestoreRevision()
      && pendingImportLinks.value[0] === pending;
    importLinkOpening.value = true;
    try {
      if (!await closeReaderForImport(revision) || !isCurrent()) return;
      if (pending.kind === "read" || pending.kind === "addToBookshelf") {
        const optionStart = pending.url.search(/,\s*\{/);
        const url = new URL(optionStart < 0 ? pending.url : pending.url.slice(0, optionStart));
        const origin = optionStart < 0 ? undefined : JSON.parse(pending.url.slice(optionStart + 1)).origin;
        let sourceId = "";
        for (const source of (await readSources()).filter(source => source.enabled && !source.isRss)) {
          const definition = JSON.parse(await readSourceDefinition(source.id));
          const base = definition.bookSourceUrl;
          let matches = origin ? base === origin : false;
          if (!origin) {
            try { matches = new URL(base).origin === url.origin; } catch { /* 继续按 URL 规则匹配。 */ }
            if (!matches && definition.bookUrlPattern) {
              try { matches = new RegExp(`^(?:${definition.bookUrlPattern})$`).test(pending.url); } catch { /* 跳过无效规则。 */ }
            }
          }
          if (matches) { sourceId = source.id; break; }
        }
        if (!sourceId) throw new Error("未找到匹配书源");
        await moreInformation(sourceId, pending.url);
        if (!isCurrent()) return;
        const book = await readMoreInformation(sourceId, pending.url);
        const inShelf = libraryType(book.id) !== undefined;
        if (pending.kind === "addToBookshelf" && inShelf) throw new Error("已在书架");
        if (!isCurrent()) return;
        if (pending.kind === "read" && inShelf) {
          await router.push({ name: routeNames.shelf });
          if (isCurrent()) await options.getReaderSession().startReading(book);
        } else {
          if (router.currentRoute.value.name === routeNames.bookDetail) await router.push({ name: routeNames.shelf });
          if (!isCurrent()) return;
          const requestId = requestBookDetailOpen({ kind: "cached-book", bookId: book.id, initialTab: "info" });
          try {
            const failure = await router.push({ name: routeNames.bookDetail });
            if (failure) throw new Error("无法打开书籍详情");
          } catch (error) { discardBookDetailOpenRequest(requestId); throw error; }
        }
        if (isCurrent()) pendingImportLinks.value.shift();
        return;
      }
      if (pending.kind === "theme" || pending.kind === "dictRule" || pending.kind === "readConfig") {
        throw new Error("本应用尚未提供原版" + deepLinkLabels[pending.kind] + "导入");
      }
      activeImportLink.value = pending;
      importError.value = "";
      const prepared = await prepareDeepLinkImport(pending.kind, pending.url);
      if (!isCurrent()) return;
      importKind.value = prepared.kind;
      const items = prepared.document[prepared.kind === "sources" ? "sources" : prepared.kind === "httpTtsConfigs" ? "configs" : "rules"];
      if (!Array.isArray(items) || !items.length) throw new Error("没有可导入的内容");
      importEntries.value = items.map((item, index) => {
        const json = typeof item === "string" ? item : undefined;
        const data = json ? JSON.parse(json) as Record<string, unknown> : item as Record<string, unknown>;
        return { key: String(index), data, json,
          name: String(data.bookSourceName ?? data.sourceName ?? data.name ?? data.bookSourceUrl ?? data.sourceUrl ?? "未命名") };
      });
      selectedImportKeys.value = importEntries.value.map(entry => entry.key);
    } catch (error) {
      if (isCurrent()) {
        if (activeImportLink.value) importError.value = options.errorText(error);
        else { pendingImportLinks.value.shift(); options.notify(options.errorText(error), "error"); }
      }
    } finally {
      if (generation === importGeneration) importLinkOpening.value = false;
    }
  }

  function cancelPendingImport(consume = true): void {
    if (consume && activeImportLink.value === pendingImportLinks.value[0]) pendingImportLinks.value.shift();
    importGeneration++;
    activeImportLink.value = null;
    importKind.value = null;
    importEntries.value = [];
    selectedImportKeys.value = [];
    importError.value = "";
    importLinkOpening.value = false;
  }

  function retryPendingImport(): void {
    cancelPendingImport(false);
    void openPendingImportLink();
  }

  async function savePendingImport(): Promise<void> {
    const kind = importKind.value;
    if (!kind || importLinkOpening.value || backupBusy.value || shelfBatchRecoveryRequired.value) return;
    const keys = new Set(selectedImportKeys.value);
    const selected = importEntries.value.filter(entry => keys.has(entry.key));
    if (!selected.length) return;
    const generation = importGeneration, revision = options.getRestoreRevision();
    const isCurrent = () => !disposed && generation === importGeneration && revision === options.getRestoreRevision();
    importLinkOpening.value = true;
    importError.value = "";
    try {
      if (kind === "sources") await importSourceDefinitions(selected.map(entry => entry.json!));
      else for (const { data } of selected) {
        if (kind === "replacementRules") replacementRuleStore.update(data as unknown as DisplayReplacementRule);
        else if (kind === "txtTocRules") txtTocRuleStore.update(data as unknown as TxtTocRule);
        else httpTtsStore.update(data as unknown as HttpTtsConfigEntry);
      }
      if (!isCurrent()) return;
      options.notify("已导入 " + selected.length + " 项" + deepLinkLabels[kind] + "。");
      cancelPendingImport();
    } catch (error) { if (isCurrent()) importError.value = options.errorText(error); }
    finally { if (isCurrent()) importLinkOpening.value = false; }
  }

  let autoOpenedLink: PendingImportLink | undefined;
  const stopAutoOpen = watch([() => pendingImportLinks.value[0], importLinkBlocked], () => {
    const pending = pendingImportLinks.value[0];
    if (!importLinkBlocked.value && pending && autoOpenedLink !== pending) {
      autoOpenedLink = pending;
      void openPendingImportLink();
    }
  });

  function dispose(): void {
    disposed = true;
    importGeneration++;
    stopAutoOpen();
  }

  return {
    importDialogOpen, importKind, importEntries, selectedImportKeys, importError, importTitle,
    cancelPendingImport, retryPendingImport, savePendingImport,
    pendingImportLinks,
    importLinkOpening,
    importLinkBlocked,
    receiveImportLinks,
    requestLinkImport,
    setupImportLinks,
    openPendingImportLink,
    dispose,
  };
}
