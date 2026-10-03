import { invoke, isTauri } from "@tauri-apps/api/core";
import { readResource, type ResourceDescriptor } from "./app";

export interface TxtTocRule {
  id: string;
  name: string;
  rule: string;
  example?: string;
  serialNumber: number;
  enable: boolean;
}

export interface TxtTocRulesDocument {
  schemaVersion: number;
  rules: TxtTocRule[];
}

export interface LoadedTxtTocRules {
  resource: ResourceDescriptor;
  document: TxtTocRulesDocument;
}

async function command<T>(name: string, args?: Record<string, unknown>): Promise<T> {
  if (!isTauri()) throw new Error("TXT 目录识别规则只能在应用中管理。");
  return invoke<T>(name, args);
}

/** Fetch the persisted JSON resource through Rust's resource server. */
export async function loadTxtTocRules(source?: ResourceDescriptor): Promise<LoadedTxtTocRules> {
  const resource = source ?? await command<ResourceDescriptor>("get_txt_toc_rules");
  const document = await readResource<TxtTocRulesDocument>(resource);
  return { resource, document };
}

export const upsertTxtTocRule = (rule: TxtTocRule) =>
  command<ResourceDescriptor>("upsert_txt_toc_rule", { rule });

export const deleteTxtTocRule = (ruleId: string) =>
  command<ResourceDescriptor>("delete_txt_toc_rule", { ruleId });
