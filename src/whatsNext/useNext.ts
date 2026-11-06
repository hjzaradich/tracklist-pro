import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { unwrap } from "../api/errors";
import { commands, type Direction, type NextReply, type NextSettings } from "../bindings";

/** Everything "What's next" asks for starts with this. */
export const NEXT_QUERY_KEY = ["next"] as const;
const SETTINGS_KEY = [...NEXT_QUERY_KEY, "settings"] as const;

/** The best tracks to play after `recordingId`, going `direction` in energy. */
export function useNext(recordingId: number, direction: Direction) {
  return useQuery({
    queryKey: [...NEXT_QUERY_KEY, "for", recordingId, direction],
    queryFn: (): Promise<NextReply> => unwrap(commands.nextFor(recordingId, direction)),
  });
}

/** The weights and the tolerance. */
export function useNextSettings() {
  return useQuery({
    queryKey: SETTINGS_KEY,
    queryFn: (): Promise<NextSettings> => unwrap(commands.nextSettings()),
  });
}

/** Stores the weights and the tolerance (not an operation), then asks for the suggestions again. */
export function useSetNextSettings() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (settings: NextSettings): Promise<NextSettings> => unwrap(commands.nextSetSettings(settings)),
    onSuccess: (stored) => {
      queryClient.setQueryData(SETTINGS_KEY, stored);
      void queryClient.invalidateQueries({ queryKey: [...NEXT_QUERY_KEY, "for"] });
    },
  });
}
