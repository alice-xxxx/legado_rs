import type { ComputedRef, Ref, UnwrapNestedRefs } from "vue";
import type { SourceLoginForm, SourceMetadata } from "../../api/types";
import type { useSourceDebugActions } from "./useSourceDebugActions";
import type { useBookSourceSwitch } from "./useBookSourceSwitch";
import type { useSourceLoginActions } from "./useSourceLoginActions";
import type { useSourceManagementActions } from "./useSourceManagementActions";

export type BookSourceSwitchContract = UnwrapNestedRefs<Pick<
  ReturnType<typeof useBookSourceSwitch>,
  | "bookSourceSwitchOpen"
  | "bookSourceSwitchInline"
  | "bookSourceSwitchBusy"
  | "chapterSourceBusyResultId"
  | "chapterSourceError"
  | "bookSourceSearchBusy"
  | "bookSourceSwitchConfirmOpen"
  | "bookSourceSwitchBook"
  | "bookSourceEligibleSources"
  | "bookSourceChapterTargetId"
  | "bookSourceChapterTargetTitle"
  | "bookSourceSearchId"
  | "bookSourceCandidateResults"
  | "bookSourceCandidateErrors"
  | "selectedBookSourceCandidate"
  | "bookSourceSwitchError"
  | "bookSourceChangeError"
  | "bookSourceSearchStatus"
  | "bookSourceSearchKeyword"
  | "bookSourceCurrentChapter"
  | "openBookSourceSwitch"
  | "openBookSourceSwitchForBook"
  | "closeBookSourceSwitch"
  | "startBookSourceCandidateSearch"
  | "stopBookSourceCandidateSearch"
  | "selectBookSourceCandidate"
  | "chooseBookSourceCandidate"
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
  | "sourceDebugKeywordLoading"
  | "sourceDebugBusy"
  | "openSelectedSourceDebug"
  | "closeSourceDebug"
  | "startSourceDebugSearch"
  | "inspectSourceDebugResult"
  | "previewSourceDebugChapter"
>>;

export type SourceManagementContract = UnwrapNestedRefs<
  Pick<
    ReturnType<typeof useSourceManagementActions>,
    | "changeSourceEnabled"
    | "setSelectedSourcesEnabled"
    | "setSelectedSourcesGroup"
    | "beginSourceEdit"
    | "beginSourceCreate"
    | "sourceEditCreating"
    | "sourceEditorOpen"
    | "closeSourceEdit"
    | "saveSourceDefinitionJson"
    | "sourceEditJson"
    | "sourceEditOriginalJson"
    | "sourceEditBusy"
    | "sourceRemovalBusy"
    | "importSourceFileFromPicker"
    | "importSourceFromUrl"
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
