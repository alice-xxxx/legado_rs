import type { ResourceDescriptor } from "./types";

async function readResponse<T>(src: string, decode: (response: Response) => Promise<T>, signal?: AbortSignal): Promise<T> {
  try {
    const response = await fetch(src, { cache: "no-store", signal });
    if (!response.ok) throw new ResourceHttpError(response.status, `读取资源失败（HTTP ${response.status}）。`);
    return await decode(response);
  } catch (error) {
    if (signal?.aborted) throw error;
    if (!(error instanceof TypeError) && !(error instanceof DOMException && error.name === "NetworkError")) throw error;
    throw new Error(`无法读取内容：${error instanceof Error ? error.message : String(error)}`);
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
