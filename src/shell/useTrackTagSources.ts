import { useQuery } from "@tanstack/react-query";
import { unwrap } from "../api/errors";
import { commands, type TagSources } from "../bindings";

/** The Details panel's tag sources, per track; asked for again whenever the lists are. */
export const TRACK_TAG_SOURCES_QUERY_KEY = ["trackTagSources"] as const;

/**
 * Each tag field of a track with its value and where it comes from, and the
 * confirmation that can be withdrawn (1cB-17). `shownFile` is the file of the
 * selected row, whose path the panel shows: a track outside the Library reads
 * its tags from that file first, so the path and the tags are one file's.
 */
export function useTrackTagSources(recordingId: number, shownFile: number | null) {
  return useQuery({
    queryKey: [...TRACK_TAG_SOURCES_QUERY_KEY, recordingId, shownFile],
    queryFn: (): Promise<TagSources> => unwrap(commands.trackTagSources(recordingId, shownFile)),
  });
}
