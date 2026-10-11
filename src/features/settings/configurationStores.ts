import type { TxtTocRulesDocument } from "../../api/settings";
import { computed, nextTick, ref, watch, type Ref, type WatchStopHandle } from "vue";
import { listen } from "@tauri-apps/api/event";
import { command } from "../../api/ipc";
import { readResource } from "../../api/resources";
import type { AppBootstrap, DisplayReplacementRule, HttpTtsConfigResource, ResourceDescriptor } from "../../api/types";

type ConfigurationKind = "replacementRules" | "txtTocRules" | "httpTtsConfigs";
type RuleUpdatedEvent = { kind: ConfigurationKind; ruleId: string; requestId: number; resource: ResourceDescriptor };

// One shared reactive document per file. Only changed complete entries are sent.
function configurationStore<T, R extends { id: string }>(kind: ConfigurationKind, initial: T, entries: (document: T) => R[]) {
  const state = ref(initial) as Ref<T>;
  const items = computed(() => entries(state.value));
  let requestId = 0;
  const requests = new Map<string, number>();
  let stopWatch: WatchStopHandle = () => {};

  function startWatch(): void {
    let previous = new Map(items.value.map(rule => [rule.id, JSON.stringify(rule)]));
    stopWatch = watch(state, () => {
      const current = new Map(items.value.map(rule => [rule.id, JSON.stringify(rule)]));
      function save(ruleId: string, rule: R | null): void {
        const id = ++requestId;
        requests.set(ruleId, id);
        void command("save_configuration_rule", { kind, ruleId, rule, requestId: id }).catch(console.error);
      }
      for (const rule of items.value) {
        if (current.get(rule.id) !== previous.get(rule.id)) save(rule.id, rule);
      }
      for (const ruleId of previous.keys()) {
        if (!current.has(ruleId)) save(ruleId, null);
      }
      previous = current;
    }, { deep: true });
  }

  function update(rule: R): void {
    const index = items.value.findIndex(item => item.id === rule.id);
    if (index < 0) items.value.push(rule);
    else items.value[index] = rule;
  }

  function remove(ruleId: string): void {
    const index = items.value.findIndex(item => item.id === ruleId);
    if (index >= 0) items.value.splice(index, 1);
  }

  async function updateRule(event: RuleUpdatedEvent): Promise<void> {
    if (requests.get(event.ruleId) !== event.requestId) return;
    const document = await readResource<T>(event.resource);
    await nextTick();
    if (requests.get(event.ruleId) !== event.requestId) return;
    const rule = entries(document).find(item => item.id === event.ruleId);
    stopWatch();
    if (rule) update(rule);
    else remove(event.ruleId);
    requests.delete(event.ruleId);
    startWatch();
  }

  async function load(descriptor: ResourceDescriptor): Promise<void> {
    state.value = await readResource<T>(descriptor);
    await listen<RuleUpdatedEvent>("resource-updated", ({ payload }) => {
      if (payload.kind === kind) void updateRule(payload).catch(console.error);
    });
    await listen<AppBootstrap>("app-state-updated", async ({ payload }) => {
      const id = ++requestId;
      requests.clear();
      const next = await readResource<T>(payload[kind]);
      await nextTick();
      if (id !== requestId) return;
      stopWatch();
      state.value = next;
      startWatch();
    });
    startWatch();
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

export async function loadConfigurations(bootstrap: AppBootstrap): Promise<void> {
  await Promise.all([
    replacementRuleStore.load(bootstrap.replacementRules),
    txtTocRuleStore.load(bootstrap.txtTocRules),
    httpTtsStore.load(bootstrap.httpTtsConfigs),
  ]);
}
