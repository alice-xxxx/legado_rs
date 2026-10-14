import type { SearchBookResult } from "../../api/types";
import router, { routeNames } from "../../router";

export type BookDetailTab = "catalog" | "info" | "sources";

type BookDetailOpenRequestInput =
  | {
    kind: "search-result";
    result: SearchBookResult;
  }
  | {
    kind: "shelf-book" | "cached-book";
    bookId: string;
    initialTab?: BookDetailTab;
  };

export type BookDetailOpenRequest = BookDetailOpenRequestInput & {
  requestId: number;
  returnPath: string;
  returnPosition: number | null;
};

let nextRequestId = 0;
let pendingRequest: BookDetailOpenRequest | null = null;

/** Publishes one detail-page request for the destination feature to consume. */
export function requestBookDetailOpen(input: BookDetailOpenRequestInput): number {
  const requestId = ++nextRequestId;
  const position = router.options.history.state.position;
  pendingRequest = { ...input, requestId, returnPath: router.currentRoute.value.fullPath,
    returnPosition: typeof position === "number" && Number.isFinite(position) ? position : null };
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

/** Return to the captured entry, even if extra detail entries were added afterward. */
export function returnFromBookDetail(request: BookDetailOpenRequest | null): void {
  const current = router.currentRoute.value;
  if (current.name !== routeNames.bookDetail) return;
  const state = router.options.history.state;
  const origin = request?.returnPath;
  if (origin && router.resolve(origin).name !== routeNames.bookDetail) {
    const position = state.position;
    if (request.returnPosition !== null && typeof position === "number" && Number.isFinite(position)
      && position > request.returnPosition) {
      const remove = router.afterEach(to => {
        remove();
        if (to.fullPath !== origin) void router.replace(origin);
      });
      router.go(request.returnPosition - position);
    }
    else void router.replace(origin);
    return;
  }
  const back = state.back;
  if (typeof back === "string" && router.resolve(back).name !== routeNames.bookDetail) router.back();
  else void router.replace({ name: routeNames.shelf });
}
