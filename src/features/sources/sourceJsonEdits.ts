// Edit only the selected JSON token; never reserialize untouched Legado source rules.
// The editor may display parsed values, but raw numeric literals must survive a field edit.
type SourceJsonPath = readonly (string | number)[];

interface JsonNode {
  start: number;
  end: number;
  properties?: JsonProperty[];
  items?: JsonNode[];
}
interface JsonProperty {
  key: string;
  keyStart: number;
  value: JsonNode;
}

function parseSourceDocument(text: string): JsonNode {
  // Validate the whole document before using the lightweight location scanner.
  const parsed: unknown = JSON.parse(text);
  if (!parsed || typeof parsed !== "object" || Array.isArray(parsed))
    throw new Error("书源 JSON 必须是一个对象");
  let index = 0;
  function whitespace(): void {
    while (index < text.length && /[\t\n\r ]/.test(text[index]!)) index++;
  }
  function quoted(): { start: number; end: number; value: string } {
    whitespace();
    const start = index;
    if (text[index++] !== '"') throw new Error("无效的 JSON 字段名");
    let escaped = false;
    while (index < text.length) {
      const ch = text[index++]!;
      if (escaped) { escaped = false; continue; }
      if (ch === "\\") { escaped = true; continue; }
      if (ch === '"') return { start, end: index, value: JSON.parse(text.slice(start, index)) as string };
    }
    throw new Error("JSON 字符串没有结束");
  }
  function node(): JsonNode {
    whitespace();
    const start = index;
    if (text[index] === "{") {
      index++;
      const properties: JsonProperty[] = [];
      whitespace();
      while (text[index] !== "}") {
        const key = quoted();
        whitespace();
        if (text[index++] !== ":") throw new Error("JSON 字段缺少冒号");
        const value = node();
        properties.push({ key: key.value, keyStart: key.start, value });
        whitespace();
        if (text[index] !== ",") break;
        index++;
      }
      if (text[index++] !== "}") throw new Error("JSON 对象没有结束");
      return { start, end: index, properties };
    }
    if (text[index] === "[") {
      index++;
      const items: JsonNode[] = [];
      whitespace();
      while (text[index] !== "]") {
        items.push(node());
        whitespace();
        if (text[index] !== ",") break;
        index++;
      }
      if (text[index++] !== "]") throw new Error("JSON 数组没有结束");
      return { start, end: index, items };
    }
    if (text[index] === '"') {
      const value = quoted();
      return { start: value.start, end: value.end };
    }
    while (index < text.length && !/[\t\n\r ,}\]]/.test(text[index]!)) index++;
    if (index === start) throw new Error("无效的 JSON 值");
    return { start, end: index };
  }
  const root = node();
  whitespace();
  if (index !== text.length) throw new Error("JSON 末尾存在多余内容");
  return root;
}

function at(root: JsonNode, path: SourceJsonPath): JsonNode | undefined {
  let current: JsonNode | undefined = root;
  for (const part of path) {
    if (typeof part === "number") {
      current = current?.items?.[part];
    } else {
      const matches = current?.properties?.filter(property => property.key === part) ?? [];
      if (matches.length > 1) {
        throw new Error(`JSON 中字段“${part}”重复，无法安全确定编辑目标。`);
      }
      current = matches[0]?.value;
    }
    if (!current) return undefined;
  }
  return current;
}
function replaceRange(text: string, start: number, end: number, replacement: string): string {
  const result = text.slice(0, start) + replacement + text.slice(end);
  parseSourceDocument(result);
  return result;
}
function indentation(text: string, start: number): string {
  const line = text.slice(text.lastIndexOf("\n", start - 1) + 1, start);
  return /^[ \t]*$/.test(line) ? line : "";
}
function validatedValue(value: string): void { JSON.parse(value); }

export function sourceJsonValue(text: string, path: SourceJsonPath): string {
  const selected = at(parseSourceDocument(text), path);
  if (!selected) throw new Error("字段已不存在，请重新读取书源");
  return text.slice(selected.start, selected.end);
}

