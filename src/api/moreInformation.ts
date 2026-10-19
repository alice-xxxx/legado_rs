import { command } from "./ipc";

/** 各请求独立执行并入库，Vue 等待后自行读取存储。 */
export function moreInformation(sourceId: string, bookUrl: string) {
  let active = true;
  const request = command<void>("more_information", { sourceId, bookUrl }).finally(() => { active = false; });
  // 取消定位到同一源和 URL，不影响其他书籍的详情请求。
  return Object.assign(request, {
    cancel: () => active ? command<void>("cancel_more_information", { sourceId, bookUrl }) : Promise.resolve(),
  });
}
