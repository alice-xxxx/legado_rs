import type { ResourceDescriptor } from "./types";

export async function readResource<T>(resource: ResourceDescriptor | string, signal?: AbortSignal): Promise<T> {
  const src = typeof resource === "string" ? resource : resource.src;
  if (!src) throw new Error("资源暂时不可用。");
  let response: Response;
  try {
    response = await fetch(src, { cache: "no-store", signal });
  } catch (error) {
    if (signal?.aborted) throw error;
    throw new Error(`无法读取内容：${error instanceof Error ? error.message : String(error)}`);
  }
  if (!response.ok) {
    throw new Error(`读取资源失败（HTTP ${response.status}）：${src}`);
  }
  return (await response.json()) as T;
}

async function readTextFromSrc(src: string): Promise<string> {
  if (!src) throw new Error("章节内容暂时不可用。");
  let response: Response;
  try {
    response = await fetch(src, { cache: "no-store" });
  } catch (error) {
    throw new Error(`无法读取章节资源：${error instanceof Error ? error.message : String(error)}`);
  }
  if (!response.ok) throw new ResourceHttpError(response.status, `读取章节失败（HTTP ${response.status}）。`);
  return response.text();
}

export const readTextResource = (descriptor: ResourceDescriptor): Promise<string> =>
  readTextFromSrc(descriptor.src);

export class ResourceHttpError extends Error {
  constructor(readonly status: number, message: string) {
    super(message);
    this.name = "ResourceHttpError";
  }
}
