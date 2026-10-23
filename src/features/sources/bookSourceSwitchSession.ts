import type { BookSourceSwitchContract } from "./sourceContext";

/** Source-switch state shared by the detail and reader screens. */
export type BookSourceSwitchSession = Pick<
  BookSourceSwitchContract,
  | "bookSourceSwitchOpen"
  | "bookSourceSwitchInline"
  | "bookSourceSwitchBusy"
  | "bookSourceSwitchCancellable"
  | "bookSourceCheckBusy"
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
  | "checkBookSourceUpdates"
  | "replaceCurrentChapterFromCandidate"
  | "confirmBookSourceChange"
>;

let activeSession: BookSourceSwitchSession | null = null;

/** Registers the source feature surface consumed by the book detail and reader screens. */
export function installBookSourceSwitchSession(session: BookSourceSwitchSession): () => void {
  activeSession = session;
  return () => {
    if (activeSession === session) activeSession = null;
  };
}

/** Returns only the source-switch operations and state used by those screens. */
export function getBookSourceSwitchSession(): BookSourceSwitchSession {
  if (!activeSession) throw new Error("Book source-switch session is not mounted");
  return activeSession;
}
