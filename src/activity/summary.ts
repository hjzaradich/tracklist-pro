import type { JobKind, JobUpdate } from "../bindings";

/** What the Activity status says, before it's put into words. */
export type ActivitySummary =
  | { state: "idle" }
  | { state: "waiting"; count: number }
  | {
      state: "running";
      /** The job named in the status. */
      task: JobKind;
      /** Whole percent, or null before the job reports any. */
      percent: number | null;
      /** How many other jobs are running or queued. */
      others: number;
    };

/** Higher priority first, then the oldest. */
export function byTurn(a: JobUpdate, b: JobUpdate) {
  return b.priority - a.priority || a.id - b.id;
}

/**
 * Names the running job the user most likely cares about (highest priority,
 * then oldest) and counts the rest.
 */
export function summarize(jobs: JobUpdate[]): ActivitySummary {
  if (jobs.length === 0) return { state: "idle" };
  const [lead] = jobs.filter((job) => job.status === "running").sort(byTurn);
  if (!lead) return { state: "waiting", count: jobs.length };
  return {
    state: "running",
    task: lead.kind,
    // Rounded down, so a job at 99.6% doesn't claim 100 while it's running.
    percent: lead.progress == null ? null : Math.floor(lead.progress * 100),
    others: jobs.length - 1,
  };
}

/**
 * Every job in the order workers take them: running jobs first, then
 * queued; each by priority, then the oldest.
 */
export function inTurn(jobs: JobUpdate[]): JobUpdate[] {
  const running = (job: JobUpdate) => (job.status === "running" ? 0 : 1);
  return [...jobs].sort((a, b) => running(a) - running(b) || byTurn(a, b));
}