function addSourceJsonValue(text: string, path: SourceJsonPath, key: string | null, raw: string): string {
  validatedValue(raw);
  const parent = at(parseSourceDocument(text), path);
  if (!parent) throw new Error("目标字段已不存在");
  const object = parent.properties;
  const array = parent.items;
  if (!object && !array) throw new Error("只能在对象或数组中添加字段");
  if (object) {
    if (key === null || !key) throw new Error("请输入字段名");
    if (object.some(property => property.key === key)) throw new Error("字段已经存在");
  }
  const entry = object ? JSON.stringify(key) + ": " + raw : raw;
  const children = object ? object.map(property => property.value) : array!;
  const last = children[children.length - 1];
  if (last) {
    const multiline = text.slice(parent.start, parent.end).includes("\n");
    const separator = multiline ? ",\n" + indentation(text, last.start) : ", ";
    return replaceRange(text, last.end, last.end, separator + entry);
  }
  const closeIndent = indentation(text, parent.end - 1);
  const multiline = text.slice(parent.start, parent.end).includes("\n");
  const content = multiline ? "\n" + closeIndent + "  " + entry + "\n" + closeIndent : entry;
  return replaceRange(text, parent.start + 1, parent.end - 1, content);
}

/** Serialize a structured form field without silently changing an existing Legado
 * JSON value from object to array, bool to string, or number to string.
 * Text fields remain strings even when they happen to contain JSON syntax.
 */
export function sourceFormFieldJsonValue(key: string, label: string, original: unknown, draft: string): string {
  if (key === "legadoRsUserAgentOverride") {
    if (new TextEncoder().encode(draft).length > 512 || /[\u0000-\u001f\u007f]/.test(draft)) {
      throw new Error("User-Agent 必须不超过 512 字节且不能包含控制字符");
    }
  }
  if (key === "bookSourceType") {
    if (!/^-?\d+$/.test(draft) || !Number.isSafeInteger(Number(draft))) {
      throw new Error("内容类型必须填写安全范围内的整数");
    }
    return draft;
  }
  if (original !== null && typeof original === "object") {
    let parsed: unknown;
    try { parsed = JSON.parse(draft); }
    catch { throw new Error(label + " 必须填写有效的 JSON 对象或数组"); }
    if (!parsed || typeof parsed !== "object" || Array.isArray(parsed) !== Array.isArray(original)) {
      throw new Error(label + (Array.isArray(original) ? " 必须保留 JSON 数组类型" : " 必须保留 JSON 对象类型"));
    }
    return draft;
  }
  if (typeof original === "boolean") {
    if (draft !== "true" && draft !== "false") throw new Error(label + " 必须填写 true 或 false");
    return draft;
  }
  if (typeof original === "number") {
    let parsed: unknown;
    try { parsed = JSON.parse(draft); }
    catch { throw new Error(label + " 必须填写有效的 JSON 数字"); }
    if (typeof parsed !== "number" || !Number.isFinite(parsed)) {
      throw new Error(label + " 必须保留 JSON 数字类型");
    }
    return draft;
  }
  return JSON.stringify(draft);
}

export function setSourceJsonValue(text: string, path: SourceJsonPath, raw: string): string {
  validatedValue(raw);
  if (!path.length) throw new Error("不能替换整个书源根对象");
  const root = parseSourceDocument(text);
  const existing = at(root, path);
  if (existing) return replaceRange(text, existing.start, existing.end, raw);
  const key = path[path.length - 1]!;
  if (typeof key !== "string") throw new Error("数组下标无效");
  return addSourceJsonValue(text, path.slice(0, -1), key, raw);
}

export function removeSourceJsonValue(text: string, path: SourceJsonPath): string {
  if (!path.length) throw new Error("Cannot remove the source root object");
  const root = parseSourceDocument(text);
  const key = path[path.length - 1];
  if (typeof key !== "string") throw new Error("Only object properties can be removed");
  const parent = at(root, path.slice(0, -1));
  const properties = parent?.properties;
  if (!properties) throw new Error("The target parent must be an object");
  const matches = properties
    .map((property, index) => ({ property, index }))
    .filter(({ property }) => property.key === key);
  if (matches.length > 1) throw new Error(`Duplicate JSON property '${key}' cannot be removed safely`);
  const match = matches[0];
  if (!match) return text;
  const { property, index } = match;
  let start = property.keyStart;
  let end = property.value.end;
  if (index < properties.length - 1) {
    end = properties[index + 1]!.keyStart;
  } else if (index > 0) {
    start = properties[index - 1]!.value.end;
  }
  return replaceRange(text, start, end, "");
}