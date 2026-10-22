import type { TxtTocRulesDocument } from "../../api/settings";
import { computed, nextTick, ref, type Ref } from "vue";
import { listen } from "@tauri-apps/api/event";
import { modifyDatabase, type DatabaseOperation } from "../../api/feDb";
import { readConfiguration } from "../../api/database";
import type { AppBootstrap, DisplayReplacementRule, HttpTtsConfigResource } from "../../api/types";

type ConfigurationKind = "replacementRules" | "txtTocRules" | "httpTtsConfigs";
function ruleChange(kind: ConfigurationKind, id: string, rule: unknown): DatabaseOperation {
  if (kind === "replacementRules") return rule
    ? { operation: "saveReplacementRules", rules: [rule as DisplayReplacementRule] }
    : { operation: "deleteReplacementRule", id };
  if (kind === "txtTocRules") return rule
    ? { operation: "saveTxtTocRules", rules: [rule as TxtTocRulesDocument["rules"][number]] }
    : { operation: "deleteTxtTocRule", id };
  if (!rule) return { operation: "deleteHttpTts", id };
  const config = rule as HttpTtsConfigResource["configs"][number];
  return { operation: "saveHttpTts", configs: [{ id, name: config.name, enabled: config.enabled, definition: JSON.stringify(config.config) }] };
}

// Mutations name the changed entry; reading or applying a response never writes it back.
function configurationStore<T, R extends { id: string }>(kind: ConfigurationKind, initial: T, entries: (document: T) => R[]) {
  const state = ref(initial) as Ref<T>;
  const items = computed(() => entries(state.value));
  let requestId = 0;
  const requests = new Map<string, number>();
  const pending = new Map<string, { id: number; rule: R | null }>();
  function save(ruleId: string, rule: R | null): void {
    const id = ++requestId;
    requests.set(ruleId, id);
    if (!pending.size) queueMicrotask(() => {
      const changes = [...pending];
      pending.clear();
      for (const [ruleId, { id, rule }] of changes) {
        void modifyDatabase([ruleChange(kind, ruleId, rule)], {
          onComplete: result => {
            if (result.error) { console.error(result.error); return; }
            void updateRule(ruleId, id).catch(console.error);
          },
        }).catch(console.error);
      }
    });
    pending.set(ruleId, { id, rule });
  }

  function apply(ruleId: string, rule: R | null): void {
    const index = items.value.findIndex(item => item.id === ruleId);
    if (rule) {
      if (index < 0) items.value.push(rule);
      else items.value[index] = rule;
    } else if (index >= 0) items.value.splice(index, 1);
  }

  function update(rule: R): void {
    apply(rule.id, rule);
    save(rule.id, rule);
  }

  function remove(ruleId: string): void {
    const index = items.value.findIndex(item => item.id === ruleId);
    if (index < 0) return;
    items.value.splice(index, 1);
    save(ruleId, null);
  }

  async function updateRule(ruleId: string, id: number): Promise<void> {
    if (requests.get(ruleId) !== id) return;
    const document = await readConfiguration<T>(kind, ruleId);
    await nextTick();
    if (requests.get(ruleId) !== id) return;
    const rule = entries(document).find(item => item.id === ruleId);
    apply(ruleId, rule ?? null);
    requests.delete(ruleId);
  }

  async function load(): Promise<void> {
    state.value = await readConfiguration<T>(kind);
    await listen<AppBootstrap>("app-state-updated", async () => {
      const id = ++requestId;
      requests.clear();
      pending.clear();
      const next = await readConfiguration<T>(kind);
      await nextTick();
      if (id !== requestId) return;
      state.value = next;
    });
  }
  return { state, items, update, remove, load };
}

export const replacementRuleStore = configurationStore(
  "replacementRules", { schemaVersion: 1, rules: [] as DisplayReplacementRule[] }, document => document.rules,
);
export const replacementRules = replacementRuleStore.items;
export const txtTocRuleStore = configurationStore(
  "txtTocRules", { schemaVersion: 1, rules: [] } as TxtTocRulesDocument, document => document.rules,
);
export const txtTocRules = txtTocRuleStore.items;
export const httpTtsStore = configurationStore(
  "httpTtsConfigs", { schemaVersion: 1, configs: [] } as HttpTtsConfigResource, document => document.configs,
);
export const httpTtsConfigs = httpTtsStore.items;

export async function loadConfigurations(): Promise<void> {
  await Promise.all([
    replacementRuleStore.load(),
    txtTocRuleStore.load(),
    httpTtsStore.load(),
  ]);
}
