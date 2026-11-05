import { useEffect, useState } from "react";
import { unwrap } from "../api/errors";
import { type AddProgressState, commands } from "../bindings";

/** An add quicker than this shows nothing, so a quick one doesn't flicker. */
export const SHOW_PROGRESS_AFTER_MS = 1000;

/** How often the backend is asked how far an add has got, once it shows. */
export const PROGRESS_EVERY_MS = 400;

/**
 * How far the add to a crate has got (1cE-P2), while `active` is true (the
 * add is pending) and only once it has taken a while. `null` before that, and
 * as soon as it is no longer pending. The backend answers without waiting for
 * the add, which holds the database writer.
 */
export function useAddProgress(active: boolean): AddProgressState | null {
  const [reading, setReading] = useState<AddProgressState | null>(null);

  useEffect(() => {
    if (!active) return;
    let live = true;
    let timer: ReturnType<typeof setTimeout>;
    const ask = async () => {
      try {
        const now = await unwrap(commands.crateAddProgress());
        if (live) setReading(now);
      } catch {
        // The line is a nicety: a failed look just shows the last one.
      }
      if (live) timer = setTimeout(() => void ask(), PROGRESS_EVERY_MS);
    };
    timer = setTimeout(() => void ask(), SHOW_PROGRESS_AFTER_MS);
    return () => {
      live = false;
      clearTimeout(timer);
      setReading(null);
    };
  }, [active]);

  return active ? reading : null;
}
