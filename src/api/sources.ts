import { command } from "./ipc";
import type { SourceLoginForm } from "./types";

export type LoginAction =
  | { type: "open" | "cancel" | "clear" }
  | { type: "submit"; fields?: Record<string, string>; button?: number };

/** 只包含当前界面需要更新的内容；Cookie、规则和窗口会话不传入 Vue。 */
export interface LoginUpdate {
  sourceId: string;
  form?: SourceLoginForm;
  hasLoginState?: boolean;
  webOpen?: boolean;
  message?: string;
  error?: string;
  inputUpdates?: Record<string, string>;
  refreshTargets?: string[];
  copyText?: string;
}

export const login = (sourceId: string, action: LoginAction) =>
  command<void>("login", { sourceId, action });
