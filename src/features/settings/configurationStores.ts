import type { TxtTocRulesDocument } from "../../api/settings";
import { computed, nextTick, ref, type Ref } from "vue";
import { listen } from "@tauri-apps/api/event";
import { command } from "../../api/ipc";
import { readConfiguration } from "../../api/database";
import type { AppBootstrap, DisplayReplacementRule, HttpTtsConfigResource } from "../../api/types";

type ConfigurationKind = "replacementRules" | "txtTocRules" | "httpTtsConfigs";
type RuleUpdatedEvent = { kind: ConfigurationKind; ruleId: string; requestId: number };

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
        void command("save_configuration_rule", { kind, ruleId, rule, requestId: id }).catch(console.error);
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

  async function updateRule(event: RuleUpdatedEvent): Promise<void> {
    if (requests.get(event.ruleId) !== event.requestId) return;
    const document = await readConfiguration<T>(kind, event.ruleId);
    await nextTick();
    if (requests.get(event.ruleId) !== event.requestId) return;
    const rule = entries(document).find(item => item.id === event.ruleId);
    apply(event.ruleId, rule ?? null);
    requests.delete(event.ruleId);
  }

  async function load(): Promise<void> {
    state.value = await readConfiguration<T>(kind);
    await listen<RuleUpdatedEvent>("resource-updated", ({ payload }) => {
      if (payload.kind === kind) void updateRule(payload).catch(console.error);
    });
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
