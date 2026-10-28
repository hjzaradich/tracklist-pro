import { useEffect, useRef } from "react";
import { create } from "zustand";

/**
 * How many times an Undo has taken an operation back (the top bar's or
 * Ctrl+Z; no screen has one of its own, owner decision 2026-10-27). The
 * shell counts them (`useKeepNextUndoCurrent`); a screen whose line says what
 * its last action did watches the count (`useWhenUndone`) and drops the line,
 * which describes an action that no longer stands.
 */
export const useUndoneCount = create<{ count: number; undone: () => void }>((set) => ({
  count: 0,
  undone: () => set((state) => ({ count: state.count + 1 })),
}));

/** Calls `onUndone` each time an Undo took an operation back after this mounted. */
export function useWhenUndone(onUndone: () => void) {
  const count = useUndoneCount((state) => state.count);
  const seen = useRef(count);
  const latest = useRef(onUndone);
  useEffect(() => {
    latest.current = onUndone;
  });
  useEffect(() => {
    if (count === seen.current) return;
    seen.current = count;
    latest.current();
  }, [count]);
}
