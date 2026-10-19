import { command } from "./ipc";

export const cancelMainText = (bookId: string, chapterId: string) =>
  command<void>("cancel_main_text", { bookId, chapterId });

/** 文本始终缓存；cache=true 时也下载完整图片和音视频。 */
export function mainText(bookId: string, chapterId: string, cache = false) {
  let active = true;
  const request = command<void>("main_text", { bookId, chapterId, cache }).finally(() => { active = false; });
  return Object.assign(request, {
    cancel: () => active ? cancelMainText(bookId, chapterId) : Promise.resolve(),
  });
}
