// Keep the label mapping aligned with KMP's BookSourceType constants.
// Imported values are never coerced or normalized: legacy/custom values remain editable
// through the full JSON editor even when the form has no matching known type.
export const knownBookSourceTypes = [
  { value: "0", label: "小说 / 文本" },
  { value: "1", label: "音频" },
  { value: "2", label: "图片 / 漫画" },
  { value: "3", label: "下载" },
  { value: "4", label: "视频" },
  { value: "5", label: "订阅" },
] as const;

export function bookSourceTypeOptions(raw: string): Array<{ value: string; label: string }> {
  const result: Array<{ value: string; label: string }> = [...knownBookSourceTypes];
  if (!result.some(option => option.value === raw)) {
    result.push({ value: raw, label: raw.trim() ? `其他类型（原值 ${raw}）` : "未设置（原值为空）" });
  }
  return result;
}
