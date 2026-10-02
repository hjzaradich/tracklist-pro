import { useTranslation } from "react-i18next";
import { useShallow } from "zustand/react/shallow";
import type { JobKind } from "../bindings";
import { useActivityStore } from "./activityStore";
import { inTurn } from "./summary";

/**
 * What an empty list says while the background work that fills it is
 * still queued or running: that task, in Activity's own words ("Reading
 * tags"). `null` when none of `kinds` is under way, so the list can say
 * it's empty.
 */
export function useWaitingOn(kinds: readonly JobKind[]): string | null {
  const { t } = useTranslation("activity");
  const jobs = useActivityStore(
    useShallow((state) => Object.values(state.jobs).filter((job) => kinds.includes(job.kind))),
  );
  const [lead] = inTurn(jobs);
  return lead ? t(`task.${lead.kind}`) : null;
}
