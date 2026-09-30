import { create } from "zustand";
import type { ActivitySnapshot, JobUpdate } from "../bindings";

/** At most this many jobs' updates are held while the snapshot is on its way. */
export const MAX_PENDING = 500;
/** Finished jobs are forgotten once this many jobs are remembered. */
export const MAX_SEEN = 1000;

type ActivityState = {
  /** False until the first snapshot arrives; updates wait until then. */
  synced: boolean;
  /** Queued and running jobs, by id. */
  jobs: Record<number, JobUpdate>;
  /**
   * The newest update heard for each job before the first snapshot, at most
   * {@link MAX_PENDING} jobs. Changed in place: it's bookkeeping, never
   * rendered.
   */
  pending: Map<number, JobUpdate>;
  /** Every update at or below this `seq` is already in the snapshot. */
  floor: number;
  /**
   * The `seq` of the newest update applied for each job, finished ones
   * included, so a late update can't bring a finished job back. Changed in
   * place: it's bookkeeping, never rendered. Kept to about
   * {@link MAX_SEEN} jobs (see `forgetFinished`).
   */
  seen: Map<number, number>;
  /** `seen` is trimmed when it grows past this many jobs. */
  pruneAt: number;
  /**
   * Running jobs asked to stop that haven't yet. Forgotten when the job
   * finishes, so it can't outgrow the job list.
   */
  stopping: ReadonlySet<number>;
  applySnapshot: (snapshot: ActivitySnapshot) => void;
  applyUpdates: (updates: JobUpdate[]) => void;
  /** Marks a job as asked to stop, or (`false`) not any more. */
  setStopping: (id: number, stopping: boolean) => void;
  reset: () => void;
};

const FINISHED = new Set<JobUpdate["status"]>(["done", "failed", "cancelled"]);

/**
 * Applies each update that's newer than what's known about its job.
 * Returns the new job list, or null if nothing changed. The list is copied
 * once per batch, however many updates it holds.
 */
function apply(
  state: Pick<ActivityState, "jobs" | "floor" | "seen">,
  updates: Iterable<JobUpdate>,
): Record<number, JobUpdate> | null {
  let jobs: Record<number, JobUpdate> | null = null;
  for (const update of updates) {
    if (update.seq <= state.floor) continue;
    const last = state.seen.get(update.id);
    if (last !== undefined && update.seq <= last) continue;
    state.seen.set(update.id, update.seq);
    jobs ??= { ...state.jobs };
    if (FINISHED.has(update.status)) {
      delete jobs[update.id];
    } else {
      jobs[update.id] = update;
    }
  }
  return jobs;
}

/**
 * Keeps `seen` bounded: once it holds more than {@link MAX_SEEN} jobs, the
 * finished jobs with the oldest updates are forgotten, down to three
 * quarters of it. The floor rises to the newest `seq` forgotten, so a late
 * update for a forgotten job is still ignored. The app sends updates in
 * `seq` order, so nothing at or below that `seq` is still on its way.
 *
 * With more than that many jobs still active, nothing more can go; the next
 * look is a quarter of {@link MAX_SEEN} jobs later, so a flood of jobs
 * isn't sorted on every batch. Returns the new floor and the size at which
 * to look again.
 */
function forgetFinished(
  state: Pick<ActivityState, "seen" | "floor" | "pruneAt">,
  jobs: Record<number, JobUpdate>,
): Pick<ActivityState, "floor" | "pruneAt"> {
  const { seen } = state;
  let floor = state.floor;
  if (seen.size <= state.pruneAt) return { floor, pruneAt: state.pruneAt };
  const finished = [...seen].filter(([id]) => !(id in jobs)).sort((a, b) => a[1] - b[1]);
  const target = Math.floor((MAX_SEEN * 3) / 4);
  for (const [id, seq] of finished) {
    if (seen.size <= target) break;
    seen.delete(id);
    floor = Math.max(floor, seq);
  }
  return { floor, pruneAt: Math.max(MAX_SEEN, seen.size + MAX_SEEN / 4) };
}

/** Holds `updates` until the snapshot: the newest per job, at most {@link MAX_PENDING} jobs. */
function hold(pending: Map<number, JobUpdate>, updates: JobUpdate[]) {
  for (const update of updates) {
    const held = pending.get(update.id);
    if (held === undefined || update.seq > held.seq) pending.set(update.id, update);
  }
  if (pending.size <= MAX_PENDING) return;
  // The oldest are the likeliest to be in the snapshot already.
  const oldest = [...pending.values()].sort((a, b) => a.seq - b.seq);
  for (const update of oldest.slice(0, pending.size - MAX_PENDING)) pending.delete(update.id);
}

/** `stopping` without the jobs that are no longer in `jobs`: the same set if none left. */
function stillActive(stopping: ReadonlySet<number>, jobs: Record<number, JobUpdate>) {
  for (const id of stopping) {
    if (!(id in jobs)) return new Set([...stopping].filter((kept) => kept in jobs));
  }
  return stopping;
}

const initial = () => ({
  synced: false,
  jobs: {},
  pending: new Map<number, JobUpdate>(),
  floor: 0,
  seen: new Map<number, number>(),
  pruneAt: MAX_SEEN,
  stopping: new Set<number>() as ReadonlySet<number>,
});

/**
 * Background jobs as the Activity status sees them. Starts from the app's
 * snapshot and applies each job update newer than both the snapshot and
 * the last update applied for that job, so a job never goes back to an
 * older state, even if updates for different jobs arrive out of order.
 * Updates heard before the snapshot are held, then applied on top of it.
 */
export const useActivityStore = create<ActivityState>()((set) => ({
  ...initial(),
  applySnapshot: (snapshot) =>
    set((state) => {
      const next = {
        jobs: Object.fromEntries(snapshot.jobs.map((job) => [job.id, job])),
        floor: snapshot.seq,
        seen: new Map(snapshot.jobs.map((job) => [job.id, job.seq])),
      };
      const held = [...state.pending.values()].sort((a, b) => a.seq - b.seq);
      const jobs = apply(next, held) ?? next.jobs;
      const trimmed = forgetFinished({ ...next, pruneAt: MAX_SEEN }, jobs);
      return {
        ...next,
        jobs,
        ...trimmed,
        stopping: stillActive(state.stopping, jobs),
        synced: true,
        pending: new Map(),
      };
    }),
  applyUpdates: (updates) =>
    set((state) => {
      if (!state.synced) {
        hold(state.pending, updates);
        return state;
      }
      const jobs = apply(state, updates);
      if (!jobs) return state;
      return {
        jobs,
        ...forgetFinished(state, jobs),
        stopping: stillActive(state.stopping, jobs),
      };
    }),
  setStopping: (id, stopping) =>
    set((state) => {
      if (state.stopping.has(id) === stopping) return state;
      const next = new Set(state.stopping);
      if (stopping) next.add(id);
      else next.delete(id);
      return { stopping: next };
    }),
  reset: () => set(initial()),
}));
