import { ref } from "vue";
import { prepareConfigurationImport } from "../../api/settings";
import type { DisplayReplacementRule } from "../../api/types";
import { replacementRules, replacementRuleStore } from "./configurationStores";

type UseReplacementRulesOptions = {
  getRestoreRevision: () => number;
  notify: (message: string, kind?: "success" | "error") => void;
  errorText: (error: unknown) => string;
};

export function useReplacementRules(options: UseReplacementRulesOptions) {
  const replacementImportOpen = ref(false);
  const replacementImportBusy = ref(false);
  const replacementImportUrl = ref("");
  const replacementImportError = ref("");

  function createReplacementRule(rule: DisplayReplacementRule): string {
    const id = `rule-${crypto.randomUUID()}`;
    replacementRuleStore.update({ ...rule, id });
    return id;
  }
  function updateReplacementRule(ruleId: string, patch: Partial<DisplayReplacementRule>): void {
    const rule = replacementRules.value.find((entry) => entry.id === ruleId);
    if (rule) replacementRuleStore.update({ ...rule, ...patch, id: ruleId });
  }
  function saveReplacementRule(ruleId: string, patch: Partial<DisplayReplacementRule>): void {
    updateReplacementRule(ruleId, patch);
  }
  function removeReplacements(ruleIds: string[]): void {
    ruleIds.forEach(id => replacementRuleStore.remove(id));
  }
  const removeReplacement = (id: string) => removeReplacements([id]);
  function openReplacementRulesUrlImport(): boolean {
    replacementImportOpen.value = true;
    replacementImportError.value = "";
    return true;
  }
  function cancelReplacementRulesUrlImport(): void {
    if (replacementImportBusy.value) return;
    replacementImportOpen.value = false;
    replacementImportUrl.value = "";
    replacementImportError.value = "";
  }
  async function importRules(kind: "url" | "local"): Promise<void> {
    if (replacementImportBusy.value) return;
    const revision = options.getRestoreRevision();
    replacementImportBusy.value = true;
    replacementImportError.value = "";
    try {
      const result = await prepareConfigurationImport<{ rules: DisplayReplacementRule[] }>("replacementRules", kind === "url"
        ? { kind, url: replacementImportUrl.value.trim() } : { kind });
      if (revision !== options.getRestoreRevision() || !result) return;
      for (const rule of result.rules) {
        const exists = replacementRules.value.some(entry => entry.pattern === rule.pattern
          && entry.replacement === rule.replacement && entry.enabled === rule.enabled
          && entry.isRegex === rule.isRegex && entry.scope === rule.scope);
        if (!exists) replacementRuleStore.update(rule);
      }
      replacementImportOpen.value = false;
      options.notify("显示替换规则已导入。");
    } catch (error) {
      replacementImportError.value = options.errorText(error);
    } finally { replacementImportBusy.value = false; }
  }
  function resetForRestore(): void {
    replacementImportOpen.value = false;
    replacementImportUrl.value = "";
    replacementImportError.value = "";
  }
  return {
    replacementImportOpen, replacementImportBusy, replacementImportUrl,
    replacementImportError, createReplacementRule,
    updateReplacementRule, saveReplacementRule, removeReplacement, removeReplacements,
    openReplacementRulesUrlImport, cancelReplacementRulesUrlImport,
    importReplacementRulesByUrl: () => importRules("url"),
    importReplacementRulesFromFile: () => importRules("local"),
    resetForRestore,
  };
}
