import { type Ref } from "vue";
import { getSourceLoginForm, getSourceLoginState, mutateSource } from "../../api/sources";
import { type SourceLoginField, type SourceLoginForm, type SourceMetadata } from "../../api/types";

interface SourceLoginActionDependencies {
  sources: Ref<SourceMetadata[]>;
  selectedSourceRows: Ref<string[]>;
  sourceBatchBusy: Ref<boolean>;
  sourceExportBusy: Ref<boolean>;
  shelfBatchRecoveryRequired: Readonly<Ref<boolean>>;
  sourceLoginTarget: Ref<SourceMetadata | null>;
  sourceLoginBusy: Ref<boolean>;
  sourceHasLoginState: Ref<boolean | null>;
  sourceLoginError: Ref<string>;
  sourceLoginForm: Ref<SourceLoginForm | null>;
  sourceLoginWebSessionId: Ref<string>;
  sourceLoginCredentials: Ref<Record<string, string>>;
  sourceLoginFormError: Ref<string>;
  sourceLoginFeedback: Ref<string>;
  notify: (message: string, kind?: "success" | "error") => void;
  errorText: (error: unknown) => string;
}

/** 管理选中书源的登录表单、Cookie 状态和私有网页登录会话。 */
export function useSourceLoginActions({
  sources,
  selectedSourceRows,
  sourceBatchBusy,
  sourceExportBusy,
  shelfBatchRecoveryRequired,
  sourceLoginTarget,
  sourceLoginBusy,
  sourceHasLoginState,
  sourceLoginError,
  sourceLoginForm,
  sourceLoginWebSessionId,
  sourceLoginCredentials,
  sourceLoginFormError,
  sourceLoginFeedback,
  notify,
  errorText,
}: SourceLoginActionDependencies) {
  // 每次切换或关闭登录对象都会递增代次，丢弃旧请求的迟到结果。
  let sourceLoginGeneration = 0;

  function isCurrentSourceLogin(sourceId: string, generation: number): boolean {
    return sourceLoginGeneration === generation && sourceLoginTarget.value?.id === sourceId;
  }

  function makeSourceLoginCredentials(
    fields: SourceLoginField[],
    previous: Record<string, string> = Object.create(null) as Record<string, string>,
  ): Record<string, string> {
    const credentials = Object.create(null) as Record<string, string>;
    for (const field of fields) {
      const hasPrevious = Object.prototype.hasOwnProperty.call(previous, field.name);
      credentials[field.name] = hasPrevious
        ? previous[field.name]
        : field.type === "toggle" ? "false" : field.choices?.[0] ?? "";
    }
    return credentials;
  }

  async function openSelectedSourceLogin(): Promise<void> {
    if (selectedSourceRows.value.length !== 1 || sourceBatchBusy.value || sourceExportBusy.value || shelfBatchRecoveryRequired.value) return;
    const source = sources.value.find((entry) => entry.id === selectedSourceRows.value[0]);
    if (!source) return;
    if (source.isRss) {
      notify("当前登录状态管理适用于普通书源。", "error");
      return;
    }

    const generation = ++sourceLoginGeneration;
    sourceLoginBusy.value = false;
    sourceLoginTarget.value = source;
    sourceHasLoginState.value = null;
    sourceLoginError.value = "";
    sourceLoginForm.value = null;
    sourceLoginWebSessionId.value = "";
    sourceLoginCredentials.value = Object.create(null) as Record<string, string>;
    sourceLoginFormError.value = "";
    sourceLoginFeedback.value = "";

    await refreshSelectedSourceLogin();
    if (!isCurrentSourceLogin(source.id, generation)) return;
    await loadSelectedSourceLoginForm();
  }

  async function loadSelectedSourceLoginForm(): Promise<void> {
    const source = sourceLoginTarget.value;
    if (!source || sourceLoginBusy.value || shelfBatchRecoveryRequired.value) return;
    const generation = sourceLoginGeneration;
    sourceLoginBusy.value = true;
    sourceLoginFormError.value = "";
    try {
      const form = await getSourceLoginForm(source.id);
      if (!isCurrentSourceLogin(source.id, generation)) return;
      sourceLoginForm.value = form;
      sourceLoginWebSessionId.value = "";
      sourceLoginCredentials.value = makeSourceLoginCredentials(
        form.mode === "form" ? form.fields : [],
        sourceLoginCredentials.value,
      );
    } catch (error) {
      if (!isCurrentSourceLogin(source.id, generation)) return;
      sourceLoginForm.value = null;
      sourceLoginFormError.value = errorText(error);
    } finally {
      if (isCurrentSourceLogin(source.id, generation)) sourceLoginBusy.value = false;
    }
  }

  async function executeSelectedSourceLogin(actionId?: number): Promise<void> {
    const source = sourceLoginTarget.value;
    const form = sourceLoginForm.value;
    if (!source || !form || form.mode !== "form" || sourceLoginBusy.value || shelfBatchRecoveryRequired.value) return;
    const generation = sourceLoginGeneration;
    sourceLoginBusy.value = true;
    sourceLoginFeedback.value = "";
    sourceLoginFormError.value = "";
    try {
      const credentials = Object.assign(Object.create(null), sourceLoginCredentials.value) as Record<string, string>;
      const result = actionId === undefined
        ? await mutateSource({ kind: "login", sourceId: source.id, credentials })
        : await mutateSource({ kind: "runLoginAction", sourceId: source.id, actionId, credentials });
      if (!isCurrentSourceLogin(source.id, generation)) return;
      if (!("status" in result)) throw new Error("Source login returned an invalid response");
      if (result.status === "failed") sourceLoginFormError.value = result.message;
      else sourceLoginFeedback.value = result.message;
      if (result.status === "executed") {
        for (const field of form.fields) {
          if (field.password || field.type === "password") sourceLoginCredentials.value[field.name] = "";
        }
      }
      const state = await getSourceLoginState(source.id);
      if (isCurrentSourceLogin(source.id, generation)) sourceHasLoginState.value = state.hasLoginState;
    } catch {
      if (isCurrentSourceLogin(source.id, generation)) {
        sourceLoginFormError.value = "登录操作或状态读取失败，请检查登录信息后重试。";
        sourceHasLoginState.value = null;
      }
    } finally {
      if (isCurrentSourceLogin(source.id, generation)) sourceLoginBusy.value = false;
    }
  }

  async function refreshSelectedSourceLogin(): Promise<void> {
    const source = sourceLoginTarget.value;
    if (!source || sourceLoginBusy.value || shelfBatchRecoveryRequired.value) return;
    const generation = sourceLoginGeneration;
    sourceLoginBusy.value = true;
    sourceLoginError.value = "";
    try {
      const result = await getSourceLoginState(source.id);
      if (isCurrentSourceLogin(source.id, generation)) sourceHasLoginState.value = result.hasLoginState;
    } catch (error) {
      if (isCurrentSourceLogin(source.id, generation)) {
        sourceHasLoginState.value = null;
        sourceLoginError.value = `读取登录状态失败：${errorText(error)}`;
      }
    } finally {
      if (isCurrentSourceLogin(source.id, generation)) sourceLoginBusy.value = false;
    }
  }

  async function clearSelectedSourceLogin(): Promise<void> {
    const source = sourceLoginTarget.value;
    if (!source || sourceLoginBusy.value || sourceHasLoginState.value === null || shelfBatchRecoveryRequired.value) return;
    const generation = sourceLoginGeneration;
    if (!window.confirm(`清除「${source.name}」的登录信息及该域名共享的 Cookie？使用同一域名的其他书源也可能需要重新登录。书源规则和书籍会保留。`)) return;
    if (!isCurrentSourceLogin(source.id, generation) || sourceLoginBusy.value || shelfBatchRecoveryRequired.value) return;
    sourceLoginBusy.value = true;
    sourceLoginError.value = "";
    try {
      const result = await mutateSource({
        kind: "clearLoginState",
        sourceId: source.id,
      });
      if (!("cleared" in result)) throw new Error("Source login state returned an invalid response");
      if (!result.cleared) throw new Error("没有收到登录状态清除完成结果。");
      if (!isCurrentSourceLogin(source.id, generation)) return;
      sourceLoginCredentials.value = makeSourceLoginCredentials(
        sourceLoginForm.value?.mode === "form" ? sourceLoginForm.value.fields : [],
      );
      sourceLoginFeedback.value = "";
      sourceLoginFormError.value = "";
      const state = await getSourceLoginState(source.id);
      if (!isCurrentSourceLogin(source.id, generation)) return;
      sourceHasLoginState.value = state.hasLoginState;
      notify("已清除登录状态。");
    } catch (error) {
      if (isCurrentSourceLogin(source.id, generation)) {
        sourceHasLoginState.value = null;
        sourceLoginError.value = `清除或重新读取登录状态失败：${errorText(error)}。请重新读取状态后再操作。`;
      }
    } finally {
      if (isCurrentSourceLogin(source.id, generation)) sourceLoginBusy.value = false;
    }
  }

  async function startSelectedSourceWebLogin(): Promise<void> {
    const source = sourceLoginTarget.value;
    if (!source || sourceLoginForm.value?.mode !== "web" || sourceLoginBusy.value || shelfBatchRecoveryRequired.value) return;
    const generation = sourceLoginGeneration;
    sourceLoginBusy.value = true;
    sourceLoginFeedback.value = "";
    sourceLoginFormError.value = "";
    try {
      const result = await mutateSource({
        kind: "startLoginWeb",
        sourceId: source.id,
      });
      if (!("sessionId" in result)) throw new Error("Source web login returned no session ID");
      if (!isCurrentSourceLogin(source.id, generation)) {
        // 界面已切换时，关闭迟到创建的浏览器会话，避免留下孤立登录窗口。
        void mutateSource({
          kind: "cancelLoginWeb",
          sourceId: source.id,
          sessionId: result.sessionId,
        }).catch(() => undefined);
        return;
      }
      sourceLoginWebSessionId.value = result.sessionId;
      sourceLoginFeedback.value = "网页登录窗口已打开。完成网页登录后关闭私有浏览器，回此处点“完成并保存 Cookie”。应用只保存 Cookie，不会验证站点是否已认证。";
    } catch (error) {
      if (isCurrentSourceLogin(source.id, generation)) sourceLoginFormError.value = errorText(error);
    } finally {
      if (isCurrentSourceLogin(source.id, generation)) sourceLoginBusy.value = false;
    }
  }

  async function finishSelectedSourceWebLogin(): Promise<void> {
    const source = sourceLoginTarget.value;
    const sessionId = sourceLoginWebSessionId.value;
    if (!source || !sessionId || sourceLoginBusy.value || shelfBatchRecoveryRequired.value) return;
    const generation = sourceLoginGeneration;
    sourceLoginBusy.value = true;
    sourceLoginFormError.value = "";
    try {
      const result = await mutateSource({
        kind: "finishLoginWeb",
        sourceId: source.id,
        sessionId,
      });
      if (!("importedCount" in result)) throw new Error("Source web login returned an invalid response");
      if (!isCurrentSourceLogin(source.id, generation)) return;
      sourceLoginWebSessionId.value = "";
      sourceLoginFeedback.value = result.message;
      const state = await getSourceLoginState(source.id);
      if (isCurrentSourceLogin(source.id, generation)) sourceHasLoginState.value = state.hasLoginState;
    } catch (error) {
      if (isCurrentSourceLogin(source.id, generation)) {
        sourceLoginWebSessionId.value = "";
        sourceLoginFormError.value = errorText(error);
      }
    } finally {
      if (isCurrentSourceLogin(source.id, generation)) sourceLoginBusy.value = false;
    }
  }

  function cancelSelectedSourceWebLogin(): void {
    const source = sourceLoginTarget.value;
    const sessionId = sourceLoginWebSessionId.value;
    sourceLoginWebSessionId.value = "";
    if (source && sessionId) {
      void mutateSource({
        kind: "cancelLoginWeb",
        sourceId: source.id,
        sessionId,
      }).catch(() => undefined);
    }
  }

  function closeSourceLogin(): void {
    if (sourceLoginBusy.value && sourceLoginWebSessionId.value) return;
    cancelSelectedSourceWebLogin();
    sourceLoginGeneration += 1;
    sourceLoginTarget.value = null;
    sourceLoginBusy.value = false;
    sourceHasLoginState.value = null;
    sourceLoginError.value = "";
    sourceLoginCredentials.value = Object.create(null) as Record<string, string>;
    sourceLoginForm.value = null;
    sourceLoginFormError.value = "";
    sourceLoginFeedback.value = "";
  }

  return {
    openSelectedSourceLogin,
    loadSelectedSourceLoginForm,
    executeSelectedSourceLogin,
    refreshSelectedSourceLogin,
    clearSelectedSourceLogin,
    startSelectedSourceWebLogin,
    finishSelectedSourceWebLogin,
    cancelSelectedSourceWebLogin,
    closeSourceLogin,
  };
}
