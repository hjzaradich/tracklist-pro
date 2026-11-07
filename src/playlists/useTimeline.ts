import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { unwrap } from "../api/errors";
import { commands, type FillGapReply, type PlaylistChanged, type TargetPoint, type Timeline } from "../bindings";
import { invalidatePlaylists, PLAYLISTS_QUERY_KEY } from "./usePlaylists";

/** Suggestions asked for one gap (the backend ranks them; this is how many are shown). */
export const GAP_SUGGESTIONS = 8;

/** A playlist's timeline (3C-P2b): its entries in order, the flags between them and its target shape. */
export function usePlaylistTimeline(id: number) {
  return useQuery({
    queryKey: [...PLAYLISTS_QUERY_KEY, "timeline", id] as const,
    queryFn: (): Promise<Timeline> => unwrap(commands.playlistTimeline(id)),
  });
}

/** Sets the target shape; one operation. Resolves to it, or `null` when the shape is the one it has. */
export function useSetTarget() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ id, points }: { id: number; points: TargetPoint[] }) => unwrap(commands.setPlaylistTarget(id, points)),
    onSettled: () => invalidatePlaylists(queryClient),
  });
}

/** Clears the target shape; one operation. */
export function useClearTarget() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (id: number) => unwrap(commands.clearPlaylistTarget(id)),
    onSettled: () => invalidatePlaylists(queryClient),
  });
}

/** The tracks that fit the place after entry `after` (`null`: the start); asked only while a gap is open. */
export function useFillGap(id: number, after: number | null, open: boolean) {
  return useQuery({
    queryKey: [...PLAYLISTS_QUERY_KEY, "gap", id, after] as const,
    queryFn: (): Promise<FillGapReply> => unwrap(commands.fillGap(id, after, GAP_SUGGESTIONS, false)),
    enabled: open,
    gcTime: 0,
  });
}

/** "Insert here": one Library track at position `at` of the playlist, one operation. */
export function useInsertAt() {
  const queryClient = useQueryClient();
  return useMutation<PlaylistChanged, Error, { id: number; track: number; at: number }>({
    mutationFn: ({ id, track, at }) => unwrap(commands.addTracksToPlaylist(id, [track], at)),
    onSettled: () => invalidatePlaylists(queryClient),
  });
}
