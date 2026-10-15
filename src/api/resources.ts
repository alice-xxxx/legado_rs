import type { ResourceDescriptor } from "./types";

function isLocalResource(src: string): boolean {
  try {
    const url = new URL(src);
    return (url.hostname === "127.0.0.1" || url.hostname === "localhost")
      && url.pathname.startsWith("/r/");
  } catch {
    return false;
  }
}

function waitBeforeRetry(ms: number, signal?: AbortSignal): Promise<void> {
  return new Promise((resolve, reject) => {
    if (signal?.aborted) { reject(signal.reason); return; }
    const onAbort = () => {
      clearTimeout(timer);
      reject(signal?.reason);
    };
    const timer = setTimeout(() => {
      signal?.removeEventListener("abort", onAbort);
      resolve();
    }, ms);
    signal?.addEventListener("abort", onAbort, { once: true });
  });
}

function waitForForeground(signal?: AbortSignal): Promise<void> {
  if (typeof document === "undefined" || document.visibilityState !== "hidden") return Promise.resolve();
  return new Promise((resolve, reject) => {
    const cleanup = () => {
      document.removeEventListener("visibilitychange", onVisible);
      signal?.removeEventListener("abort", onAbort);
    };
    const onAbort = () => { cleanup(); reject(signal?.reason ?? new DOMException("Aborted", "AbortError")); };
    const onVisible = () => { if (document.visibilityState !== "hidden") { cleanup(); resolve(); } };
    if (signal?.aborted) { onAbort(); return; }
    document.addEventListener("visibilitychange", onVisible);
    signal?.addEventListener("abort", onAbort, { once: true });
  });
}

export class ResourceNetworkError extends Error {
  constructor(message: string) { super(message); this.name = "ResourceNetworkError"; }
}

async function readResponse<T>(src: string, decode: (response: Response) => Promise<T>, signal?: AbortSignal): Promise<T> {
  const delays = isLocalResource(src) ? [250, 750, 1_500] : [];
  for (let attempt = 0; ; attempt++) {
    try {
      const response = await fetch(src, { cache: "no-store", signal });
      if (!response.ok) throw new ResourceHttpError(response.status, `读取资源失败（HTTP ${response.status}）。`);
      // WebKit may lose the connection after receiving headers. Retry the body
      // read too; JSON/schema/HTTP errors must keep their original meaning.
      return await decode(response);
    } catch (error) {
      if (signal?.aborted) throw error;
      if (!(error instanceof TypeError) && !(error instanceof DOMException && error.name === "NetworkError")) throw error;
      if (attempt === delays.length) throw new ResourceNetworkError(`无法读取内容：${error instanceof Error ? error.message : String(error)}`);
      await waitForForeground(signal);
      await waitBeforeRetry(delays[attempt], signal);
    }
  }
}

export async function readResource<T>(resource: ResourceDescriptor | string, signal?: AbortSignal): Promise<T> {
  const src = typeof resource === "string" ? resource : resource.src;
  if (!src) throw new Error("资源暂时不可用。");
  return readResponse(src, response => response.json() as Promise<T>, signal);
}

async function readTextFromSrc(src: string, signal?: AbortSignal): Promise<string> {
  if (!src) throw new Error("章节内容暂时不可用。");
  return readResponse(src, response => response.text(), signal);
}

export const readTextResource = (descriptor: ResourceDescriptor, signal?: AbortSignal): Promise<string> =>
  readTextFromSrc(descriptor.src, signal);

export class ResourceHttpError extends Error {
  constructor(readonly status: number, message: string) {
    super(message);
    this.name = "ResourceHttpError";
  }
}
