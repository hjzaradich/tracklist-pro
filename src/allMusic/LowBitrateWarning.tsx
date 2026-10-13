import { useEffect, useId, useRef } from "react";
import { useTranslation } from "react-i18next";
import styles from "./LowBitrateWarning.module.css";

/**
 * Warns before a low-bitrate file joins the Library (1bE-8). Nothing is
 * added until the user chooses "Add anyway"; "Cancel" (or Escape) changes
 * nothing at all. The rule that asks for this lives in the backend: this is
 * only how its answer is shown.
 */
export function LowBitrateWarning({
  title,
  kbps,
  onAddAnyway,
  onCancel,
}: {
  title: string;
  kbps: number;
  onAddAnyway: () => void;
  onCancel: () => void;
}) {
  const { t } = useTranslation("allMusic");
  // The dialog takes focus, on the safe choice.
  const cancel = useRef<HTMLButtonElement>(null);
  useEffect(() => cancel.current?.focus(), []);
  const titleId = useId();
  const detailId = useId();
  return (
    <div
      role="alertdialog"
      aria-labelledby={titleId}
      aria-describedby={detailId}
      className={styles.confirm}
      onKeyDown={(event) => {
        if (event.key === "Escape") onCancel();
      }}
    >
      <p id={titleId} className={styles.title}>
        {t("lowBitrate.title", { title })}
      </p>
      <p id={detailId} className={styles.detail}>
        {t("lowBitrate.detail", { kbps })}
      </p>
      <div className={styles.actions}>
        <button type="button" className={styles.button} onClick={onAddAnyway}>
          {t("lowBitrate.addAnyway")}
        </button>
        <button ref={cancel} type="button" className={styles.button} onClick={onCancel}>
          {t("lowBitrate.cancel")}
        </button>
      </div>
    </div>
  );
}
