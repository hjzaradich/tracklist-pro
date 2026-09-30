import { beforeEach, describe, expect, it } from "vitest";
import type { JobUpdate } from "../bindings";
import { MAX_PENDING, MAX_SEEN, useActivityStore } from "./activityStore";

function update(
  seq: number,
  id: number,
  status: JobUpdate["status"],
  progress: number | null = null,
): JobUpdate {
  return { seq, id, kind: "hash", status, progress, priority: 0 };
}

const store = () => useActivityStore.getState();
const jobs = () => Object.values(store().jobs).map((j) => [j.id, j.status, j.progress]);

describe("activity store", () => {
  beforeEach(() => store().reset());

  it("starts from the snapshot and applies newer updates", () => {
    store().applySnapshot({ seq: 5, jobs: [update(5, 1, "running", 0.2)] });
    store().applyUpdates([update(6, 1, "running", 0.3), update(7, 2, "queued")]);
    expect(jobs()).toEqual([
      [1, "running", 0.3],
      [2, "queued", null],
    ]);
  });

  it("drops finished jobs", () => {
    store().applySnapshot({
      seq: 1,
      jobs: [update(1, 1, "running"), update(1, 2, "running"), update(1, 3, "queued")],
    });
    store().applyUpdates([update(2, 1, "done", 1), update(3, 2, "failed")]);
    store().applyUpdates([update(4, 3, "cancelled")]);
    expect(jobs()).toEqual([]);
  });

  it("holds updates heard before the snapshot and applies only the newer ones on top", () => {
    // Heard while the snapshot was on its way: seq 3 is already in it.
    store().applyUpdates([update(3, 1, "running", 0.1)]);
    store().applyUpdates([update(5, 1, "running", 0.5)]);
    expect(jobs()).toEqual([]);
    store().applySnapshot({ seq: 4, jobs: [update(4, 1, "running", 0.4)] });
    expect(jobs()).toEqual([[1, "running", 0.5]]);
  });

  it("never lets an older update undo a newer one", () => {
    store().applySnapshot({ seq: 10, jobs: [] });
    // Job 1 finished at seq 10, which the snapshot already reflects.
    store().applyUpdates([update(9, 1, "running", 0.9)]);
    expect(jobs()).toEqual([]);
    store().applyUpdates([update(12, 2, "running", 0.5)]);
    store().applyUpdates([update(11, 2, "running", 0.1)]);
    expect(jobs()).toEqual([[2, "running", 0.5]]);
  });

  it("applies updates for different jobs that arrive out of order", () => {
    store().applySnapshot({ seq: 0, jobs: [] });
    // Job 2's update (seq 11) overtakes job 1's (seq 10).
    store().applyUpdates([update(11, 2, "running", 0.2)]);
    store().applyUpdates([update(10, 1, "running", 0.1)]);
    expect(jobs()).toEqual([
      [1, "running", 0.1],
      [2, "running", 0.2],
    ]);
    // Job 1 finishes, then a late update for it turns up: it stays finished.
    store().applyUpdates([update(13, 1, "done", 1)]);
    store().applyUpdates([update(12, 1, "running", 0.9)]);
    expect(jobs()).toEqual([[2, "running", 0.2]]);
  });

  it("keeps the same job list when a batch changes nothing, so nothing re-renders", () => {
    store().applySnapshot({ seq: 5, jobs: [update(5, 1, "running")] });
    const before = store().jobs;
    store().applyUpdates([update(4, 1, "running", 0.9)]);
    expect(store().jobs).toBe(before);
  });

  it("stays fast with tens of thousands of updates", () => {
    store().applySnapshot({ seq: 0, jobs: [] });
    const start = performance.now();
    // 20,000 jobs queued then started, in batches of 100 as the app sends
    // them, then all finished in one batch. Copying the job list for every
    // update instead of once per batch takes several times the limit.
    let seq = 0;
    const all: JobUpdate[] = [];
    for (let id = 1; id <= 20_000; id++) all.push(update(++seq, id, "queued"));
    for (let id = 1; id <= 20_000; id++) all.push(update(++seq, id, "running", 0.5));
    for (let i = 0; i < all.length; i += 100) store().applyUpdates(all.slice(i, i + 100));
    expect(Object.keys(store().jobs)).toHaveLength(20_000);
    const done = Array.from({ length: 20_000 }, (_, i) => update(++seq, i + 1, "done", 1));
    store().applyUpdates(done);
    expect(jobs()).toEqual([]);
    expect(performance.now() - start).toBeLessThan(1_000);
  });

  it("holds at most MAX_PENDING jobs' updates before the snapshot, newest per job", () => {
    const heard: JobUpdate[] = [];
    let seq = 0;
    for (let id = 1; id <= MAX_PENDING + 200; id++) heard.push(update(++seq, id, "queued"));
    // A second update for job 1 replaces its first rather than adding one.
    heard.push(update(seq + 1, 1, "running", 0.5));
    store().applyUpdates(heard);
    expect(store().pending.size).toBe(MAX_PENDING);
    // The oldest were let go; job 1's newest update was kept.
    expect(store().pending.get(1)?.progress).toBe(0.5);
    expect(store().pending.has(2)).toBe(false);

    store().applySnapshot({ seq: 0, jobs: [] });
    expect(store().pending.size).toBe(0);
    expect(store().jobs[1]?.progress).toBe(0.5);
    expect(Object.keys(store().jobs)).toHaveLength(MAX_PENDING);
  });

  it("forgets old finished jobs, so its bookkeeping stays bounded", () => {
    store().applySnapshot({ seq: 0, jobs: [] });
    let seq = 0;
    for (let id = 1; id <= 20 * MAX_SEEN; id++) {
      store().applyUpdates([update(++seq, id, "running")]);
      store().applyUpdates([update(++seq, id, "done", 1)]);
    }
    expect(store().seen.size).toBeLessThanOrEqual(MAX_SEEN);
    expect(jobs()).toEqual([]);
    // A late update for a forgotten job still can't bring it back.
    store().applyUpdates([update(3, 2, "running", 0.9)]);
    expect(jobs()).toEqual([]);
  });

  it("never forgets a job that's still running or queued", () => {
    store().applySnapshot({ seq: 0, jobs: [] });
    let seq = 0;
    const active = 2 * MAX_SEEN;
    const all: JobUpdate[] = [];
    for (let id = 1; id <= active; id++) all.push(update(++seq, id, "queued"));
    store().applyUpdates(all);
    for (let id = active + 1; id <= active + MAX_SEEN; id++) {
      store().applyUpdates([update(++seq, id, "done", 1)]);
    }
    expect(Object.keys(store().jobs)).toHaveLength(active);
    for (let id = 1; id <= active; id++) expect(store().seen.has(id)).toBe(true);
    // Each still takes its updates.
    store().applyUpdates([update(seq + 1, 1, "running", 0.4)]);
    expect(store().jobs[1]?.progress).toBe(0.4);
  });
});
