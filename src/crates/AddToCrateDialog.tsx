import { useEffect, useId, useMemo, useRef, useState, type KeyboardEvent } from "react";
import { useTranslation } from "react-i18next";
import { errorMessage } from "../api/errors";
import type { Crate } from "../bindings";
import { matches } from "../palette/match";
import { useAddSelectionToCrate } from "./addSelection";
import styles from "./AddToCrateDialog.module.css";
import { crateChoices } from "./tree";
import { useCrates, useCreateCrate } from "./useCrates";

/** How a name is compared with its siblings (ROADMAP 1.9): NFC, letter case and trailing space ignored. */
const siblingKey = (name: string) => name.normalize("NFC").trim().toLowerCase();

type Option =
  | { kind: "crate"; crate: Crate; label: string }
  /** A new crate at the top of the tree, named by what was typed. */
  | { kind: "create"; name: string };

/**
 * "Add to crate or playlist" (1cB-13; ROADMAP 1.14): a text box over the list
 * of crates, each named by its place ("Friday residency › Warmup"). Typing
 * finds crates the way the command palette finds actions: every word typed
 * starts a word of the name. Enter adds the selected tracks to the highlighted
 * crate; the last row, when something is typed, makes a new crate at the top
 * of the tree from the typed name (the sibling-name rules are the backend's,
 * so a name already used is refused there) and adds to it. Folders can't be
 * chosen: they hold crates, not tracks. Escape, a click outside and "Go back"
 * close it. What was added is told on the line over the track table.
 *
 * `recordings` are the tracks (`TableRow.recordingId`) as they were when it
 * opened.
 */
export function AddToCrateDialog({
  recordings,
  onClose,
}: {
  recordings: readonly number[];
  onClose: () => void;
}) {
  const { t } = useTranslation("crates");
  const crates = useCrates().data;
  const create = useCreateCrate();
  const addSelection = useAddSelectionToCrate();
  const [query, setQuery] = useState("");
  const [chosen, setChosen] = useState<string | null>(null);
  const [working, setWorking] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const input = useRef<HTMLInputElement>(null);
  const back = useRef<HTMLButtonElement>(null);
  const listId = useId();
  const dialogId = useId();
  const titleId = useId();

  // Focus goes into the box, and back to where it was when the dialog closes.
  useEffect(() => {
    const comesFrom = document.activeElement;
    input.current?.focus();
    return () => {
      if (comesFrom instanceof HTMLElement) comesFrom.focus();
    };
  }, []);

  const options = useMemo(() => {
    const list = crates ?? [];
    const found: Option[] = crateChoices(list)
      .filter(({ label }) => matches(label, query))
      .map(({ crate, label }) => ({ kind: "crate", crate, label }));
    const name = query.trim();
    const taken = list.some(
      (c) => c.kind === "static" && c.parentId === null && siblingKey(c.name) === siblingKey(name),
    );
    if (name !== "" && !taken) found.push({ kind: "create", name });
    return found;
  }, [crates, query]);

  const idOf = (option: Option) => (option.kind === "crate" ? `crate-${option.crate.id}` : "create");
  const highlighted = options.find((option) => idOf(option) === chosen) ?? options[0] ?? null;
  const highlightedId = highlighted === null ? undefined : idOf(highlighted);

  useEffect(() => {
    if (highlightedId === undefined) return;
    document.getElementById(`${dialogId}-${highlightedId}`)?.scrollIntoView?.({ block: "nearest" });
  }, [highlightedId, dialogId]);

  const run = async (option: Option) => {
    if (working) return;
    setWorking(true);
    setError(null);
    try {
      const target =
        option.kind === "crate"
          ? option.crate
          : { id: await create.mutateAsync({ name: option.name, parent: null }), name: option.name.trim() };
      await addSelection(target, recordings);
      onClose();
    } catch (failure) {
      setError(errorMessage(failure));
      setWorking(false);
    }
  };

  const move = (step: number) => {
    if (options.length === 0) return;
    const at = highlighted === null ? 0 : options.indexOf(highlighted);
    const next = options[(at + step + options.length) % options.length];
    setChosen(next === undefined ? null : idOf(next));
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
      case "Home": {
        const first = options[0];
        setChosen(first === undefined ? null : idOf(first));
        break;
      }
      case "End": {
        const last = options.at(-1);
        setChosen(last === undefined ? null : idOf(last));
        break;
      }
      case "Enter":
        // A button keeps its Enter.
        if (event.target === back.current) return;
        if (highlighted !== null && !event.repeat) void run(highlighted);
        break;
      case "Escape":
        onClose();
        break;
      case "Tab": {
        // The focus stays in the dialog: the box and "Go back".
        (event.target === back.current ? input.current : back.current)?.focus();
        break;
      }
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
            {t("addDialog.title")}
          </h2>
          <p className={styles.quiet}>{t("trackCount", { count: recordings.length })}</p>
        </div>
        <input
          ref={input}
          type="text"
          role="combobox"
          aria-label={t("addDialog.search")}
          aria-expanded="true"
          aria-controls={listId}
          aria-autocomplete="list"
          aria-activedescendant={highlightedId === undefined ? undefined : `${dialogId}-${highlightedId}`}
          placeholder={t("addDialog.search")}
          autoComplete="off"
          spellCheck={false}
          className={styles.input}
          value={query}
          disabled={working}
          onChange={(event) => {
            setQuery(event.target.value);
            setChosen(null);
            setError(null);
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
          {options.map((option) => (
            <li
              key={idOf(option)}
              id={`${dialogId}-${idOf(option)}`}
              role="option"
              aria-selected={option === highlighted}
              className={styles.option}
              onMouseMove={() => setChosen(idOf(option))}
              onClick={() => void run(option)}
            >
              <span className={styles.label}>
                {option.kind === "crate" ? option.label : t("addDialog.create", { name: option.name })}
              </span>
            </li>
          ))}
        </ul>
        {options.length === 0 && (
          <p role="status" className={styles.quiet}>
            {crates !== undefined && crates.length === 0 ? t("empty") : t("addDialog.none")}
          </p>
        )}
        {error !== null && (
          <p role="alert" className={styles.error}>
            {error}
          </p>
        )}
        <div className={styles.footer}>
          <button ref={back} type="button" className={styles.back} onClick={onClose}>
            {t("addDialog.cancel")}
          </button>
        </div>
      </div>
    </div>
  );
}
