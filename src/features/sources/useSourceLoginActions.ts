import { onBeforeUnmount, type Ref } from "vue";
import { listen } from "@tauri-apps/api/event";
import { shelfBatchRecoveryRequired } from "../../app/recoveryState";
import { login, type LoginAction, type LoginUpdate } from "../../api/sources";
import { sources } from "./sourcesStore";
import type { SourceLoginField, SourceLoginForm, SourceMetadata } from "../../api/types";
import { confirmAction } from "../../app/confirmation";

interface SourceLoginActionDependencies {
  selectedSourceRows: Ref<string[]>;
  sourceBatchBusy: Ref<boolean>;
  sourceExportBusy: Ref<boolean>;
  sourceLoginTarget: Ref<SourceMetadata | null>;
  sourceLoginBusy: Ref<boolean>;
  sourceHasLoginState: Ref<boolean | null>;
  sourceLoginForm: Ref<SourceLoginForm | null>;
  sourceLoginWebOpen: Ref<boolean>;
  sourceLoginCredentials: Ref<Record<string, string>>;
  sourceLoginFormError: Ref<string>;
  sourceLoginFeedback: Ref<string>;
  notify: (message: string, kind?: "success" | "error") => void;
  errorText: (error: unknown) => string;
}

/** Vue 只维护输入和显示；所有登录动作都调用同一个 Rust 入口。 */
export function useSourceLoginActions({
  selectedSourceRows, sourceBatchBusy, sourceExportBusy, sourceLoginTarget,
  sourceLoginBusy, sourceHasLoginState, sourceLoginForm,
  sourceLoginWebOpen, sourceLoginCredentials, sourceLoginFormError,
  sourceLoginFeedback, notify, errorText,
}: SourceLoginActionDependencies) {
  let generation = 0;
  let listening: ReturnType<typeof listen<LoginUpdate>> | undefined;
  let cancelling = Promise.resolve();

  function makeCredentials(fields: SourceLoginField[], previous: Record<string, string> = {}) {
    return Object.fromEntries(fields.map(field => [field.name,
      Object.prototype.hasOwnProperty.call(previous, field.name) ? previous[field.name]
        : field.initialValue ?? field.choices?.[0] ?? (field.type === "toggle" ? "false" : ""),
    ]));
  }

  async function display(update: LoginUpdate) {
    if (update.sourceId !== sourceLoginTarget.value?.id) return;
    if (update.form) {
      sourceLoginForm.value = update.form;
      sourceLoginCredentials.value = makeCredentials(
        update.form.mode === "form" ? update.form.fields : [], sourceLoginCredentials.value,
      );
    }
    if (update.hasLoginState !== undefined) sourceHasLoginState.value = update.hasLoginState;
    if (update.webOpen !== undefined) sourceLoginWebOpen.value = update.webOpen;
    sourceLoginFeedback.value = update.message ?? "";
    sourceLoginFormError.value = update.error ?? "";
    for (const name of Object.keys(sourceLoginCredentials.value)) {
      const value = update.inputUpdates?.[name];
      if (value !== undefined) sourceLoginCredentials.value[name] = value;
    }
    if (update.copyText !== undefined) {
      try { await navigator.clipboard.writeText(update.copyText); }
      catch (error) { notify(`复制失败：${errorText(error)}`, "error"); }
    }
  }

  async function run(action: LoginAction): Promise<void> {
    const source = sourceLoginTarget.value;
    if (!source || sourceLoginBusy.value || shelfBatchRecoveryRequired.value) return;
    const current = generation;
    const input = action.type === "submit" && action.button !== undefined &&
      sourceLoginForm.value?.mode === "form" &&
      sourceLoginForm.value.layout?.some(row => row.kind === "field" && row.id === action.button);
    if (!input) sourceLoginBusy.value = true;
    sourceLoginFormError.value = "";
    sourceLoginFeedback.value = "";
    try {
      // 先装好监听再调用，避免同步规则的结果先于监听到达。
      await (listening ??= listen<LoginUpdate>("login", event => { void display(event.payload); }));
      await cancelling;
      if (current !== generation) return;
      await login(source.id, action);
    } catch (error) {
      if (current === generation) sourceLoginFormError.value = errorText(error);
    } finally {
      if (!input && current === generation) {
        sourceLoginBusy.value = false;
        if (action.type === "submit" && sourceLoginForm.value?.mode === "web") sourceLoginWebOpen.value = false;
      }
    }
  }

  async function openSelectedSourceLogin(): Promise<void> {
    if (selectedSourceRows.value.length !== 1 || sourceBatchBusy.value || sourceExportBusy.value || shelfBatchRecoveryRequired.value) return;
    const source = sources.value.find(entry => entry.id === selectedSourceRows.value[0]);
    if (!source) return;
    if (source.isRss) { notify("当前登录状态管理适用于普通源。", "error"); return; }
    closeSourceLogin();
    sourceLoginTarget.value = source;
    await run({ type: "open" });
  }

  function executeSelectedSourceLogin(button?: number): Promise<void> {
    return run({ type: "submit", fields: { ...sourceLoginCredentials.value }, button });
  }

  async function clearSelectedSourceLogin(): Promise<void> {
    const source = sourceLoginTarget.value;
    if (!source || sourceLoginBusy.value || shelfBatchRecoveryRequired.value) return;
    if (!await confirmAction("清除登录数据？", `将清除「${source.name}」的登录数据，同域名的其他源也可能需要重新登录。`, "清除登录数据")) return;
    if (sourceLoginTarget.value !== source || sourceLoginBusy.value || shelfBatchRecoveryRequired.value) return;
    sourceLoginCredentials.value = {};
    await run({ type: "clear" });
  }

  function closeSourceLogin(): void {
    const source = sourceLoginTarget.value;
    ++generation;
    sourceLoginTarget.value = null;
    sourceLoginBusy.value = false;
    sourceHasLoginState.value = null;
    sourceLoginWebOpen.value = false;
    sourceLoginCredentials.value = {};
    sourceLoginForm.value = null;
    sourceLoginFormError.value = "";
    sourceLoginFeedback.value = "";
    if (source) cancelling = login(source.id, { type: "cancel" }).catch(error => notify(errorText(error), "error"));
  }

  onBeforeUnmount(() => { void listening?.then(unlisten => unlisten()); });

  return {
    openSelectedSourceLogin,
    runLogin: run,
    executeSelectedSourceLogin,
    clearSelectedSourceLogin,
    closeSourceLogin,
  };
}
