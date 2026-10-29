import { useId } from "react";
import { useTranslation } from "react-i18next";
import type { TFunction } from "i18next";
import type { ConflictField } from "../bindings";
import { moveInList } from "./ReviewQueue";
import styles from "./ReviewQueue.module.css";

/**
 * A field as the user knows it: "Rating", "Title". A field the app has no
 * word for yet (rekordbox has more attributes than the app names) shows
 * rekordbox's own name.
 */
export function fieldLabel(t: TFunction<"conflicts">, field: string): string {
  // Not a typed key: the field names come from the data.
  return (t as (key: string, options: object) => string)(`field.${field}`, { defaultValue: field });
}

/**
 * A field with conflicts as the user knows it. A crate membership has no
 * attribute name to show (it is a playlist path): it reads "In" and the crate's
 * name, inside the names of its folders.
 */
export function conflictLabel(t: TFunction<"conflicts">, field: ConflictField): string {
  return field.kind === "membership"
    ? t("membership.field", { name: field.crateName ?? "" })
    : fieldLabel(t, field.field);
}

/** The fields in the order the queue lists them: by the name the user sees. */
export function sortedFields(t: TFunction<"conflicts">, fields: ConflictField[]): ConflictField[] {
  return fields
    .map((f) => ({ f, label: conflictLabel(t, f) }))
    .sort((a, b) => a.label.localeCompare(b.label))
    .map(({ f }) => f);
}

/**
 * The Conflicts group of the queue (1cB-1, ROADMAP 1.12): one line per field
 * that has open conflicts, with how many; the field's conflicts are in the
 * pane beside the queue. Drawn only when a field has conflicts, like every
 * other group. Same keys as the other groups: one tab stop (the selected
 * field, else the first), Up, Down, Home and End move and select.
 */
export function ConflictsGroup({
  fields,
  selected,
  onSelect,
}: {
  /** The fields with conflicts, in the order to list them (`sortedFields`). */
  fields: ConflictField[];
  /** The field whose conflicts are shown, if one is. */
  selected: string | null;
  onSelect: (field: string) => void;
}) {
  const { t } = useTranslation("conflicts");
  const { t: review } = useTranslation("review");
  const headingId = useId();
  if (fields.length === 0) return null;
  const sorted = fields.map((f) => ({ ...f, label: conflictLabel(t, f) }));
  const total = fields.reduce((n, f) => n + f.count, 0);
  const inGroup = sorted.some((f) => f.field === selected);
  return (
    <section className={styles.group} aria-labelledby={headingId}>
      <div className={styles.heading}>
        <h2 id={headingId} className={styles.title}>
          {t("group")}
        </h2>
        <span className={styles.count}>{review("queue.left", { count: total })}</span>
      </div>
      <ul
        role="listbox"
        aria-labelledby={headingId}
        className={styles.list}
        onKeyDown={(event) =>
          moveInList(event, (to) => {
            const field = sorted[to];
            if (field) onSelect(field.field);
          })
        }
      >
        {sorted.map((f, n) => (
          <li
            key={f.field}
            role="option"
            aria-selected={f.field === selected}
            tabIndex={f.field === selected || (!inGroup && n === 0) ? 0 : -1}
            className={styles.item}
            onClick={() => onSelect(f.field)}
          >
            <span className={styles.track}>{f.label}</span>
            <span className={styles.kind}>{t("fieldCount", { count: f.count })}</span>
          </li>
        ))}
      </ul>
    </section>
  );
}
