import { useEffect, useId, useMemo, useRef, useState, type KeyboardEvent } from "react";
import { useTranslation } from "react-i18next";
import { errorMessage } from "../api/errors";
import { matches } from "../palette/match";
// The same plain panel as "Add to crate".
import styles from "../crates/AddToCrateDialog.module.css";
import { useApplyTag, useVocab } from "./useVocab";

/**
 * The tag picker, opened by the T key (3A-T; ROADMAP 3.1): a text box over the
 * list of tags, each named by its group ("Vibe › Warm"). Typing finds tags the
 * way the command palette finds actions. Enter gives the highlighted tag to the
 * selected tracks, in one operation (one Undo). Escape, a click outside and
 * "Go back" close it. Taking a tag off is in the Details panel.
 *
 * `recordings` are the tracks (`TableRow.recordingId`) as they were when it
 * opened.
 */
export function TagPicker({ recordings, onClose }: { recordings: readonly number[]; onClose: () => void }) {
  const { t } = useTranslation("vocab");
  const vocab = useVocab().data;
  const apply = useApplyTag();
  const [query, setQuery] = useState("");
  const [chosen, setChosen] = useState<number | null>(null);
  const [error, setError] = useState<string | null>(null);
  const input = useRef<HTMLInputElement>(null);
  const back = useRef<HTMLButtonElement>(null);
  const listId = useId();
  const dialogId = useId();
  const titleId = useId();

  // Focus goes into the box, and back to where it was when the picker closes.
  useEffect(() => {
    const comesFrom = document.activeElement;
    input.current?.focus();
    return () => {
      if (comesFrom instanceof HTMLElement) comesFrom.focus();
    };
  }, []);

  const options = useMemo(
    () =>
      (vocab ?? [])
        .flatMap((group) => group.tags.map((tag) => ({ id: tag.id, label: `${group.name} › ${tag.name}` })))
        .filter(({ label }) => matches(label, query)),
    [vocab, query],
  );
  const highlighted = options.find((option) => option.id === chosen) ?? options[0] ?? null;

  const run = (id: number) => {
    if (apply.isPending) return;
    setError(null);
    apply.mutate(
      { tag: id, recordings },
      {
        onSuccess: onClose,
        onError: (failure) => {
          setError(errorMessage(failure));
          input.current?.focus();
        },
      },
    );
  };

  const move = (step: number) => {
    if (options.length === 0) return;
    const at = highlighted === null ? 0 : options.indexOf(highlighted);
    setChosen(options[(at + step + options.length) % options.length]?.id ?? null);
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
      case "Enter":
        // A button keeps its Enter.
        if (event.target === back.current) return;
        if (highlighted !== null && !event.repeat) run(highlighted.id);
        break;
      case "Escape":
        onClose();
        break;
      case "Tab":
        // The focus stays in the picker: the box and "Go back".
        (event.target === back.current ? input.current : back.current)?.focus();
        break;
      default:
        handled = false;
    }
    if (handled) {
      event.preventDefault();
      event.stopPropagation();
    }
  };

  return (
    // A click anywhere outside the panel closes it, like Escape.
    <div className={styles.backdrop} onMouseDown={onClose}>
      <div
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        className={styles.panel}
        onMouseDown={(event) => {
          event.stopPropagation();
          // A click on the panel must not take the focus out of the box.
          if (event.target !== input.current && event.target !== back.current) event.preventDefault();
        }}
        onKeyDown={onKeyDown}
      >
        <div className={styles.top}>
          <h2 id={titleId} className={styles.title}>
            {t("picker.title")}
          </h2>
          <p className={styles.quiet}>{t("picker.trackCount", { count: recordings.length })}</p>
        </div>
        <input
          ref={input}
          type="text"
          role="combobox"
          aria-label={t("picker.search")}
          aria-expanded="true"
          aria-controls={listId}
          aria-autocomplete="list"
          aria-activedescendant={highlighted === null ? undefined : `${dialogId}-${highlighted.id}`}
          placeholder={t("picker.search")}
          autoComplete="off"
          spellCheck={false}
          className={styles.input}
          value={query}
          onChange={(event) => {
            setQuery(event.target.value);
            setChosen(null);
            setError(null);
          }}
        />
        <ul
          id={listId}
          role="listbox"
          aria-label={t("picker.list")}
          className={styles.list}
          onMouseDown={(event) => event.preventDefault()}
        >
          {options.map((option) => (
            <li
              key={option.id}
              id={`${dialogId}-${option.id}`}
              role="option"
              aria-selected={option === highlighted}
              className={styles.option}
              onMouseMove={() => setChosen(option.id)}
              onClick={() => run(option.id)}
            >
              <span className={styles.label}>{option.label}</span>
            </li>
          ))}
        </ul>
        {options.length === 0 && (
          <p role="status" className={styles.quiet}>
            {vocab !== undefined && vocab.every((group) => group.tags.length === 0)
              ? t("picker.empty")
              : t("picker.none")}
          </p>
        )}
        {error !== null && (
          <p role="alert" className={styles.error}>
            {error}
          </p>
        )}
        <div className={styles.footer}>
          <button ref={back} type="button" className={styles.back} onClick={onClose}>
            {t("picker.cancel")}
          </button>
        </div>
      </div>
    </div>
  );
}
