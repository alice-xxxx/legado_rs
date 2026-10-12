import { command } from "./ipc";
import type {
  SourceLoginForm,
  SourceMutation,
  SourceMutationResponse,
} from "./types";

export const mutateSource = (mutation: SourceMutation, requestId = 0) =>
  command<SourceMutationResponse>("mutate_source", { mutation, requestId });

export const getSourceLoginState = (sourceId: string) =>
  command<{ hasLoginState: boolean }>("get_source_login_state", { sourceId });

export const getSourceLoginForm = (sourceId: string) =>
  command<SourceLoginForm>("get_source_login_form", { sourceId });
