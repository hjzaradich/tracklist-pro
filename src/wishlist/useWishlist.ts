import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { reloadLists } from "../app/lists";
import { unwrap } from "../api/errors";
import { commands, type WishItemsReply, type WishlistOutcome, type WishlistsReply } from "../bindings";

/** Everything the wishlist screen shows starts with this, so one invalidation reaches it all. */
export const WISHLIST_QUERY_KEY = ["wishlist"] as const;
const LISTS_KEY = [...WISHLIST_QUERY_KEY, "lists"] as const;
const ITEMS_KEY = [...WISHLIST_QUERY_KEY, "items"] as const;
/** The on/off switch is a setting, not a list: Undo and imports leave it alone. */
export const WISHLIST_FEATURE_QUERY_KEY = ["setting", "feature", "wishlist"] as const;

/**
 * Whether the user has turned wishlists on (off until they do). `undefined`
 * while it is being read, and when it can't be read: the sidebar entry stays
 * hidden then, and the screen shows nothing yet.
 */
export function useWishlistFeature() {
  return useQuery({
    queryKey: WISHLIST_FEATURE_QUERY_KEY,
    queryFn: () => unwrap(commands.featureEnabled("wishlist")),
  });
}

/** Turns wishlists on or off. A setting is never an operation, so there is nothing to Undo. */
export function useSetWishlistFeature() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: async (on: boolean) => {
      await unwrap(commands.setFeatureEnabled("wishlist", on));
      return on;
    },
    onSuccess: (on) => queryClient.setQueryData(WISHLIST_FEATURE_QUERY_KEY, on),
  });
}

/** The wishlists with a count per result; only asked for while the feature is on. */
export function useWishlists(enabled: boolean) {
  return useQuery({
    queryKey: LISTS_KEY,
    enabled,
    queryFn: (): Promise<WishlistsReply> => unwrap(commands.wishlistsList()),
  });
}

/** The items of one wishlist, each with its result and reason. */
export function useWishItems(wishlistId: number | null) {
  return useQuery({
    queryKey: [...ITEMS_KEY, wishlistId],
    enabled: wishlistId !== null,
    queryFn: (): Promise<WishItemsReply> => unwrap(commands.wishlistItems(wishlistId ?? 0)),
  });
}

/** What the screen can ask of a wishlist. Each is one operation, taken back by the top bar's Undo. */
export type WishlistAction =
  | { kind: "createFromPaste"; name: string; pasted: string }
  | { kind: "createFromCsv"; name: string; path: string }
  | { kind: "delete"; wishlistId: number }
  | { kind: "removeItem"; itemId: number }
  | { kind: "setBought"; itemId: number; bought: boolean }
  | { kind: "setNotes"; itemId: number; notes: string | null };

function run(action: WishlistAction): Promise<WishlistOutcome> {
  switch (action.kind) {
    case "createFromPaste":
      return unwrap(commands.wishlistCreateFromPaste(action.name, action.pasted));
    case "createFromCsv":
      return unwrap(commands.wishlistCreateFromCsv(action.name, action.path));
    case "delete":
      return unwrap(commands.wishlistDelete(action.wishlistId));
    case "removeItem":
      return unwrap(commands.wishlistRemoveItem(action.itemId));
    case "setBought":
      return unwrap(commands.wishlistSetBought(action.itemId, action.bought));
    case "setNotes":
      return unwrap(commands.wishlistSetNotes(action.itemId, action.notes));
  }
}

/** Runs a wishlist action, then asks again for the wishlists and their items. */
export function useWishlistAction() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: run,
    onSuccess: () => reloadLists(queryClient, "wishlists"),
  });
}
