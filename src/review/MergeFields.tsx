import { useId } from "react";
import { useTranslation } from "react-i18next";
import type { MergeField } from "../bindings";
import { ConflictValue } from "./ConflictsPane";
import { fieldLabel } from "./ConflictsGroup";
import styles from "./ReviewComparison.module.css";
import conflictStyles from "./ConflictsPane.module.css";

/** The track whose value is picked for a field: the user's pick, else the default. */
export function pickedFor(field: MergeField, picks: Record<string, number>): number {
  return picks[field.field] ?? field.chosen;
}

/**
 * The values a merge of two Library tracks asks about (1cB-20; ROADMAP 1.12):
 * one row per field where the tracks differ and one value is the user's (their
 * edits, and rekordbox's rating, color and comments), the same side-by-side
 * values as the conflicts screen. The track that stays is the first column and
 * its value is picked at first. A track with no value for the field shows
 * "No value" and can't be picked. Nothing is tinted: the picked value is the
 * checked radio, and the radios of one row are one group the arrow keys move
 * through. `names` gives each track its name, by recording id.
 */
export function MergeFields({
  fields,
  names,
  picks,
  disabled,
  onPick,
}: {
  fields: MergeField[];
  names: ReadonlyMap<number, string>;
  /** The track picked per field, where the user picked one. */
  picks: Record<string, number>;
  disabled: boolean;
  onPick: (field: string, recordingId: number) => void;
}) {
  const { t } = useTranslation("review");
  const { t: conflicts } = useTranslation("conflicts");
  const headingId = useId();
  const group = useId();
  const first = fields[0];
  if (first === undefined) return null;
  return (
    <div role="group" aria-labelledby={headingId}>
      <p id={headingId} className={styles.doubtLabel}>
        {t("same.values.heading")}
      </p>
      <div className={conflictStyles.scroll}>
        <table className={styles.table}>
          <thead>
            <tr>
              <th scope="col">{t("same.values.field")}</th>
              {first.values.map((v, at) => (
                <th key={v.recordingId} scope="col">
                  <span className={styles.name}>{names.get(v.recordingId) ?? t("item.untitled")}</span>
                  {at === 0 && (
                    <span className={styles.marks}>
                      <span className={styles.mark}>{t("same.values.stays")}</span>
                    </span>
                  )}
                </th>
              ))}
            </tr>
          </thead>
          <tbody>
            {fields.map((field) => {
              const label = fieldLabel(conflicts, field.field);
              const picked = pickedFor(field, picks);
              return (
                <tr key={field.field} className={styles.row}>
                  <th scope="row" className={styles.rowHead}>{label}</th>
                  {field.values.map((v) => (
                    <td key={v.recordingId}>
                      {v.value === null ? (
                        <ConflictValue field={field.field} kind={field.kind} value={null} />
                      ) : (
                        <label className={conflictStyles.pick}>
                          <input
                            type="radio"
                            name={`${group}-${field.field}`}
                            aria-label={t("same.values.use", {
                              name: names.get(v.recordingId) ?? t("item.untitled"),
                              field: label,
                            })}
                            checked={picked === v.recordingId}
                            disabled={disabled}
                            onChange={() => onPick(field.field, v.recordingId)}
                          />
                          <ConflictValue field={field.field} kind={field.kind} value={v.value} />
                        </label>
                      )}
                    </td>
                  ))}
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>
    </div>
  );
}
