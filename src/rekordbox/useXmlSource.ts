import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { unwrap } from "../api/errors";
import { commands, type XmlSource } from "../bindings";

// The rekordbox XML source (1aB-11): the export the user chose, the watch for
// newer exports, and the last read. The read itself is a job (Activity).

export const XML_SOURCE_QUERY_KEY = ["rekordbox", "xmlSource"] as const;

/** How often the watch looks for a newer export while the screen is open. */
export const WATCH_INTERVAL_MS = 15_000;
/** How often the source is asked again while a read is under way. */
export const READING_INTERVAL_MS = 1_000;

function fetchXmlSource(): Promise<XmlSource> {
  return unwrap(commands.rekordboxXmlSource());
}

/**
 * The XML source. With the watch on it's asked again every
 * {@link WATCH_INTERVAL_MS}. While a read begun at `readingSince` (the
 * source's {@link readMarker} then) hasn't been recorded, it's asked every
 * {@link READING_INTERVAL_MS}.
 */
export function useXmlSource(readingSince: string | null) {
  return useQuery({
    queryKey: XML_SOURCE_QUERY_KEY,
    queryFn: fetchXmlSource,
    refetchInterval: (query) => {
      const data = query.state.data;
      if (readingSince !== null && readMarker(data) === readingSince) return READING_INTERVAL_MS;
      return data?.watch ? WATCH_INTERVAL_MS : false;
    },
  });
}

/**
 * Reads an export as a job: `path` (a file just picked, or a newer export)
 * becomes the chosen one; `null` reads the chosen one again. Resolves to the
 * job's id.
 */
export function useReadXml() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (path: string | null) => unwrap(commands.readRekordboxXml(path)),
    onSettled: () => queryClient.invalidateQueries({ queryKey: XML_SOURCE_QUERY_KEY }),
  });
}

/** Turns the watch for newer exports on or off. */
export function useSetWatch() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (watch: boolean) => unwrap(commands.setRekordboxXmlWatch(watch)),
    onSettled: () => queryClient.invalidateQueries({ queryKey: XML_SOURCE_QUERY_KEY }),
  });
}

/**
 * What changes when a read finishes, either way: the last read's time or the
 * last failure's. Compared before and after, to tell a read has ended.
 */
export function readMarker(source: XmlSource | undefined): string {
  return `${source?.lastRead?.readAt ?? ""}|${source?.lastFailure?.at ?? ""}`;
}
