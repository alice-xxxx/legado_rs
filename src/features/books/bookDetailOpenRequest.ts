import type { Ref } from "vue";
import type { SearchBookResult } from "../../api/types";

export type BookDetailTab = "catalog" | "info" | "sources";

export interface BookDetailSearchResultActions {
  searchResultBatchBusy: Readonly<Ref<boolean>>;
  searchResultIsBusy: (result: SearchBookResult) => boolean;
  addSearchResult: (result: SearchBookResult) => Promise<void>;
}

type BookDetailOpenRequestInput =
  | {
    kind: "search-result";
    result: SearchBookResult;
    actions: BookDetailSearchResultActions;
  }
  | {
    kind: "shelf-book";
    bookId: string;
    initialTab?: BookDetailTab;
  };

export type BookDetailOpenRequest = BookDetailOpenRequestInput & { requestId: number };

let nextRequestId = 0;
let pendingRequest: BookDetailOpenRequest | null = null;

/** Publishes one detail-page request for the destination feature to consume. */
export function requestBookDetailOpen(input: BookDetailOpenRequestInput): number {
  const requestId = ++nextRequestId;
  pendingRequest = input.kind === "search-result"
    ? { ...input, requestId }
    : { ...input, requestId };
  return requestId;
}

/** Consumes the pending handoff once; detail-page state stays owned by BookDetailPage. */
export function consumeBookDetailOpenRequest(): BookDetailOpenRequest | null {
  const request = pendingRequest;
  pendingRequest = null;
  return request;
}

/** Clears only the request whose navigation failed, preserving any newer handoff. */
export function discardBookDetailOpenRequest(requestId: number): void {
  if (pendingRequest?.requestId === requestId) pendingRequest = null;
}
