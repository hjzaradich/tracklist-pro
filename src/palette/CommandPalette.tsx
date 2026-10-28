import { useEffect, useId, useMemo, useRef, useState, type KeyboardEvent } from "react";
import { useTranslation } from "react-i18next";
import styles from "./CommandPalette.module.css";
import { filterActions } from "./match";
import { listedActions, usePaletteRegistry } from "./registry";
import { useAppActions } from "./useAppActions";

/** A question waiting for an answer has the keyboard: the palette doesn't open over it. */
const QUESTION = '[role="alertdialog"], dialog[open]';

/**
 * The command palette (1cB-7, ROADMAP 1.11): Ctrl+K opens a text box over a
 * list of the actions in the registry, filtered as you type. Enter runs the
 * highlighted one, the arrow keys move the highlight, Escape closes it and
 * gives the focus back to where it was. It is one dialog with a combobox and
 * its listbox; the focus stays in the box while it is open.
 *
 * Ctrl+K opens it from anywhere, a text field included. While it is open it
 * is a `dialog`, which the other keyboard shortcuts already stand aside for.
 * The action runs after the palette has closed and the focus is back, so an
 * action that moves the focus keeps it.
 */
export function CommandPalette() {
  const { t } = useTranslation("palette");
  useAppActions();
  const sources = usePaletteRegistry((state) => state.sources);
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const [chosen, setChosen] = useState<string | null>(null);
  const input = useRef<HTMLInputElement>(null);
  const comesFrom = useRef<Element | null>(null);
  const toRun = useRef<(() => void) | null>(null);
  const listId = useId();
  const dialogId = useId();

  const shown = useMemo(() => filterActions(listedActions(sources), query), [sources, query]);
  // The highlight follows the action, not its place: a list that changes
  // under it keeps it, and falls back to the first row.
  const highlighted = shown.find((action) => action.id === chosen) ?? shown[0] ?? null;
  const highlightedId = highlighted?.id;

  useEffect(() => {
    const onKeyDown = (event: globalThis.KeyboardEvent) => {
      if (!event.ctrlKey || event.shiftKey || event.altKey || event.metaKey) return;
      if (event.key.toLowerCase() !== "k") return;
      if (document.querySelector(QUESTION) !== null) return;
      event.preventDefault();
      if (event.repeat) return;
      if (open) {
        setOpen(false);
      } else {
        comesFrom.current = document.activeElement;
        setQuery("");
        setChosen(null);
        setOpen(true);
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [open]);

  // Opening puts the focus in the box. Closing gives it back, then runs the
  // action that was chosen, if any.
  useEffect(() => {
    if (!open) return;
    input.current?.focus();
    return () => {
      const back = comesFrom.current;
      comesFrom.current = null;
      if (back instanceof HTMLElement) back.focus();
      const run = toRun.current;
      toRun.current = null;
      run?.();
    };
  }, [open]);

  useEffect(() => {
    if (highlightedId === undefined) return;
    document.getElementById(`${dialogId}-${highlightedId}`)?.scrollIntoView?.({ block: "nearest" });
  }, [highlightedId, dialogId]);

  if (!open) return null;

  const move = (step: number) => {
    if (shown.length === 0) return;
    const at = highlighted === null ? 0 : shown.indexOf(highlighted);
    setChosen(shown[(at + step + shown.length) % shown.length]?.id ?? null);
  };
  const runAction = (action: { run: () => void }) => {
    toRun.current = action.run;
    setOpen(false);
  };

  const onKeyDown = (event: KeyboardEvent) => {
    let handled = true;
    switch (event.key) {
      case "ArrowDown":
        move(1);
        break;
      case "ArrowUp":
        move(-1);
        break;
      case "Home":
        setChosen(shown[0]?.id ?? null);
        break;
      case "End":
        setChosen(shown.at(-1)?.id ?? null);
        break;
      case "Enter":
        // A held Enter runs once: the repeats would land on the restored focus.
        if (highlighted !== null && !event.repeat) runAction(highlighted);
        break;
      case "Escape":
        setOpen(false);
        break;
      case "Tab":
        // The box is the only stop: the focus stays in the dialog.
        break;
      default:
        handled = false;
    }
    if (handled) {
      event.preventDefault();
      // Nothing behind the palette answers a key the palette used.
      event.stopPropagation();
    }
  };

  return (
    // A click anywhere outside the panel closes it, like Escape.
    <div className={styles.backdrop} onMouseDown={() => setOpen(false)}>
      <div
        role="dialog"
        aria-modal="true"
        aria-label={t("title")}
        className={styles.panel}
        onMouseDown={(event) => {
          event.stopPropagation();
          // A click on the panel (the "No matching commands" line, a gap)
          // must not take the focus out of the box, where the keys are heard.
          if (event.target !== input.current) event.preventDefault();
        }}
        onKeyDown={onKeyDown}
      >
        <input
          ref={input}
          type="text"
          role="combobox"
          aria-label={t("input.label")}
          aria-expanded="true"
          aria-controls={listId}
          aria-autocomplete="list"
          aria-activedescendant={highlightedId === undefined ? undefined : `${dialogId}-${highlightedId}`}
          placeholder={t("input.placeholder")}
          autoComplete="off"
          spellCheck={false}
          className={styles.input}
          value={query}
          onChange={(event) => {
            setQuery(event.target.value);
            setChosen(null);
          }}
        />
        <ul
          id={listId}
          role="listbox"
          aria-label={t("list.label")}
          className={styles.list}
          // A click on a row must not take the focus out of the box.
          onMouseDown={(event) => event.preventDefault()}
        >
          {shown.map((action) => (
            <li
              key={action.id}
              id={`${dialogId}-${action.id}`}
              role="option"
              aria-selected={action.id === highlightedId}
              className={styles.option}
              onMouseMove={() => setChosen(action.id)}
              onClick={() => runAction(action)}
            >
              <span className={styles.label}>{action.label}</span>
              {action.shortcut !== undefined && <kbd className={styles.shortcut}>{action.shortcut}</kbd>}
            </li>
          ))}
        </ul>
        {shown.length === 0 && (
          <p role="status" className={styles.empty}>
            {t("empty")}
          </p>
        )}
      </div>
    </div>
  );
}
