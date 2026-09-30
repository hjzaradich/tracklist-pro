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

describe("Activity sync", () => {
  it("asks for the snapshot again until the app can answer", async () => {
    const asked = flakyApp(3, { seq: 1, jobs: [job({ seq: 1, id: 1, progress: 0.6 })] });
    render(<Activity />);
    const wait = SNAPSHOT_RETRY_MS.slice(0, 3).reduce((a, b) => a + b, 0);
    expect(
      await screen.findByText("Scanning music folders (60%)", undefined, { timeout: wait + 2000 }),
    ).toBeInTheDocument();
    expect(asked()).toBe(4);
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
      // A timeout and interval of its own, so none of the test's timers
      // share a wait with the retries.
      expect(
        await screen.findByText("Scanning music folders (60%)", undefined, {
          timeout: 3_001,
          interval: 17,
        }),
      ).toBeInTheDocument();
      expect(asked()).toBe(failures + 1);
      const last = SNAPSHOT_RETRY_MS[SNAPSHOT_RETRY_MS.length - 1];
      expect(waits).toEqual([...SNAPSHOT_RETRY_MS, last, last]);
    } finally {
      spy.mockRestore();
    }
  });

  it("stops asking once the status is gone", async () => {
    const asked = flakyApp(Infinity, { seq: 0, jobs: [] });
    const { unmount } = render(<Activity />);
    await act(async () => {});
    unmount();
    const before = asked();
    await act(() => new Promise((resolve) => setTimeout(resolve, SNAPSHOT_RETRY_MS[1] * 2)));
    expect(asked()).toBe(before);
  });
});
