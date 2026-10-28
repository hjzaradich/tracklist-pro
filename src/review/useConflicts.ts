import {
  useInfiniteQuery,
  useMutation,
  useQuery,
  useQueryClient,
} from "@tanstack/react-query";
import { unwrap } from "../api/errors";
import {
  commands,
  type Choice,
  type ConflictField,
  type ConflictId,
  type ConflictPage,
  type Settled,
  type Side,
} from "../bindings";
import {
  CONFLICT_FIELDS_QUERY_KEY,
  CONFLICTS_QUERY_KEY,
  PAGE_SIZE,
  REVIEW_QUERY_KEYS,
} from "./useReview";

// The Conflicts group of Review (1cB-1, 1cB-2): which fields have open
// conflicts, one field's conflicts, and how they are settled. Each settling
// is one undoable operation on the Rust side (the top bar's Undo takes it
// back); here it only calls the command and asks for the lists again,
// whether it worked or was refused (a refusal means the conflict changed
// under the screen).

/** The fields that have open conflicts, with how many each has. */
export function useConflictFields() {
  return useQuery({
    queryKey: CONFLICT_FIELDS_QUERY_KEY,
    queryFn: (): Promise<ConflictField[]> => unwrap(commands.reviewConflictFields()),
  });
}

/** One field's open conflicts, a page at a time, oldest first. */
export function useConflicts(field: string) {
  return useInfiniteQuery({
    queryKey: [...CONFLICTS_QUERY_KEY, field],
    initialPageParam: 0,
    queryFn: ({ pageParam }): Promise<ConflictPage> =>
      unwrap(commands.reviewConflicts(field, pageParam, PAGE_SIZE)),
    getNextPageParam: (last, pages) => {
      const loaded = pages.reduce((n, page) => n + page.items.length, 0);
      return loaded < last.total && last.items.length > 0 ? loaded : undefined;
    },
  });
}

/** Runs a settling, then asks for everything Review shows again. */
function useSettling<Args, Done>(run: (args: Args) => Promise<Done>) {
  const queryClient = useQueryClient();
  return useMutation<Done, Error, Args>({
    mutationFn: run,
    onSettled: (_done, error) => {
      const reloaded = Promise.all(
        REVIEW_QUERY_KEYS.map((queryKey) => queryClient.invalidateQueries({ queryKey })),
      );
      // After a refusal the lists are asked for again but not awaited, so the
      // reason shows at once.
      return error === null ? reloaded : undefined;
    },
  });
}

/** Settles one conflict: keep the app's value, keep rekordbox's, or a value given by hand. */
export function useResolveConflict() {
  return useSettling<{ id: ConflictId; choice: Choice }, Settled>(({ id, choice }) =>
    unwrap(commands.reviewResolveConflict(id, choice)),
  );
}

/**
 * Keeps one side for every open conflict of a field: one operation, one Undo.
 * `expected` is how many the screen showed; if there are more or fewer now,
 * nothing is settled (the lists are asked for again, and show the new number).
 */
export function useResolveConflicts() {
  return useSettling<{ field: string; side: Side; expected: number }, number>(({ field, side, expected }) =>
    unwrap(commands.reviewResolveConflicts(field, side, expected)),
  );
}
