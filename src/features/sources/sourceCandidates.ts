import { appBootstrap } from "../../api/core";
import { readResource } from "../../api/resources";
import type { ResourceDescriptor, SourceDefinitionsResource, SourceMetadata } from "../../api/types";

/** Read enabled non-RSS sources used for book search without passing app state through route props. */
export async function loadBookSourceCandidates(descriptor?: ResourceDescriptor): Promise<SourceMetadata[]> {
  const sourceDescriptor = descriptor ?? (await appBootstrap()).sources;
  const document = await readResource<SourceDefinitionsResource>(sourceDescriptor);
  if (!Array.isArray(document.sources)) throw new Error("Source resource has an invalid format");
  return document.sources
    .filter((source) => source.enabled && source.isRss !== true)
    .map(({ definitionJson: _definitionJson, ...source }) => source);
}
