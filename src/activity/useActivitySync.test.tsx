import { act, cleanup, render, screen } from "@testing-library/react";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { ActivitySnapshot, JobUpdate } from "../bindings";
import { Activity } from "./Activity";
import { SNAPSHOT_RETRY_MS } from "./useActivitySync";

function job(fields: Partial<JobUpdate> & Pick<JobUpdate, "id" | "seq">): JobUpdate {
  return { kind: "scan", status: "running", progress: null, priority: 0, ...fields };
}

/** Answers `activity` with an error `failures` times, then with `snapshot`. */
function flakyApp(failures: number, snapshot: ActivitySnapshot) {
  let asked = 0;
  mockIPC(
    (cmd) => {
      if (cmd !== "activity") throw new Error(`unexpected command ${cmd}`);
      asked += 1;
      if (asked <= failures) throw "database error";
      return snapshot;
    },
    { shouldMockEvents: true },
  );
  return () => asked;
}

afterEach(() => {
  cleanup();
  clearMocks();
});

/** Timers the hook uses, faked; React's own scheduling is left alone. */
const HOOK_TIMERS = ["setTimeout", "clearTimeout"] as const;

/** Lets promises settle and 0 ms timers fire, under fake timers. */
const settle = () => act(() => vi.advanceTimersByTimeAsync(0));

/** Moves the fake clock on by `ms`, running whatever comes due. */
const elapse = (ms: number) => act(() => vi.advanceTimersByTimeAsync(ms));

describe("Activity sync", () => {
  it("asks for the snapshot again until the app can answer, each time after its wait", async () => {
    vi.useFakeTimers({ toFake: [...HOOK_TIMERS] });
    try {
      const asked = flakyApp(3, { seq: 1, jobs: [job({ seq: 1, id: 1, progress: 0.6 })] });
      render(<Activity />);
      // Listening, then the first ask, which fails.
      await settle();
      expect(asked()).toBe(1);
      // Each retry comes once its wait is up, and not a moment before.
      for (const [n, wait] of SNAPSHOT_RETRY_MS.slice(0, 3).entries()) {
        await elapse(wait - 1);
        expect(asked()).toBe(n + 1);
        await elapse(1);
        expect(asked()).toBe(n + 2);
      }
      expect(screen.getByText("Scanning music folders (60%)")).toBeInTheDocument();
    } finally {
      vi.useRealTimers();
    }
  });

  it("waits longer after each failure, then keeps to the longest wait", async () => {
    // The waits it really schedules, as it schedules them.
    const waits: number[] = [];
    const realSetTimeout = globalThis.setTimeout;
    const spy = vi.spyOn(globalThis, "setTimeout").mockImplementation(((
      handler: () => void,
      ms?: number,
    ) => {
      if (ms !== undefined && SNAPSHOT_RETRY_MS.includes(ms)) {
        waits.push(ms);
        // Run it at once, so the test doesn't sit through 13 s of waits.
        return realSetTimeout(handler, 0);
      }
      return realSetTimeout(handler, ms);
    }) as typeof setTimeout);
    try {
      const failures = SNAPSHOT_RETRY_MS.length + 2;
      const asked = flakyApp(failures, { seq: 1, jobs: [job({ seq: 1, id: 1, progress: 0.6 })] });
      render(<Activity />);
      // An interval of its own, so none of the test's timers shares a wait
      // with the retries (the suite's timeout isn't one either).
      expect(
        await screen.findByText("Scanning music folders (60%)", undefined, { interval: 17 }),
      ).toBeInTheDocument();
      expect(asked()).toBe(failures + 1);
      const last = SNAPSHOT_RETRY_MS[SNAPSHOT_RETRY_MS.length - 1];
      expect(waits).toEqual([...SNAPSHOT_RETRY_MS, last, last]);
    } finally {
      spy.mockRestore();
    }
  });

  it("stops asking once the status is gone", async () => {
    vi.useFakeTimers({ toFake: [...HOOK_TIMERS] });
    try {
      const asked = flakyApp(Infinity, { seq: 0, jobs: [] });
      const { unmount } = render(<Activity />);
      await settle();
      unmount();
      const before = asked();
      // Longer than every retry wait put together: nothing is left to fire.
      await elapse(2 * SNAPSHOT_RETRY_MS.reduce((a, b) => a + b, 0));
      expect(asked()).toBe(before);
    } finally {
      vi.useRealTimers();
    }
  });
});
