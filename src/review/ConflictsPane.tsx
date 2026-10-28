import { useEffect, useId, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { errorMessage } from "../api/errors";
import type { Choice, ConflictRow, ValueKind } from "../bindings";
import {
  canGive,
  colourOf,
  colourValue,
  dayOf,
  isNone,
  RATING_VALUES,
  readableLocation,
  starsOf,
  startingValue,
} from "./conflictValues";
import { fieldLabel } from "./ConflictsGroup";
import { trackLabel } from "./ReviewQueue";
import { useConflicts, useResolveConflict, useResolveConflicts } from "./useConflicts";
import styles from "./ReviewComparison.module.css";
import conflictStyles from "./ConflictsPane.module.css";
import screenStyles from "./ReviewScreen.module.css";

/** The fields where the track's name, as Review picks it, is one of the values in dispute. */
const NAMED_BY_THE_FIELD = ["Name", "Artist"];

/** No color: a color field has no empty text, so "none" is its own choice. */
const BLACK = `#${"0".repeat(6)}`;

/** One side's value, shown by the kind of the field: stars, the color, a date. */
export function ConflictValue({
  field,
  kind,
  value,
}: {
  field: string;
  kind: ValueKind;
  value: string | null;
}) {
  const { t, i18n } = useTranslation("conflicts");
  if (kind === "rating") {
    const stars = starsOf(value);
    if (stars === 0) return <>{t("noRating")}</>;
    if (stars !== null) {
      return (
        <span role="img" aria-label={t("stars", { count: stars })} title={t("stars", { count: stars })}>
          {"★".repeat(stars)}
          {"☆".repeat(RATING_VALUES.length - 1 - stars)}
        </span>
      );
    }
  }
  const text = value ?? "";
  if (isNone(field, text)) return <span className={conflictStyles.muted}>{t("noValue")}</span>;
  if (kind === "colour") {
    const css = colourOf(text);
    if (css !== null) {
      // The color is data (rekordbox's), so it is set inline; its code is
      // written beside it, so the color never carries the meaning alone.
      return (
        <span className={conflictStyles.colour}>
          <span className={conflictStyles.swatch} style={{ background: css }} aria-hidden="true" />
          {text}
        </span>
      );
    }
  }
  if (kind === "date") return <span title={text}>{dayOf(text, i18n.language)}</span>;
  const shown = kind === "location" ? readableLocation(text) : text;
  return (
    <span className={conflictStyles.cut} title={shown}>
      {shown}
    </span>
  );
}

/** The field a value is given in, of the field's kind. `onChange` gets rekordbox's text for it. */
function EditField({
  kind,
  value,
  onChange,
  invalid,
}: {
  kind: ValueKind;
  value: string;
  onChange: (value: string) => void;
  invalid: boolean;
}) {
  const { t } = useTranslation("conflicts");
  const label = t("edit.label");
  const none = useId();
  switch (kind) {
    case "rating":
      return (
        <select aria-label={label} value={value} onChange={(e) => onChange(e.target.value)}>
          {RATING_VALUES.map((rating, stars) => (
            <option key={rating} value={rating}>
              {stars === 0 ? t("noRating") : t("stars", { count: stars })}
            </option>
          ))}
        </select>
      );
    case "colour":
      return (
        <span className={conflictStyles.colour}>
          <input
            type="color"
            aria-label={label}
            disabled={value === ""}
            value={colourOf(value) ?? BLACK}
            onChange={(e) => onChange(colourValue(e.target.value))}
          />
          <input
            id={none}
            type="checkbox"
            checked={value === ""}
            onChange={(e) => onChange(e.target.checked ? "" : colourValue(BLACK))}
          />
          <label htmlFor={none}>{t("edit.noColor")}</label>
        </span>
      );
    case "date":
      return (
        <input
          type="date"
          aria-label={label}
          value={value}
          aria-invalid={invalid}
          onChange={(e) => onChange(e.target.value)}
        />
      );
    default:
      return (
        <input
          type="text"
          inputMode={kind === "number" ? "numeric" : undefined}
          aria-label={label}
          value={value}
          aria-invalid={invalid}
          onChange={(e) => onChange(e.target.value)}
        />
      );
  }
}

/** One conflict: the track, the three values, and what to do about it. */
function ConflictLine({
  row,
  busy,
  onKeep,
  onEdit,
}: {
  row: ConflictRow;
  busy: boolean;
  onKeep: (side: "keepApp" | "keepRekordbox") => void;
  onEdit: (value: string) => void;
}) {
  const { t } = useTranslation("conflicts");
  const { t: review } = useTranslation("review");
  const [draft, setDraft] = useState<string | null>(null);
  const name = trackLabel(review, { recordingId: null, title: row.track.title, artist: row.track.artist });
  const fits = draft !== null && canGive(row.kind, draft);
  // The app holds no value of its own for a file location: nothing to give.
  const editable = row.kind !== "location";
  // Under a title or artist in dispute the name is one of the two values, so
  // the file's name says which track this is.
  const showFile = NAMED_BY_THE_FIELD.includes(row.field) && row.track.fileName !== null;
  return (
    <tr className={styles.row} data-conflict={row.id}>
      <td className={styles.trackCell}>
        <span className={styles.name} title={name}>
          {name}
        </span>
        {showFile && (
          <span className={conflictStyles.file} title={row.track.fileName ?? undefined}>
            {row.track.fileName}
          </span>
        )}
      </td>
      <td>
        <ConflictValue field={row.field} kind={row.kind} value={row.app} />
      </td>
      <td>
        <ConflictValue field={row.field} kind={row.kind} value={row.rekordbox} />
      </td>
      <td>
        {row.base === null ? (
          <span className={conflictStyles.muted}>{t("neverSent")}</span>
        ) : (
          <ConflictValue field={row.field} kind={row.kind} value={row.base} />
        )}
      </td>
      <td className={styles.rowAction}>
        {draft === null ? (
          <span className={conflictStyles.buttons}>
            <button type="button" className={styles.button} disabled={busy} onClick={() => onKeep("keepApp")}>
              {t("action.keepApp")}
            </button>
            <button
              type="button"
              className={styles.button}
              disabled={busy}
              onClick={() => onKeep("keepRekordbox")}
            >
              {t("action.keepRekordbox")}
            </button>
            {editable && (
              <button
                type="button"
                className={styles.button}
                disabled={busy}
                onClick={() => setDraft(startingValue(row.kind, row.app))}
              >
                {t("action.edit")}
              </button>
            )}
          </span>
        ) : (
          <span className={conflictStyles.buttons}>
            <EditField kind={row.kind} value={draft} onChange={setDraft} invalid={!fits} />
            <button
              type="button"
              className={styles.button}
              disabled={busy || !fits}
              onClick={() => onEdit(draft)}
            >
              {t("action.save")}
            </button>
            <button type="button" className={styles.button} disabled={busy} onClick={() => setDraft(null)}>
              {t("action.goBack")}
            </button>
          </span>
        )}
      </td>
    </tr>
  );
}

/**
 * The conflicts of one field (1cB-1, 1cB-2; ROADMAP 1.10, 1.12), side by side:
 * the app's value, rekordbox's, and what the field was at the last send (or
 * that it was never sent). Each row has "Keep app's", "Keep rekordbox's" and
 * "Edit" (a value given by hand, in a field of the field's kind; not for a
 * file location); under the table two buttons settle every conflict of the
 * field in one go, refused if the number of conflicts is no longer the one the
 * button shows. There is no suggestion, so no button is the filled one, and
 * Enter and the other Review keys do nothing here: each conflict is a choice
 * between values. Nothing is undone from here: the top bar's Undo takes a
 * settling back, a bulk one as a whole. After a conflict is settled the focus
 * goes to the next row, so a keyboard user can go on.
 */
export function ConflictsPane({ field }: { field: string }) {
  const { t } = useTranslation("conflicts");
  // "Show more" is the one wording every Review list uses.
  const { t: review } = useTranslation("review");
  const list = useConflicts(field);
  const one = useResolveConflict();
  const all = useResolveConflicts();
  const rows = list.data?.pages.flatMap((page) => page.items) ?? [];
  // Until the first page is here nothing is known: no count, no bulk buttons.
  const loaded = list.data !== undefined;
  const total = list.data?.pages[0]?.total ?? 0;
  const busy = one.isPending || all.isPending;
  const failure = list.error ?? one.error ?? all.error;

  // The row to focus once the settled one has left the table.
  const focusNext = useRef<number | null>(null);
  useEffect(() => {
    const id = focusNext.current;
    if (id === null) return;
    const next = document.querySelector<HTMLElement>(`[data-conflict="${id}"] button`);
    if (next) {
      next.focus();
      focusNext.current = null;
    }
  }, [list.data]);

  function resolve(row: ConflictRow, choice: Choice) {
    const at = rows.findIndex((r) => r.id === row.id);
    focusNext.current = (rows[at + 1] ?? rows[at - 1])?.id ?? null;
    // A refusal from an earlier try is no longer news.
    all.reset();
    one.mutate({ id: row.id, choice }, {
      onError: () => {
        focusNext.current = null;
      },
    });
  }

  return (
    <section className={styles.comparison} aria-labelledby={`conflicts-${field}`}>
      <h2 id={`conflicts-${field}`} className={styles.heading}>
        {fieldLabel(t, field)}{" "}
        {loaded && <span className={conflictStyles.muted}>{t("fieldCount", { count: total })}</span>}
      </h2>
      {failure && (
        <p role="alert" className={screenStyles.error}>
          {errorMessage(failure)}
        </p>
      )}
      <div className={styles.tableScroll}>
        <table className={styles.table} aria-label={t("label")}>
          <thead>
            <tr>
              <th scope="col" className={styles.trackCol}>
                {t("col.track")}
              </th>
              <th scope="col">{t("col.app")}</th>
              <th scope="col">{t("col.rekordbox")}</th>
              <th scope="col">{t("col.base")}</th>
              <th scope="col">
                <span className={conflictStyles.hidden}>{t("col.actions")}</span>
              </th>
            </tr>
          </thead>
          <tbody>
            {rows.map((row) => (
              <ConflictLine
                key={row.id}
                row={row}
                busy={busy}
                onKeep={(choice) => resolve(row, { choice })}
                onEdit={(value) => resolve(row, { choice: "edit", value })}
              />
            ))}
          </tbody>
        </table>
        {list.hasNextPage && (
          <button
            type="button"
            className={styles.button}
            disabled={list.isFetchingNextPage}
            onClick={() => void list.fetchNextPage()}
          >
            {review("queue.more")}
          </button>
        )}
      </div>
      {total > 0 && (
        <div className={styles.dock}>
          <div className={styles.actions}>
            <button
              type="button"
              className={styles.button}
              disabled={busy}
              onClick={() => {
                one.reset();
                all.mutate({ field, side: "app", expected: total });
              }}
            >
              {t("action.keepAppAll", { count: total })}
            </button>
            <button
              type="button"
              className={styles.button}
              disabled={busy}
              onClick={() => {
                one.reset();
                all.mutate({ field, side: "rekordbox", expected: total });
              }}
            >
              {t("action.keepRekordboxAll", { count: total })}
            </button>
          </div>
        </div>
      )}
    </section>
  );
}
