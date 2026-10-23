import type { SearchBookResult, SearchResultSource } from "../../api/types";

/** 信息完整度占 70%，响应速度占 30%；1 秒响应取得一半速度分。 */
function sourceScore(source: SearchResultSource): number {
  const completeness = [source.author, source.coverSrc, source.intro, source.latestChapter]
    .filter(value => value?.trim()).length / 4;
  const speed = source.responseTimeMs == null ? 0 : 1000 / (1000 + source.responseTimeMs);
  return completeness * 0.7 + speed * 0.3;
}

export const compareSearchSources = (left: SearchResultSource, right: SearchResultSource): number =>
  sourceScore(right) - sourceScore(left);

/** Rust 只过滤结果；书目归并、可换来源去重和展示顺序都由 Vue 决定。 */
export function presentSearchResults(results: readonly SearchResultSource[], keyword: string): SearchBookResult[] {
  const normalize = (value = "") => value.trim().toLowerCase();
  const query = normalize(keyword);
  const groups = new Map<string, SearchBookResult>();
  for (const result of results) {
    const key = JSON.stringify([result.contentType ?? "text", normalize(result.title), normalize(result.author)]);
    const group = groups.get(key);
    if (!group) groups.set(key, { ...result, sources: [result] });
    else if (!group.sources!.some(source => source.sourceId === result.sourceId)) group.sources!.push(result);
  }
  const rank = (result: SearchBookResult) => {
    const title = normalize(result.title), author = normalize(result.author);
    return title === query || author === query ? 0 : title.includes(query) || author.includes(query) ? 1 : 2;
  };
  return [...groups.values()].map(group => {
    const sources = group.sources!.sort(compareSearchSources);
    return { ...sources[0]!, sources };
  }).sort((left, right) =>
    rank(left) - rank(right) || (rank(left) < 2 ? right.sources!.length - left.sources!.length : 0));
}
