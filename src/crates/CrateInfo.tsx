import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { errorMessage } from "../api/errors";
import { useKeyNotation } from "../keys/useKeyNotation";
import styles from "./CratesScreen.module.css";
import { summaryParts } from "./summaryLine";
import { useCrateSummary } from "./useCrateSummary";

/** The most characters a note holds (the backend refuses more: `MAX_NOTES_CHARS`). */
export const MAX_NOTES_CHARS = 300;

/** Rows the box a note is written in has at least; it grows with the note (the stylesheet). */
export const NOTES_ROWS = 3;

/**
 * A crate's note (1cB-15; ROADMAP 1.14), under its name. Click it to edit it
 * in place, in a box of at least three rows that grows with the note, so a
 * note of the longest length is all in view: Enter or leaving the box saves (a note has no line breaks, so Enter
 * has no other use), Escape goes back to the note as it was. An empty note clears it. Saving is the screen's
 * (`onSave`); it records one operation, so the top bar's Undo puts the old
 * note back.
 */
export function CrateNotes({ id, onSave }: { id: number; onSave: (notes: string) => void }) {
  const { t } = useTranslation("crates");
  const summary = useCrateSummary(id);
  // What is typed while editing; `null` when not editing.
  const [draft, setDraft] = useState<string | null>(null);
  // Enter, Escape and the blur that follows them are one decision, not two.
  const decided = useRef(true);
  const input = useRef<HTMLTextAreaElement>(null);
  const editing = draft !== null;
  useEffect(() => {
    if (editing) input.current?.focus();
  }, [editing]);

  if (summary.data === undefined) return null;
  const notes = summary.data.notes;

  const save = () => {
    if (decided.current) return;
    decided.current = true;
    const text = draft ?? "";
    setDraft(null);
    // The note as it is stored has its ends trimmed: the same is no change.
    if (text.trim() !== (notes ?? "")) onSave(text);
  };
  const cancel = () => {
    decided.current = true;
    setDraft(null);
  };

  if (draft !== null) {
    return (
      <textarea
        ref={input}
        className={styles.noteInput}
        rows={NOTES_ROWS}
        aria-label={t("notes.label")}
        maxLength={MAX_NOTES_CHARS}
        value={draft}
        // A pasted line break is a space: the note is one paragraph.
        onChange={(event) => setDraft(event.target.value.replace(/\s*[\r\n]+\s*/g, " "))}
        onBlur={save}
        onKeyDown={(event) => {
          // An input method's Enter (or Escape) settles its candidate; it isn't
          // the user's. Some browsers send the key after the composition has
          // ended, with key code 229 and no composing flag.
          if (event.nativeEvent.isComposing || event.keyCode === 229) return;
          if (event.key === "Enter") {
            event.preventDefault();
            save();
          } else if (event.key === "Escape") {
            event.preventDefault();
            event.stopPropagation();
            cancel();
          }
        }}
      />
    );
  }
  return (
    <button
      type="button"
      className={notes === null ? `${styles.note} ${styles.noteEmpty}` : styles.note}
      title={t("notes.edit")}
      onClick={() => {
        decided.current = false;
        setDraft(notes ?? "");
      }}
    >
      {notes ?? t("notes.add")}
    </button>
  );
}

/**
 * The line under a crate's path saying what its tracks add up to (1cB-15):
 * BPM range, energy range, keys, top genres. A part with no data is left
 * out; an empty crate says so.
 */
export function CrateSummaryLine({ id }: { id: number }) {
  const { t } = useTranslation("crates");
  const notation = useKeyNotation();
  const summary = useCrateSummary(id);
  if (summary.isError) return <p className={styles.error}>{errorMessage(summary.error)}</p>;
  if (summary.data === undefined) return null;
  return <p className={styles.summary}>{summaryParts(summary.data, notation, t).join(t("summary.separator"))}</p>;
}
