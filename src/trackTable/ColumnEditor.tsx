import { useEffect, useId, useLayoutEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  ALWAYS_SHOWN,
  hiddenColumns,
  hideColumn,
  isDefault,
  moveColumn,
  showColumn,
} from "./columnChoice";
import type { ColumnId, TableScreen } from "./columns";
import styles from "./ColumnEditor.module.css";

/** Which button of a column to put the focus on once the lists have changed. */
type Focus = { column: ColumnId; button: "up" | "down" | "hide" | "show" };

/**
 * The column editor (1cA-5; ROADMAP 1.11 "The track table"): a "Columns"
 * button above the table that opens a small panel listing the columns shown,
 * in order, and the ones that are not. Each shown column has Move up, Move
 * down and Hide; each hidden one has Show. The table follows every change
 * at once, so there is nothing to confirm, and "Reset to default" brings
 * back the screen's own columns. The Title can't be hidden.
 *
 * Everything is a button, so the keyboard reaches all of it; there is no
 * dragging. Escape or a click outside closes the panel, and the focus goes
 * back to the "Columns" button.
 */
export function ColumnEditor({
  screen,
  columns,
  onChange,
  onReset,
}: {
  screen: TableScreen;
  /** The columns shown, in order. */
  columns: readonly ColumnId[];
  onChange: (columns: readonly ColumnId[]) => void;
  onReset: () => void;
}) {
  const { t } = useTranslation("trackTable");
  const [open, setOpen] = useState(false);
  const panelId = useId();
  const root = useRef<HTMLDivElement>(null);
  const opener = useRef<HTMLButtonElement>(null);
  const panel = useRef<HTMLDivElement>(null);
  const focus = useRef<Focus | null>(null);

  // A click anywhere else closes the panel.
  useEffect(() => {
    if (!open) return;
    const away = (event: MouseEvent) => {
      if (root.current && !root.current.contains(event.target as Node)) setOpen(false);
    };
    document.addEventListener("mousedown", away);
    return () => document.removeEventListener("mousedown", away);
  }, [open]);

  // Opening puts the focus in the panel.
  useLayoutEffect(() => {
    if (open) panel.current?.focus();
  }, [open]);

  // A button that moved or went keeps the focus.
  useLayoutEffect(() => {
    if (!open) return;
    const wanted = focus.current;
    focus.current = null;
    if (wanted === null) return;
    const buttons = panel.current?.querySelectorAll<HTMLButtonElement>(
      `[data-column="${wanted.column}"] button`,
    );
    const named = (kind: Focus["button"]) =>
      Array.from(buttons ?? []).find((button) => button.dataset.button === kind);
    // Move up on the first row is greyed out: go to the other way instead.
    const fallback = wanted.button === "up" ? "down" : wanted.button === "down" ? "up" : "show";
    const target = [named(wanted.button), named(fallback)].find((b) => b && !b.disabled);
    target?.focus();
  });

  const close = () => {
    setOpen(false);
    opener.current?.focus();
  };

  const change = (next: readonly ColumnId[], to: Focus) => {
    focus.current = to;
    onChange(next);
  };

  const hidden = hiddenColumns(columns);
  const name = (id: ColumnId) => t(`columns.${id}`);

  return (
    <div className={styles.root} ref={root}>
      <button
        ref={opener}
        type="button"
        className={styles.opener}
        aria-haspopup="dialog"
        aria-expanded={open}
        aria-controls={open ? panelId : undefined}
        onClick={() => setOpen(!open)}
      >
        {t("editor.open")}
      </button>
      {open && (
        <div
          id={panelId}
          ref={panel}
          role="dialog"
          aria-label={t("editor.title")}
          tabIndex={-1}
          className={styles.panel}
          onKeyDown={(event) => {
            if (event.key === "Escape") {
              event.stopPropagation();
              close();
            }
          }}
        >
          <p className={styles.help}>
            {t(screen === "crate" ? "editor.helpCrate" : "editor.help")}
          </p>

          <h3 className={styles.heading}>{t("editor.shown")}</h3>
          <ul className={styles.list} aria-label={t("editor.shown")}>
            {columns.map((id, at) => (
              <li key={id} className={styles.item} data-column={id}>
                <span className={styles.name}>{name(id)}</span>
                {id === ALWAYS_SHOWN && (
                  <span className={styles.note}>{t("editor.alwaysShown")}</span>
                )}
                <span className={styles.buttons}>
                  <button
                    type="button"
                    className={styles.small}
                    data-button="up"
                    disabled={at === 0}
                    aria-label={t("editor.moveUp", { column: name(id) })}
                    title={t("editor.moveUp", { column: name(id) })}
                    onClick={() => change(moveColumn(columns, id, -1), { column: id, button: "up" })}
                  >
                    <Arrow up />
                  </button>
                  <button
                    type="button"
                    className={styles.small}
                    data-button="down"
                    disabled={at === columns.length - 1}
                    aria-label={t("editor.moveDown", { column: name(id) })}
                    title={t("editor.moveDown", { column: name(id) })}
                    onClick={() => change(moveColumn(columns, id, 1), { column: id, button: "down" })}
                  >
                    <Arrow up={false} />
                  </button>
                  <button
                    type="button"
                    className={styles.small}
                    data-button="hide"
                    disabled={id === ALWAYS_SHOWN}
                    aria-label={t("editor.hide", { column: name(id) })}
                    onClick={() => change(hideColumn(columns, id), { column: id, button: "show" })}
                  >
                    {t("editor.hideShort")}
                  </button>
                </span>
              </li>
            ))}
          </ul>

          <h3 className={styles.heading}>{t("editor.hidden")}</h3>
          {hidden.length === 0 ? (
            <p className={styles.empty}>{t("editor.noneHidden")}</p>
          ) : (
            <ul className={styles.list} aria-label={t("editor.hidden")}>
              {hidden.map((id) => (
                <li key={id} className={styles.item} data-column={id}>
                  <span className={styles.name}>{name(id)}</span>
                  <span className={styles.buttons}>
                    <button
                      type="button"
                      className={styles.small}
                      data-button="show"
                      aria-label={t("editor.show", { column: name(id) })}
                      onClick={() => change(showColumn(columns, id), { column: id, button: "hide" })}
                    >
                      {t("editor.showShort")}
                    </button>
                  </span>
                </li>
              ))}
            </ul>
          )}

          <div className={styles.foot}>
            <button
              type="button"
              className={styles.quiet}
              disabled={isDefault(screen, columns)}
              onClick={() => {
                onReset();
                panel.current?.focus();
              }}
            >
              {t("editor.reset")}
            </button>
            <button type="button" className={styles.done} onClick={close}>
              {t("editor.done")}
            </button>
          </div>
        </div>
      )}
    </div>
  );
}

/** An arrow up or down. Decorative: the button's name says what it does. */
function Arrow({ up }: { up: boolean }) {
  return (
    <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true" focusable="false">
      <path d={up ? "M1 7l4-4 4 4z" : "M1 3l4 4 4-4z"} fill="currentColor" />
    </svg>
  );
}
