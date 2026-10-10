import type { ComputedRef, Ref, UnwrapNestedRefs } from "vue";
import type { SourceLoginForm, SourceMetadata } from "../../api/types";
import type { useSourceDebugActions } from "./useSourceDebugActions";
import type { useBookSourceSwitch } from "./useBookSourceSwitch";
import type { useSourceLoginActions } from "./useSourceLoginActions";
import type { useSourceManagementActions } from "./useSourceManagementActions";

export type BookSourceSwitchContract = UnwrapNestedRefs<Pick<
  ReturnType<typeof useBookSourceSwitch>,
  | "bookSourceSwitchOpen"
  | "bookSourceSwitchBusy"
  | "chapterSourceBusyResultId"
  | "chapterSourceIdentityConfirmResultId"
  | "chapterSourceError"
  | "bookSourceSearchBusy"
  | "bookSourceSwitchConfirmOpen"
  | "bookSourceSwitchBook"
  | "bookSourceChapterTargetId"
  | "bookSourceChapterTargetTitle"
  | "bookSourceCandidateTaskId"
  | "bookSourceCandidateResults"
  | "bookSourceCandidateErrors"
  | "selectedBookSourceCandidate"
  | "bookSourceSwitchError"
  | "bookSourceChangeError"
  | "bookSourceSearchStatus"
  | "bookSourceSearchKeyword"
  | "bookSourceCurrentChapter"
  | "bookSourceNeedsIdentityConfirmation"
  | "openBookSourceSwitch"
  | "openBookSourceSwitchForBook"
  | "closeBookSourceSwitch"
  | "startBookSourceCandidateSearch"
  | "selectBookSourceCandidate"
  | "replaceCurrentChapterFromCandidate"
  | "confirmBookSourceChange"
>>;

export type SourceDebugContract = UnwrapNestedRefs<Pick<
  ReturnType<typeof useSourceDebugActions>,
  | "sourceDebugTarget"
  | "sourceDebugResults"
  | "sourceDebugSearched"
  | "sourceDebugErrors"
  | "sourceDebugSelectedResult"
  | "sourceDebugBook"
  | "sourceDebugChapters"
  | "sourceDebugChapterResource"
  | "sourceDebugChapterText"
  | "sourceDebugSelectedChapterIndex"
  | "sourceDebugStage"
  | "sourceDebugError"
  | "sourceDebugKeyword"
  | "sourceDebugBusy"
  | "openSelectedSourceDebug"
  | "selectSourceDebugSource"
  | "closeSourceDebug"
  | "startSourceDebugSearch"
  | "inspectSourceDebugResult"
  | "previewSourceDebugChapter"
>>;

export type SourceManagementContract = UnwrapNestedRefs<
  Pick<
    ReturnType<typeof useSourceManagementActions>,
    | "changeSourceEnabled"
    | "refreshSourceList"
    | "setSelectedSourcesEnabled"
    | "setSelectedSourcesGroup"
    | "beginSourceEdit"
    | "closeSourceEdit"
    | "saveSourceDefinitionJson"
    | "sourceEditJson"
    | "sourceEditOriginalJson"
    | "sourceEditInitialUrl"
    | "sourceEditBusy"
    | "sourceRemovalBusy"
    | "importSourceFileFromPicker"
    | "importSourceFromUrl"
    | "createSourceFromJson"
    | "exportSelectedSources"
    | "deleteSelectedSources"
  >
  & {
    sources: Ref<SourceMetadata[]>;
    enabledSources: ComputedRef<SourceMetadata[]>;
    bookSourceCandidates: ComputedRef<SourceMetadata[]>;
    selectedSourceRows: Ref<string[]>;
    sourceBatchBusy: Ref<boolean>;
    sourceExportBusy: Ref<boolean>;
    sourceImportOpen: Ref<boolean>;
    sourceImportBusy: Ref<boolean>;
    sourceImportUrl: Ref<string>;
    pendingSourceRemoval: Ref<string[]>;
    editingSource: Ref<SourceMetadata | null>;
    sourceName: (id: string) => string;
    toggleSourceRow: (id: string) => void;
  }
>;

export type SourceLoginContract = UnwrapNestedRefs<
  Pick<
    ReturnType<typeof useSourceLoginActions>,
    | "openSelectedSourceLogin"
    | "loadSelectedSourceLoginForm"
    | "executeSelectedSourceLogin"
    | "refreshSelectedSourceLogin"
    | "clearSelectedSourceLogin"
    | "startSelectedSourceWebLogin"
    | "finishSelectedSourceWebLogin"
    | "cancelSelectedSourceWebLogin"
    | "closeSourceLogin"
  >
  & {
    sourceLoginTarget: Ref<SourceMetadata | null>;
    sourceLoginBusy: Ref<boolean>;
    sourceHasLoginState: Ref<boolean | null>;
    sourceLoginError: Ref<string>;
    sourceLoginForm: Ref<SourceLoginForm | null>;
    sourceLoginWebSessionId: Ref<string>;
    sourceLoginCredentials: Ref<Record<string, string>>;
    sourceLoginFormError: Ref<string>;
    sourceLoginFeedback: Ref<string>;
  }
>;
