import { setSourceJsonValue, sourceJsonValue, sourceFormFieldJsonValue } from "./sourceJsonEdits";
import { bookSourceEditorTabs, rssSourceEditorTabs, type SourceEditorField } from "./sourceEditorFields";

const has = (object: object, key: string) => Object.prototype.hasOwnProperty.call(object, key);
export function sourceEditorTabs(json: string) {
  const root = JSON.parse(json) as Record<string, unknown>;
  return !has(root, "bookSourceUrl") && has(root, "sourceUrl") ? rssSourceEditorTabs : bookSourceEditorTabs;
}

function readField(json: string, key: string): { value: unknown; raw: string } | undefined {
  const parts = key.split(".");
  let current = json;
  for (let i = 0; i < parts.length; i++) {
    const object = JSON.parse(current) as Record<string, unknown>;
    if (!object || typeof object !== "object" || Array.isArray(object) || !has(object, parts[i]!)) return;
    const raw = sourceJsonValue(current, [parts[i]!]);
    const value: unknown = JSON.parse(raw);
    if (i === parts.length - 1) return { value, raw };
    current = typeof value === "string" ? value : raw;
    try { JSON.parse(current); } catch { return; }
  }
}
export function sourceEditorFieldKey(json: string, field: SourceEditorField): string {
  return [field.key, ...(field.aliases ?? [])].find(key => readField(json, key)) ?? field.key;
}
export function readSourceEditorDraft(json: string, fields: SourceEditorField[], enabled: boolean): Record<string, string> {
  const draft: Record<string, string> = {};
  for (const field of fields) {
    const found = readField(json, sourceEditorFieldKey(json, field));
    draft[field.key] = found && found.value !== null
      ? typeof found.value === "string" ? found.value : found.raw
      : field.key === "enabled" ? String(enabled) : field.defaultValue ?? "";
  }
  return draft;
}
function writeField(json: string, key: string, raw: string): string {
  const [parent, child] = key.split(".");
  if (!child) return setSourceJsonValue(json, [parent!], raw);
  const container = readField(json, parent!);
  let ruleJson = container?.value != null ? typeof container.value === "string" ? container.value : container.raw : "{}";
  const rule: unknown = JSON.parse(ruleJson);
  if (!rule || typeof rule !== "object" || Array.isArray(rule)) throw new Error("规则必须是一个对象");
  ruleJson = setSourceJsonValue(ruleJson, [child], raw);
  return setSourceJsonValue(json, [parent!], typeof container?.value === "string" ? JSON.stringify(ruleJson) : ruleJson);
}

/** Build the definition at Save. Unedited missing fields stay absent. */
export function serializeSourceEditorDraft(
  json: string, fields: SourceEditorField[], baseline: Record<string, string>, draft: Record<string, string>,
): string {
  let result = json;
  for (const field of fields) {
    const text = draft[field.key] ?? "";
    if (text === baseline[field.key]) continue;
    const key = sourceEditorFieldKey(json, field);
    const original = readField(json, key)?.value;
    let raw: string;
    if (field.kind === "toggle") {
      if (text !== "true" && text !== "false") throw new Error(`${field.label}必须是开关值`);
      raw = text;
    } else if (field.kind === "number" || field.kind === "type") {
      if (!/^-?(?:0|[1-9]\d*)$/.test(text)) throw new Error(`${field.label}必须填写整数`);
      const limit = ["lastUpdateTime", "respondTime"].includes(field.key) ? 2n ** 63n : 2n ** 31n;
      if (BigInt(text) < -limit || BigInt(text) >= limit) throw new Error(`${field.label}超出支持的整数范围`);
      raw = text;
    } else raw = sourceFormFieldJsonValue(key.split(".").slice(-1)[0]!, field.label, original, text);
    try { result = writeField(result, key, raw); }
    catch (cause) { throw new Error(`${field.label}：${cause instanceof Error ? cause.message : String(cause)}`); }
    if (field.key === "enabled") {
      result = setSourceJsonValue(result, ["enabled"], raw);
      if (readField(json, "bookSourceEnabled")) result = setSourceJsonValue(result, ["bookSourceEnabled"], raw);
    }
  }
  return result;
}
