import { describe, expect, it } from "vitest";
import type { JobUpdate } from "../bindings";
import { summarize } from "./summary";

function job(fields: Partial<JobUpdate> & Pick<JobUpdate, "id">): JobUpdate {
  return { seq: 1, kind: "scan", status: "running", progress: null, priority: 0, ...fields };
}

describe("activity summary", () => {
  it("is idle with no jobs", () => {
    expect(summarize([])).toEqual({ state: "idle" });
  });

  it("counts queued jobs when none is running yet", () => {
    expect(summarize([job({ id: 1, status: "queued" }), job({ id: 2, status: "queued" })])).toEqual({
      state: "waiting",
      count: 2,
    });
  });

  it("names the highest-priority running job, then the oldest, and counts the rest", () => {
    const jobs = [
      job({ id: 1, kind: "fingerprint", priority: -10 }),
      job({ id: 3, kind: "hash", priority: 10 }),
      job({ id: 2, kind: "analyze", priority: 10 }),
      job({ id: 4, kind: "scan", priority: 99, status: "queued" }),
    ];
    expect(summarize(jobs)).toEqual({ state: "running", task: "analyze", percent: null, others: 3 });
  });

  it("rounds progress down to a whole percent", () => {
    expect(summarize([job({ id: 1, progress: 0.996 })])).toMatchObject({ percent: 99 });
    expect(summarize([job({ id: 1, progress: 0.42 })])).toMatchObject({ percent: 42 });
    expect(summarize([job({ id: 1, progress: 0 })])).toMatchObject({ percent: 0 });
  });
});
