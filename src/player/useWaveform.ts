import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect, useRef } from "react";
import { unwrap } from "../api/errors";
import { commands, type Waveform } from "../bindings";
import { useWhenJobsEnd } from "../activity/useWhenJobsEnd";

/**
 * How often a waveform that isn't drawn yet is asked for again: every
 * {@link WAVEFORM_POLL_MS} for the first {@link WAVEFORM_QUICK_POLLS} looks,
 * then every {@link WAVEFORM_SLOW_POLL_MS}, and no more after
 * {@link WAVEFORM_POLL_LIMIT} looks. A file that is never drawn (no hash
 * yet, its drive unplugged) so costs a bounded number of cheap reads; the
 * next task to end looks once more and starts a bounded run again.
 */
export const WAVEFORM_POLL_MS = 1000;
export const WAVEFORM_SLOW_POLL_MS = 5000;
export const WAVEFORM_QUICK_POLLS = 5;
export const WAVEFORM_POLL_LIMIT = 20;

/** Every waveform query starts with this. */
export const WAVEFORM_QUERY_KEY = ["waveform"] as const;

/**
 * A file's waveform (1cB-9): the peaks the waveform job cached, "not ready"
 * while it hasn't reached the file, or "unavailable" for a file that can't
 * be decoded. Reading is cheap and never decodes. The job draws waveforms in
 * the background, so a file whose waveform isn't there yet is asked for
 * first (the job draws it before the rest) and looked at again, on the
 * schedule above, until it lands; the ones on screen are also asked for
 * again whenever a task ends. `null` asks for nothing.
 */
export function useWaveform(fileId: number | null) {
  const queryClient = useQueryClient();
  /** The looks already made when the last task ended, per file: the count starts over from there. */
  const since = useRef(new Map<number, number>());
  useWhenJobsEnd(() => {
    if (fileId !== null) {
      const state = queryClient.getQueryState([...WAVEFORM_QUERY_KEY, fileId]);
      since.current.set(fileId, state?.dataUpdateCount ?? 0);
    }
    void queryClient.invalidateQueries({ queryKey: WAVEFORM_QUERY_KEY });
  });
  const query = useQuery({
    queryKey: [...WAVEFORM_QUERY_KEY, fileId ?? -1] as const,
    queryFn: (): Promise<Waveform> => unwrap(commands.waveformPeaks(fileId ?? -1)),
    enabled: fileId !== null,
    refetchInterval: (q) => {
      if (q.state.data?.state !== "notReady") return false;
      const looks = q.state.dataUpdateCount - (since.current.get(fileId ?? -1) ?? 0);
      if (looks >= WAVEFORM_POLL_LIMIT) return false;
      return looks < WAVEFORM_QUICK_POLLS ? WAVEFORM_POLL_MS : WAVEFORM_SLOW_POLL_MS;
    },
  });
  const notReady = query.data?.state === "notReady";
  useEffect(() => {
    if (fileId === null || !notReady) return;
    // Best effort: the waveform arrives either way, only later.
    void Promise.resolve(commands.waveformFirst(fileId)).catch(() => {});
  }, [fileId, notReady]);
  return query;
}
