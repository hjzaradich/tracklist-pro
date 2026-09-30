import type { TFunction } from "i18next";
import { useEffect, useId, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { useShallow } from "zustand/react/shallow";
import { commands, type JobUpdate } from "../bindings";
import { useActivityStore } from "./activityStore";
import styles from "./Activity.module.css";
import { inTurn, summarize, type ActivitySummary } from "./summary";
import { useActivitySync } from "./useActivitySync";

/**
 * The top-bar Activity status: what background work is doing (0E-3).
 * Clicking it opens a popover listing every running and queued job, each
 * with Cancel (1aA-11).
 */
export function Activity() {
  useActivitySync();
  const { t } = useTranslation("activity");
  const jobs = useActivityStore(useShallow((state) => Object.values(state.jobs)));
  const summary = summarize(jobs);
  const [open, setOpen] = useState(false);
  const root = useRef<HTMLDivElement>(null);
  const trigger = useRef<HTMLButtonElement>(null);
  const panelId = useId();

  // Escape or a click anywhere else closes it.
  useEffect(() => {
    if (!open) return;
    const onKey = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      setOpen(false);
      trigger.current?.focus();
    };
    const onPointer = (event: PointerEvent) => {
      if (!root.current?.contains(event.target as Node)) setOpen(false);
    };
    document.addEventListener("keydown", onKey);
    document.addEventListener("pointerdown", onPointer);
    return () => {
      document.removeEventListener("keydown", onKey);
      document.removeEventListener("pointerdown", onPointer);
    };
  }, [open]);

  return (
    <div ref={root} className={styles.root}>
      <button
        ref={trigger}
        type="button"
        className={styles.trigger}
        aria-expanded={open}
        aria-controls={open ? panelId : undefined}
        onClick={() => setOpen((was) => !was)}
      >
        <span
          role="status"
          aria-label={t("label")}
          data-state={summary.state}
          className={styles.activity}
        >
          {describe(summary, t)}
        </span>
      </button>
      {open && <ActivityPanel id={panelId} jobs={jobs} />}
    </div>
  );
}

/** The summary in words. */
function describe(summary: ActivitySummary, t: TFunction<"activity">): string {
  switch (summary.state) {
    case "idle":
      return t("idle");
    case "waiting":
      return t("waiting", { count: summary.count });
    case "running": {
      const task = t(`task.${summary.task}`);
      const status =
        summary.percent == null ? task : t("percent", { task, percent: summary.percent });
      return summary.others === 0 ? status : t("andMore", { summary: status, count: summary.others });
    }
  }
}

/**
 * Every running and queued job, in the order workers take them: the panel
 * the Activity status shows and hides (a disclosure, not a dialog).
 */
function ActivityPanel({ id, jobs }: { id: string; jobs: JobUpdate[] }) {
  const { t } = useTranslation("activity");
  // Kept in the store, so closing and reopening doesn't forget it.
  const stopping = useActivityStore((state) => state.stopping);

  const cancel = async (job: JobUpdate) => {
    const { setStopping } = useActivityStore.getState();
    setStopping(job.id, true);
    const outcome = await commands.cancelJob(job.id).catch(() => null);
    // A queued job is gone at once (its update removes it); a running one
    // shows it's stopping until it does. If the app couldn't cancel it,
    // Cancel works again.
    if (outcome?.status !== "ok" || outcome.data !== "stopping") {
      setStopping(job.id, false);
    }
  };

  return (
    <div id={id} className={styles.panel}>
      {jobs.length === 0 ? (
        <p className={styles.empty}>{t("idle")}</p>
      ) : (
        <ul className={styles.list}>
          {inTurn(jobs).map((job) => (
            <JobRow
              key={job.id}
              job={job}
              stopping={stopping.has(job.id)}
              onCancel={() => void cancel(job)}
            />
          ))}
        </ul>
      )}
    </div>
  );
}

function JobRow({
  job,
  stopping,
  onCancel,
}: {
  job: JobUpdate;
  stopping: boolean;
  onCancel: () => void;
}) {
  const { t } = useTranslation("activity");
  const textId = useId();
  const task = t(`task.${job.kind}`);
  let text: string;
  if (stopping) {
    text = t("stopping", { task });
  } else if (job.status === "queued") {
    text = t("queued", { task });
  } else if (job.progress == null) {
    text = task;
  } else {
    text = t("percent", { task, percent: Math.floor(job.progress * 100) });
  }

  return (
    <li className={styles.row} data-status={job.status}>
      <span id={textId} className={styles.rowText}>
        {text}
      </span>
      <button
        type="button"
        className={styles.cancel}
        aria-describedby={textId}
        disabled={stopping}
        onClick={onCancel}
      >
        {t("cancel")}
      </button>
    </li>
  );
}
