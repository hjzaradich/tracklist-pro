import { useEffect, useRef } from "react";
import { events } from "../bindings";

/**
 * Calls `onEnd` whenever a background task ends (done, failed or
 * cancelled): the moment to ask again for anything that task could have
 * changed. Outside the app (e.g. a plain browser) no events ever arrive.
 */
export function useWhenJobsEnd(onEnd: () => void) {
  const latest = useRef(onEnd);
  useEffect(() => {
    latest.current = onEnd;
  });
  useEffect(() => {
    let stopped = false;
    let stop: (() => void) | undefined;
    events.jobUpdates
      .listen((event) => {
        const ended = event.payload.some(
          (job) => job.status !== "queued" && job.status !== "running",
        );
        if (ended) latest.current();
      })
      .then((unlisten) => {
        if (stopped) unlisten();
        else stop = unlisten;
      })
      .catch(() => {});
    return () => {
      stopped = true;
      stop?.();
    };
  }, []);
}
