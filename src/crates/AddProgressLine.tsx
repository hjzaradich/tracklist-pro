import { useTranslation } from "react-i18next";
import type { AddProgressState } from "../bindings";
import styles from "./AddProgressLine.module.css";

/** Whole percent of the phase that is behind it, 0 to 100. */
export function percentOf(progress: AddProgressState): number {
  if (progress.total <= 0) return 0;
  return Math.min(100, Math.floor((progress.done / progress.total) * 100));
}

/**
 * What an add to a crate is doing and how far it has got: the phase and the
 * percent in words, and a bar for the shape of it (1cE-P2). Spans only, so
 * it sits in a line of text as well as in a dialog.
 */
export function AddProgressLine({ crate, progress }: { crate: string; progress: AddProgressState }) {
  const { t } = useTranslation("crates");
  const percent = percentOf(progress);
  const checking = progress.phase === "idle" || progress.phase === "checking";
  return (
    <span role="status" className={styles.line}>
      <span>{checking ? t("addProgress.checking", { percent }) : t("addProgress.adding", { crate, percent })}</span>
      <span className={styles.track} aria-hidden="true">
        <span className={styles.fill} style={{ width: `${percent}%` }} />
      </span>
    </span>
  );
}
