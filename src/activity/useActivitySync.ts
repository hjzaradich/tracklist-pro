import { useEffect } from "react";
import { commands, events } from "../bindings";
import { useActivityStore } from "./activityStore";

/**
 * How long to wait before asking for the snapshot again after the app
 * couldn't answer, in ms. The last wait repeats until it answers.
 */
export const SNAPSHOT_RETRY_MS = [100, 250, 500, 1000, 2000, 5000];

/**
 * Keeps the activity store in step with the app: listens for job updates
 * first, then fetches the snapshot, so no update falls between the two. If
 * the app can't give a snapshot, it asks again, waiting longer each time;
 * updates heard meanwhile are held (a bounded number, see the store).
 */
export function useActivitySync() {
  useEffect(() => {
    const { applySnapshot, applyUpdates, reset } = useActivityStore.getState();
    let stopped = false;
    let unlisten: (() => void) | undefined;
    let retry: ReturnType<typeof setTimeout> | undefined;

    const fetchSnapshot = async (attempt: number): Promise<void> => {
      const snapshot = await commands.activity();
      if (stopped) return;
      if (snapshot.status === "ok") {
        applySnapshot(snapshot.data);
        return;
      }
      const wait = SNAPSHOT_RETRY_MS[Math.min(attempt, SNAPSHOT_RETRY_MS.length - 1)];
      retry = setTimeout(() => {
        fetchSnapshot(attempt + 1).catch(() => {});
      }, wait);
    };

    events.jobUpdates
      .listen((event) => applyUpdates(event.payload))
      .then(async (stop) => {
        unlisten = stop;
        if (stopped) return stop();
        await fetchSnapshot(0);
      })
      // Outside the app (e.g. a plain browser) there are no jobs to show;
      // the status stays idle.
      .catch(() => {});

    return () => {
      stopped = true;
      clearTimeout(retry);
      unlisten?.();
      reset();
    };
  }, []);
}
