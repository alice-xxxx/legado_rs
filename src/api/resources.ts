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

async function fetchResource(src: string, signal?: AbortSignal): Promise<Response> {
  const retryDelays = isLocalResource(src) ? [250, 750, 1_500] : [];
  for (let attempt = 0; ; attempt++) {
    try {
      return await fetch(src, { cache: "no-store", signal });
    } catch (error) {
      if (signal?.aborted || attempt === retryDelays.length) throw error;
      await waitBeforeRetry(retryDelays[attempt], signal);
    }
  }
}

export async function readResource<T>(resource: ResourceDescriptor | string, signal?: AbortSignal): Promise<T> {
  const src = typeof resource === "string" ? resource : resource.src;
  if (!src) throw new Error("资源暂时不可用。");
  let response: Response;
  try {
    response = await fetchResource(src, signal);
  } catch (error) {
    if (signal?.aborted) throw error;
    throw new Error(`无法读取内容：${error instanceof Error ? error.message : String(error)}`);
  }
  if (!response.ok) {
    throw new Error(`读取资源失败（HTTP ${response.status}）：${src}`);
  }
  return (await response.json()) as T;
}

async function readTextFromSrc(src: string, signal?: AbortSignal): Promise<string> {
  if (!src) throw new Error("章节内容暂时不可用。");
  let response: Response;
  try {
    response = await fetchResource(src, signal);
  } catch (error) {
    if (signal?.aborted) throw error;
    throw new Error(`无法读取章节资源：${error instanceof Error ? error.message : String(error)}`);
  }
  if (!response.ok) throw new ResourceHttpError(response.status, `读取章节失败（HTTP ${response.status}）。`);
  return response.text();
}

export const readTextResource = (descriptor: ResourceDescriptor, signal?: AbortSignal): Promise<string> =>
  readTextFromSrc(descriptor.src, signal);

export class ResourceHttpError extends Error {
  constructor(readonly status: number, message: string) {
    super(message);
    this.name = "ResourceHttpError";
  }
}
