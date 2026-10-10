import type { BookSourceSwitchContract } from "./sourceContext";

/** Source-switch state consumed by the detail screen and its confirmation sheet. */
export type BookSourceSwitchSession = Pick<
  BookSourceSwitchContract,
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
