import { onBeforeUnmount, ref } from "vue";
import { mutateReplacementRules } from "../../api/settings";
import { readResource } from "../../api/resources";
import { type DisplayReplacementRule, type ResourceDescriptor } from "../../api/types";

type UseReplacementRulesOptions = {
  refreshReaderDisplay?: () => void;
  getRestoreRevision: () => number;
  notify: (message: string, kind?: "success" | "error") => void;
  errorText: (error: unknown) => string;
};

/** Manages display replacement rules and import/export workflows. */
export function useReplacementRules(options: UseReplacementRulesOptions) {
  const { notify, errorText } = options;
  const refreshReaderDisplay = options.refreshReaderDisplay ?? (() => {});
  const replacementRules = ref<DisplayReplacementRule[]>([]);
  const replacementImportOpen = ref(false);
  const replacementImportBusy = ref(false);
  const replacementExportBusy = ref(false);
  const replacementRefreshBusy = ref(false);
  const replacementDeleteBusy = ref(false);
  const replacementAddBusy = ref(false);
  const replacementImportUrl = ref("");
  const replacementImportError = ref("");
  const replacementSaveTimers = new Map<string, ReturnType<typeof setTimeout>>();
  const replacementWrites = new Map<string, Promise<void>>();
  let replacementRulesRefreshGeneration = 0;
  let replacementRulesResource: ResourceDescriptor | null = null;
  function currentReplacementRulesResource(): ResourceDescriptor {
    if (!replacementRulesResource) throw new Error("显示替换规则资源尚未加载。");
    return replacementRulesResource;
  }

  async function refreshReplacementRules(descriptor: ResourceDescriptor): Promise<void> {
    const generation = ++replacementRulesRefreshGeneration;
    const restoreRevision = options.getRestoreRevision();
    replacementRulesResource = descriptor;
    const document = await readResource<{
      schemaVersion: number;
      rules: DisplayReplacementRule[];
    }>(descriptor);
    if (generation !== replacementRulesRefreshGeneration || restoreRevision !== options.getRestoreRevision()) return;
    const localById = new Map(replacementRules.value.map((rule) => [rule.id, rule]));
    const serverRules = Array.isArray(document.rules) ? document.rules : [];
    replacementRules.value = serverRules.map((serverRule) => {
      const displayRule = serverRule;
      const localRule = localById.get(serverRule.id);
      const hasPendingWrite = replacementSaveTimers.has(serverRule.id) || replacementWrites.has(serverRule.id);
      return localRule && hasPendingWrite && !replacementRulesEqual(localRule, displayRule)
        ? localRule
        : displayRule;
    });
    refreshReaderDisplay();
  }

  function replacementRulesEqual(left: DisplayReplacementRule, right: DisplayReplacementRule): boolean {
    return left.id === right.id
      && left.name === right.name
      && left.pattern === right.pattern
      && left.replacement === right.replacement
      && left.enabled === right.enabled
      && left.isRegex === right.isRegex
      && left.scope === right.scope;
  }

  async function flushReplacementRuleDrafts(): Promise<void> {
    const pendingIds = [...replacementSaveTimers.keys()];
    for (const [ruleId, timer] of replacementSaveTimers) {
      clearTimeout(timer);
      replacementSaveTimers.delete(ruleId);
    }
    await Promise.all(pendingIds.map((ruleId) => persistReplacementRule(ruleId)));
    await Promise.all([...replacementWrites.values()]);

    const savedDocument = await readResource<{ schemaVersion: number; rules: DisplayReplacementRule[] }>(
      currentReplacementRulesResource(),
    );
    const savedRules = new Map(savedDocument.rules.map((rule) => [rule.id, rule]));
    if (replacementRules.value.some((rule) => {
      const savedRule = savedRules.get(rule.id);
      return !savedRule || !replacementRulesEqual(rule, savedRule);
    })) {
      throw new Error("有显示替换编辑尚未成功保存，请稍后重试。");
    }
  }

  async function reloadReplacementRules(): Promise<void> {
    if (replacementRefreshBusy.value || replacementImportBusy.value || replacementExportBusy.value
      || replacementDeleteBusy.value || replacementAddBusy.value) return;
    const restoreRevision = options.getRestoreRevision();
    const isCurrent = () => restoreRevision === options.getRestoreRevision();
    replacementRefreshBusy.value = true;
    try {
      await flushReplacementRuleDrafts();
      if (!isCurrent()) return;
      await refreshReplacementRules(currentReplacementRulesResource());
      if (isCurrent()) notify("显示替换规则已刷新。");
    } catch (error) {
      if (isCurrent()) notify(`刷新显示替换规则失败：${errorText(error)}`, "error");
    } finally {
      if (isCurrent()) replacementRefreshBusy.value = false;
    }
  }

  async function createReplacementRule(rule: DisplayReplacementRule): Promise<string | null> {
    const restoreRevision = options.getRestoreRevision();
    const isCurrent = () => restoreRevision === options.getRestoreRevision();
    if (replacementAddBusy.value || replacementDeleteBusy.value
      || replacementExportBusy.value || replacementImportBusy.value) return null;
    replacementAddBusy.value = true;
    try {
      const result = await mutateReplacementRules({
        kind: "save",
        id: null,
        json: JSON.stringify({
          name: rule.name,
          pattern: rule.pattern,
          replacement: rule.replacement,
          enabled: rule.enabled,
          isRegex: rule.isRegex,
          scope: rule.scope,
        }),
      });
      if ("cancelled" in result) return null;
      if (!isCurrent()) return null;
      if (!("savedId" in result)) throw new Error("保存规则没有返回已提交的规则 ID");
      replacementRulesResource = result.resource;
      try {
        await refreshReplacementRules(result.resource);
      } catch (error) {
        if (isCurrent()) notify(`已添加显示替换规则，但刷新列表失败：${errorText(error)}`, "error");
        return isCurrent() ? result.savedId : null;
      }
      if (!isCurrent()) return null;
      notify("已添加显示替换规则。");
      return result.savedId;
    } catch (error) {
      if (isCurrent()) notify(`添加显示替换失败：${errorText(error)}`, "error");
      return null;
    } finally {
      if (isCurrent()) replacementAddBusy.value = false;
    }
  }

  function openReplacementRulesUrlImport(): boolean {
    if (replacementImportOpen.value || replacementImportBusy.value || replacementExportBusy.value
      || replacementDeleteBusy.value || replacementAddBusy.value) return false;
    replacementImportUrl.value = "";
    replacementImportError.value = "";
    replacementImportOpen.value = true;
    return true;
  }

  function cancelReplacementRulesUrlImport(): void {
    if (replacementImportBusy.value || replacementExportBusy.value
      || replacementDeleteBusy.value || replacementAddBusy.value) return;
    replacementImportOpen.value = false;
    replacementImportUrl.value = "";
    replacementImportError.value = "";
  }

  async function importReplacementRulesByUrl(): Promise<void> {
    const restoreRevision = options.getRestoreRevision();
    const isCurrent = () => restoreRevision === options.getRestoreRevision();
    const url = replacementImportUrl.value.trim();
    if (!url || replacementImportBusy.value || replacementExportBusy.value
      || replacementDeleteBusy.value || replacementAddBusy.value) return;
    replacementImportBusy.value = true;
    replacementImportError.value = "";
    try {
      await flushReplacementRuleDrafts();
      if (!isCurrent()) return;
      const result = await mutateReplacementRules({ kind: "url", url });
      if (!isCurrent() || "cancelled" in result) return;
      await refreshReplacementRules(result.resource);
      if (!isCurrent()) return;
      replacementImportOpen.value = false;
      replacementImportUrl.value = "";
      const importedCount = "importedCount" in result ? result.importedCount : undefined;
      if (isCurrent()) notify(importedCount
        ? `已导入 ${importedCount} 条新显示替换规则。`
        : "链接中的显示替换规则均已存在，没有重复添加。");
    } catch (error) {
      if (isCurrent()) replacementImportError.value = `导入显示替换规则失败：${errorText(error)}`;
    } finally {
      if (isCurrent()) replacementImportBusy.value = false;
    }
  }

  async function importReplacementRulesFromFile(): Promise<void> {
    const restoreRevision = options.getRestoreRevision();
    const isCurrent = () => restoreRevision === options.getRestoreRevision();
    if (replacementImportBusy.value || replacementExportBusy.value
      || replacementDeleteBusy.value || replacementAddBusy.value) return;
    replacementImportBusy.value = true;
    try {
      await flushReplacementRuleDrafts();
      if (!isCurrent()) return;
      const result = await mutateReplacementRules({ kind: "local" });
      if (!isCurrent()) return;
      if ("cancelled" in result) return;
      await refreshReplacementRules(result.resource);
      if (!isCurrent()) return;
      const importedCount = "importedCount" in result ? result.importedCount : undefined;
      if (isCurrent()) notify(importedCount
        ? `已从文件导入 ${importedCount} 条新显示替换规则。`
        : "文件中的显示替换规则均已存在，没有重复添加。");
    } catch (error) {
      if (isCurrent()) notify(`从文件导入显示替换规则失败：${errorText(error)}`, "error");
    } finally {
      if (isCurrent()) replacementImportBusy.value = false;
    }
  }

  async function exportReplacementRulesToFile(): Promise<void> {
    const restoreRevision = options.getRestoreRevision();
    const isCurrent = () => restoreRevision === options.getRestoreRevision();
    const rules = replacementRules.value;
    if (!replacementRules.value.length || replacementExportBusy.value || replacementImportBusy.value
      || replacementRefreshBusy.value
      || replacementDeleteBusy.value || replacementAddBusy.value) return;
    replacementExportBusy.value = true;
    try {
      const pendingIds = [...replacementSaveTimers.keys()];
      for (const [ruleId, timer] of replacementSaveTimers) {
        clearTimeout(timer);
        replacementSaveTimers.delete(ruleId);
      }
      await Promise.all(pendingIds.map((ruleId) => persistReplacementRule(ruleId)));
      if (!isCurrent()) return;
      await Promise.all([...replacementWrites.values()]);
      if (!isCurrent()) return;

      const savedDocument = await readResource<{ schemaVersion: number; rules: DisplayReplacementRule[] }>(
        currentReplacementRulesResource(),
      );
      if (!isCurrent()) return;
      const savedRules = new Map(savedDocument.rules.map((rule) => [rule.id, rule]));
      if (rules.some((rule) => {
        const savedRule = savedRules.get(rule.id);
        return !savedRule || !replacementRulesEqual(rule, savedRule);
      })) {
        throw new Error("有显示替换规则尚未成功保存，请稍后重试。");
      }

      const exportUrl = URL.createObjectURL(new Blob(
        [JSON.stringify(savedDocument, null, 2)],
        { type: "application/json" },
      ));
      const anchor = window.document.createElement("a");
      anchor.href = exportUrl;
      anchor.download = "display-replacement-rules.json";
      window.document.body.append(anchor);
      try {
        anchor.click();
      } finally {
        anchor.remove();
        window.setTimeout(() => URL.revokeObjectURL(exportUrl), 1000);
      }
      if (isCurrent()) notify(`已导出 ${savedDocument.rules.length} 条显示替换规则。`);
    } catch (error) {
      if (isCurrent()) notify(`导出显示替换规则失败：${errorText(error)}`, "error");
    } finally {
      if (isCurrent()) replacementExportBusy.value = false;
    }
  }

  async function removeReplacement(ruleId: string): Promise<void> {
    await removeReplacements([ruleId]);
  }

  async function removeReplacements(ruleIds: string[]): Promise<void> {
    const targets = replacementRules.value.filter((rule) => ruleIds.includes(rule.id));
    if (!targets.length || replacementDeleteBusy.value || replacementImportBusy.value
      || replacementExportBusy.value || replacementRefreshBusy.value || replacementAddBusy.value) return;
    const targetIds = targets.map((rule) => rule.id);
    const prompt = targets.length === 1
      ? `确认删除显示替换规则“${targets[0].name}”？`
      : `确认删除选中的 ${targets.length} 条显示替换规则？`;
    if (!window.confirm(prompt)) return;

    const restoreRevision = options.getRestoreRevision();
    const isCurrent = () => restoreRevision === options.getRestoreRevision();
    const pendingIds = [...replacementSaveTimers.keys()];
    replacementDeleteBusy.value = true;
    let deleted = false;
    try {
      await flushReplacementRuleDrafts();
      if (!isCurrent()) return;
      const result = await mutateReplacementRules({ kind: "delete", ruleIds: targetIds });
      if ("cancelled" in result) return;
      deleted = true;
      if (!isCurrent()) return;
      replacementRules.value = replacementRules.value.filter((entry) => !targetIds.includes(entry.id));
      refreshReaderDisplay();
      try {
        await refreshReplacementRules(result.resource);
      } catch (error) {
        if (isCurrent()) console.error("Deleted replacement rules could not be refreshed:", error);
      }
      if (isCurrent()) notify(targets.length === 1
        ? "显示替换规则已删除。"
        : `已删除 ${targets.length} 条显示替换规则。`);
    } catch (error) {
      if (isCurrent()) notify("删除显示替换规则失败：" + errorText(error), "error");
    } finally {
      if (isCurrent()) {
        replacementDeleteBusy.value = false;
        if (!deleted) {
          for (const ruleId of pendingIds) {
            if (!replacementRules.value.some((entry) => entry.id === ruleId) || replacementSaveTimers.has(ruleId)) continue;
            const retryTimer = setTimeout(() => {
              if (!isCurrent() || replacementSaveTimers.get(ruleId) !== retryTimer) return;
              replacementSaveTimers.delete(ruleId);
              void persistReplacementRule(ruleId);
            }, 500);
            replacementSaveTimers.set(ruleId, retryTimer);
          }
        }
      }
    }
  }

  function updateReplacementRule(ruleId: string, patch: Partial<DisplayReplacementRule>): void {
    if (replacementImportBusy.value || replacementDeleteBusy.value
      || replacementExportBusy.value || replacementRefreshBusy.value) return;
    const currentRule = replacementRules.value.find((rule) => rule.id === ruleId);
    if (!currentRule) return;
    const nextPatch = patch.pattern === "" && currentRule.enabled ? { ...patch, enabled: false } : patch;
    replacementRules.value = replacementRules.value.map((rule) => rule.id === ruleId ? { ...rule, ...nextPatch } : rule);
    refreshReaderDisplay();
    const oldTimer = replacementSaveTimers.get(ruleId);
    if (oldTimer) clearTimeout(oldTimer);
    const restoreRevision = options.getRestoreRevision();
    const timer = setTimeout(() => {
      if (restoreRevision !== options.getRestoreRevision() || replacementSaveTimers.get(ruleId) !== timer) return;
      replacementSaveTimers.delete(ruleId);
      void persistReplacementRule(ruleId);
    }, 500);
    replacementSaveTimers.set(ruleId, timer);
  }

  async function persistReplacementRule(ruleId: string): Promise<boolean> {
    const restoreRevision = options.getRestoreRevision();
    const isCurrent = () => restoreRevision === options.getRestoreRevision();
    const previous = replacementWrites.get(ruleId) ?? Promise.resolve();
    const next = previous.catch(() => {}).then(async () => {
      // A delayed draft from the pre-restore library must never write back
      // into the newly restored database once the earlier write completes.
      if (!isCurrent()) return;
      const rule = replacementRules.value.find((entry) => entry.id === ruleId);
      if (!rule) return;
      const snapshot = { ...rule };
      if (!isCurrent()) return;
      const { id, ...ruleJson } = snapshot;
      const result = await mutateReplacementRules({ kind: "save", id, json: JSON.stringify(ruleJson) });
      if ("cancelled" in result) return;
      if (!isCurrent()) return;
      if (!("savedId" in result) || result.savedId !== id) throw new Error("保存规则没有返回已提交的规则 ID");
      replacementRulesResource = result.resource;
      const document = await readResource<{ schemaVersion: number; rules: DisplayReplacementRule[] }>(result.resource);
      if (!isCurrent()) return;
      const savedRule = document.rules.find((saved) => saved.id === ruleId);
      const currentRule = replacementRules.value.find((entry) => entry.id === ruleId);
      if (savedRule && currentRule && replacementRulesEqual(currentRule, snapshot)) {
        replacementRules.value = replacementRules.value.map((entry) => entry.id === ruleId ? savedRule : entry);
      }
      if (isCurrent()) refreshReaderDisplay();
    });
    replacementWrites.set(ruleId, next);
    try {
      await next;
      return isCurrent() && replacementRules.value.some((entry) => entry.id === ruleId);
    } catch (error) {
      if (isCurrent()) notify(`保存显示替换失败：${errorText(error)}`, "error");
      return false;
    } finally {
      if (replacementWrites.get(ruleId) === next) replacementWrites.delete(ruleId);
    }
  }

  /** 编辑页的保存按钮立即写入；列表中的快速操作继续使用防抖写入。 */
  async function saveReplacementRule(ruleId: string, patch: Partial<DisplayReplacementRule>): Promise<boolean> {
    updateReplacementRule(ruleId, patch);
    const timer = replacementSaveTimers.get(ruleId);
    if (timer) {
      clearTimeout(timer);
      replacementSaveTimers.delete(ruleId);
    }
    return persistReplacementRule(ruleId);
  }

  function resetForRestore(): void {
    replacementRulesRefreshGeneration += 1;
    for (const timer of replacementSaveTimers.values()) clearTimeout(timer);
    replacementSaveTimers.clear();
    // In-flight writes are guarded by their originating restore revision.
    replacementWrites.clear();
    replacementRulesResource = null;
    replacementRules.value = [];
    replacementImportOpen.value = false;
    replacementImportBusy.value = false;
    replacementExportBusy.value = false;
    replacementRefreshBusy.value = false;
    replacementDeleteBusy.value = false;
    replacementAddBusy.value = false;
    replacementImportUrl.value = "";
    replacementImportError.value = "";
  }

  onBeforeUnmount(() => {
    for (const [ruleId, timer] of replacementSaveTimers) {
      clearTimeout(timer);
      replacementSaveTimers.delete(ruleId);
      void persistReplacementRule(ruleId);
    }
  });

  return {
    resetForRestore,
    replacementRules,
    replacementImportOpen,
    replacementImportBusy,
    replacementExportBusy,
    replacementRefreshBusy,
    replacementDeleteBusy,
    replacementAddBusy,
    replacementImportUrl,
    replacementImportError,
    refreshReplacementRules,
    reloadReplacementRules,
    createReplacementRule,
    openReplacementRulesUrlImport,
    cancelReplacementRulesUrlImport,
    importReplacementRulesByUrl,
    importReplacementRulesFromFile,
    exportReplacementRulesToFile,
    updateReplacementRule,
    saveReplacementRule,
    removeReplacement,
    removeReplacements,
  };
}
