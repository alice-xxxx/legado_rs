import { command } from "./ipc";
import type {
  SourceLoginForm,
  SourceLoginResult,
  SourceWebLoginResult,
} from "./types";

export const loginSource = (sourceId: string, credentials: Record<string, string>) =>
  command<SourceLoginResult>("login_source", { sourceId, credentials });
export const runSourceLoginAction = (sourceId: string, actionId: number, credentials: Record<string, string>) =>
  command<SourceLoginResult>("run_source_login_action", { sourceId, actionId, credentials });
export const clearSourceLoginState = (sourceId: string) => command<{ cleared: boolean }>("clear_source_login_state", { sourceId });
export const startSourceLoginWeb = (sourceId: string) => command<{ sessionId: string }>("start_source_login_web", { sourceId });
export const finishSourceLoginWeb = (sourceId: string, sessionId: string) =>
  command<SourceWebLoginResult>("finish_source_login_web", { sourceId, sessionId });
export const cancelSourceLoginWeb = (sourceId: string, sessionId: string) =>
  command<{ cancelled: boolean }>("cancel_source_login_web", { sourceId, sessionId });

export const getSourceLoginState = (sourceId: string) =>
  command<{ hasLoginState: boolean }>("get_source_login_state", { sourceId });

export const getSourceLoginForm = (sourceId: string) =>
  command<SourceLoginForm>("get_source_login_form", { sourceId });
