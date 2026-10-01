import { useEffect, useState } from "react";

/** Runs `run` later, and returns how to call it off. */
export type Schedule = (run: () => void) => () => void;

/** A schedule that waits `ms` on the clock. */
export function after(ms: number): Schedule {
  return (run) => {
    const timer = setTimeout(run, ms);
    return () => clearTimeout(timer);
  };
}

/**
 * `value`, once it has stopped changing: each change is scheduled, and a
 * newer change calls the older one off. Tests pass their own schedule, so
 * they never wait on the clock.
 */
export function useDebounced<T>(value: T, schedule: Schedule): T {
  const [settled, setSettled] = useState(value);
  useEffect(() => schedule(() => setSettled(value)), [value, schedule]);
  return settled;
}
