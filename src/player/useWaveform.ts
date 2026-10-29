import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect } from "react";
import { unwrap } from "../api/errors";
import { commands, type Waveform } from "../bindings";
import { useWhenJobsEnd } from "../activity/useWhenJobsEnd";

/** How often a waveform that isn't drawn yet is asked for again, in milliseconds. */
export const WAVEFORM_POLL_MS = 1000;

/** Every waveform query starts with this. */
export const WAVEFORM_QUERY_KEY = ["waveform"] as const;

/**
 * A file's waveform (1cB-9): the peaks the waveform job cached, "not ready"
 * while it hasn't reached the file, or "unavailable" for a file that can't
 * be decoded. Reading is cheap and never decodes. The job draws waveforms in
 * the background, so a file whose waveform isn't there yet is asked for
 * first (the job draws it before the rest) and looked at again every second
 * until it lands; the ones on screen are also asked for again whenever a
 * task ends. `null` asks for nothing.
 */
export function useWaveform(fileId: number | null) {
  const queryClient = useQueryClient();
  useWhenJobsEnd(() => {
    void queryClient.invalidateQueries({ queryKey: WAVEFORM_QUERY_KEY });
  });
  const query = useQuery({
    queryKey: [...WAVEFORM_QUERY_KEY, fileId ?? -1] as const,
    queryFn: (): Promise<Waveform> => unwrap(commands.waveformPeaks(fileId ?? -1)),
    enabled: fileId !== null,
    refetchInterval: (q) => (q.state.data?.state === "notReady" ? WAVEFORM_POLL_MS : false),
  });
  const notReady = query.data?.state === "notReady";
  useEffect(() => {
    if (fileId === null || !notReady) return;
    // Best effort: the waveform arrives either way, only later.
    void Promise.resolve(commands.waveformFirst(fileId)).catch(() => {});
  }, [fileId, notReady]);
  return query;
}
