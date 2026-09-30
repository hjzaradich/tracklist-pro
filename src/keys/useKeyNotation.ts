import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { unwrap } from "../api/errors";
import { commands } from "../bindings";
import { DEFAULT_KEY_NOTATION, KEY_NOTATIONS, type KeyNotation } from "./keyNotation";

// The key-notation setting, kept in the backend's `setting` table. There's no
// settings screen yet (0E-14); screens that show keys read it with
// useKeyNotation() and format with formatKey().

export const KEY_NOTATION_QUERY_KEY = ["setting", "key_notation"] as const;

function fetchKeyNotation(): Promise<KeyNotation> {
  return unwrap(commands.keyNotation());
}

/**
 * The notation keys are shown in. Camelot while it loads, if it can't be
 * read, or if the backend names a notation this build doesn't know.
 */
export function useKeyNotation(): KeyNotation {
  const { data } = useQuery({ queryKey: KEY_NOTATION_QUERY_KEY, queryFn: fetchKeyNotation });
  return data !== undefined && KEY_NOTATIONS.includes(data) ? data : DEFAULT_KEY_NOTATION;
}

/** Changes the notation for the whole app; every useKeyNotation() follows. */
export function useSetKeyNotation() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: async (notation: KeyNotation) => {
      await unwrap(commands.setKeyNotation(notation));
      return notation;
    },
    onSuccess: (notation) => queryClient.setQueryData(KEY_NOTATION_QUERY_KEY, notation),
  });
}
