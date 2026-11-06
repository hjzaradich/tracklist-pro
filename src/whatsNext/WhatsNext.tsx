import { useId, useState } from "react";
import { useTranslation } from "react-i18next";
import { errorMessage } from "../api/errors";
import type { Direction, NextSettings, NextWeights, Signal, TableRow, Tolerance } from "../bindings";
import { useKeyNotation } from "../keys/useKeyNotation";
import { reasonText } from "./reasonText";
import { useNext, useNextSettings, useSetNextSettings } from "./useNext";
import styles from "./WhatsNext.module.css";

const DIRECTIONS: readonly Direction[] = ["up", "same", "down"];
const TOLERANCES: readonly Tolerance[] = ["tight", "wide"];
const WEIGHTS: readonly (keyof NextWeights)[] = ["key", "bpm", "energy", "played"];
const MOST_WEIGHT = 10;

/** The weights and the tolerance, shown and changeable; a change applies at once and is not an operation. */
function WeightsPanel() {
  const { t } = useTranslation("next");
  const settings = useNextSettings();
  const save = useSetNextSettings();
  const id = useId();
  const stored = settings.data;
  if (stored === undefined) return null;

  const change = (next: NextSettings) => save.mutate(next);
  return (
    <details className={styles.weights}>
      <summary>{t("weights.title")}</summary>
      <div className={styles.weightsBody}>
        <p className={styles.quiet}>{t("weights.hint")}</p>
        {WEIGHTS.map((name) => (
          <label key={name} className={styles.weight} htmlFor={`${id}-${name}`}>
            {t(`weights.${name}`)}
            <input
              id={`${id}-${name}`}
              type="number"
              min={0}
              max={MOST_WEIGHT}
              step={1}
              defaultValue={stored.weights[name]}
              onChange={(event) => {
                const value = event.target.valueAsNumber;
                if (Number.isInteger(value) && value >= 0 && value <= MOST_WEIGHT) {
                  change({ ...stored, weights: { ...stored.weights, [name]: value } });
                }
              }}
            />
          </label>
        ))}
        <label className={styles.weight} htmlFor={`${id}-tolerance`}>
          {t("weights.tolerance.label")}
          <select
            id={`${id}-tolerance`}
            value={stored.tolerance}
            onChange={(event) => change({ ...stored, tolerance: event.target.value as Tolerance })}
          >
            {TOLERANCES.map((tolerance) => (
              <option key={tolerance} value={tolerance}>
                {t(`weights.tolerance.${tolerance}`)}
              </option>
            ))}
          </select>
        </label>
        <p className={styles.quiet}>{t("weights.tolerance.hint")}</p>
        {save.isError && (
          <p role="alert" className={styles.error}>
            {errorMessage(save.error)}
          </p>
        )}
      </div>
    </details>
  );
}

/**
 * "What's next" in the Details panel (ROADMAP 3.4): the ten Library tracks
 * that best follow the selected one, each with its top reasons, the way the
 * energy should go, and the weights and tolerance. A plain list, no score.
 *
 * Give it a `key` per track, so one track's answer doesn't stay under the
 * next.
 */
export function WhatsNext({ row }: { row: TableRow }) {
  const { t } = useTranslation("next");
  const notation = useKeyNotation();
  const [direction, setDirection] = useState<Direction>("same");
  const next = useNext(row.recordingId, direction);
  const headingId = useId();
  const name = useId();

  return (
    <section aria-labelledby={headingId} className={styles.section}>
      <h3 id={headingId} className={styles.heading}>
        {t("title")}
      </h3>
      <fieldset className={styles.directions}>
        <legend>{t("direction.label")}</legend>
        {DIRECTIONS.map((option) => (
          <label key={option} className={styles.direction}>
            <input
              type="radio"
              name={name}
              checked={direction === option}
              onChange={() => setDirection(option)}
            />
            {t(`direction.${option}`)}
          </label>
        ))}
      </fieldset>
      {next.isError && (
        <p role="alert" className={styles.error}>
          {errorMessage(next.error)}
        </p>
      )}
      {next.isPending && <p className={styles.quiet}>{t("loading")}</p>}
      {next.data !== undefined && (
        <>
          {next.data.results.length === 0 ? (
            <p className={styles.quiet}>{t("none")}</p>
          ) : (
            <ol className={styles.results}>
              {next.data.results.map(({ row: result, reasons }) => (
                <li key={result.recordingId}>
                  <span className={styles.track}>
                    {[result.artist, result.title].filter((part) => part !== null && part !== "").join(" - ")}
                  </span>
                  <span className={styles.reasons}>
                    {reasons.map((reason) => reasonText(reason, t, notation)).join(t("reason.separator"))}
                  </span>
                </li>
              ))}
            </ol>
          )}
          {next.data.ignored.length > 0 && (
            <p className={styles.quiet}>
              {t("ignored.label", {
                signals: next.data.ignored.map((signal: Signal) => t(`signal.${signal}`)).join(t("ignored.separator")),
              })}
            </p>
          )}
          <p className={styles.quiet}>{t("considered", { count: next.data.considered })}</p>
        </>
      )}
      <WeightsPanel />
    </section>
  );
}
