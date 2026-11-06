import { type QueryClient, useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { unwrap } from "../api/errors";
import {
  commands,
  type TagGroup,
  type TagsChanged,
  type TagsImported,
  type VocabItem,
} from "../bindings";
import { ALL_MUSIC_TABLE_QUERY_KEY } from "../allMusic/useAllMusic";
import { LIBRARY_TABLE_QUERY_KEY } from "../library/useLibrarySummary";

/** Every vocabulary query starts with this, so one invalidation reaches them all. */
export const VOCAB_QUERY_KEY = ["vocab"] as const;

/**
 * Call after anything that can change the vocabulary or what a track has: the
 * track tables can be filtered by tag, so they are asked again too.
 */
export function invalidateVocab(queryClient: QueryClient): Promise<void> {
  void queryClient.invalidateQueries({ queryKey: LIBRARY_TABLE_QUERY_KEY });
  void queryClient.invalidateQueries({ queryKey: ALL_MUSIC_TABLE_QUERY_KEY });
  return queryClient.invalidateQueries({ queryKey: VOCAB_QUERY_KEY });
}

/** Every group with its tags. */
export function useVocab() {
  return useQuery({
    queryKey: [...VOCAB_QUERY_KEY, "list"] as const,
    queryFn: (): Promise<TagGroup[]> => unwrap(commands.listTags()),
  });
}

/** The ids of the tags a track has. */
export function useTagsOfTrack(recordingId: number) {
  return useQuery({
    queryKey: [...VOCAB_QUERY_KEY, "of", recordingId] as const,
    queryFn: (): Promise<number[]> => unwrap(commands.tagsOfTrack(recordingId)),
  });
}

function useVocabMutation<Input, Output>(run: (input: Input) => Promise<Output>) {
  const queryClient = useQueryClient();
  return useMutation<Output, Error, Input>({
    mutationFn: run,
    onSettled: () => invalidateVocab(queryClient),
  });
}

export const useCreateGroup = () =>
  useVocabMutation((name: string) => unwrap(commands.createTagGroup(name)));

export const useCreateTag = () =>
  useVocabMutation(({ group, name }: { group: number; name: string }) =>
    unwrap(commands.createTag(group, name)),
  );

export const useRenameItem = () =>
  useVocabMutation(({ item, name }: { item: VocabItem; name: string }) =>
    unwrap(commands.renameTagItem(item, name)),
  );

/** A `#rrggbb` colour, or `null` for the default. */
export const useColourItem = () =>
  useVocabMutation(({ item, colour }: { item: VocabItem; colour: string | null }) =>
    unwrap(commands.colourTagItem(item, colour)),
  );

export const useApplyTag = () =>
  useVocabMutation(
    ({ tag, recordings }: { tag: number; recordings: readonly number[] }): Promise<TagsChanged> =>
      unwrap(commands.applyTag(tag, [...recordings])),
  );

export const useRemoveTag = () =>
  useVocabMutation(
    ({ tag, recordings }: { tag: number; recordings: readonly number[] }): Promise<TagsChanged> =>
      unwrap(commands.removeTag(tag, [...recordings])),
  );

/** Copies rekordbox's My Tags in; `groupName` names the group new tags go in. */
export const useImportTags = () =>
  useVocabMutation((groupName: string): Promise<TagsImported> => unwrap(commands.importRekordboxTags(groupName)));
