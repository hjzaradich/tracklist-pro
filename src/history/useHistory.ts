import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { reloadLists } from "../app/lists";
import { unwrap } from "../api/errors";
import {
  commands,
  type HistoryOutcome,
  type PlaysReply,
  type SessionSummary,
  type StickImport,
  type StickListing,
  type WishlistOutcome,
} from "../bindings";

/** Everything the History screen shows starts with this, so one invalidation reaches it all. */
export const HISTORY_QUERY_KEY = ["history"] as const;
const SESSIONS_KEY = [...HISTORY_QUERY_KEY, "sessions"] as const;
const PLAYS_KEY = [...HISTORY_QUERY_KEY, "plays"] as const;

/** Every imported set, the latest played first. */
export function useSessions() {
  return useQuery({
    queryKey: SESSIONS_KEY,
    queryFn: (): Promise<SessionSummary[]> => unwrap(commands.historySessions()),
  });
}

/** The plays of one set, in order, and whether the wishlist offer is shown. */
export function usePlays(sessionId: number | null) {
  return useQuery({
    queryKey: [...PLAYS_KEY, sessionId],
    enabled: sessionId !== null,
    queryFn: (): Promise<PlaysReply> => unwrap(commands.historyPlays(sessionId ?? 0)),
  });
}

/** What the screen can ask of History. Each is one operation, taken back by the top bar's Undo. */
export type HistoryAction =
  | { kind: "import"; path: string }
  | { kind: "setDate"; sessionId: number; playedOn: string };

/** Imports a file or corrects a date, then asks again for the sets. */
export function useHistoryAction() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (action: HistoryAction): Promise<HistoryOutcome> =>
      action.kind === "import"
        ? unwrap(commands.historyImportM3u8(action.path))
        : unwrap(commands.historySetDate(action.sessionId, action.playedOn)),
    onSuccess: () => reloadLists(queryClient, "history"),
  });
}

/** Adds a set's unmatched plays to a wishlist, then asks again for the wishlists. */
export function useAddToWishlist() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (request: { sessionId: number; wishlistId: number }): Promise<WishlistOutcome> =>
      unwrap(commands.historyAddToWishlist(request.sessionId, request.wishlistId)),
    onSuccess: () => reloadLists(queryClient, "wishlists"),
  });
}

/** Reads a gig stick's history lists; the stick is only read. */
export function useStickLists() {
  return useMutation({
    mutationFn: (root: string): Promise<StickListing> => unwrap(commands.historyStickLists(root)),
  });
}

/** Imports the chosen lists of a gig stick as sets (one step for Undo), then asks again for the sets. */
export function useStickImport() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (request: { root: string; listIds: number[]; playedOn: string }): Promise<StickImport> =>
      unwrap(commands.historyImportStick(request.root, request.listIds, request.playedOn)),
    onSuccess: () => reloadLists(queryClient, "history"),
  });
}
