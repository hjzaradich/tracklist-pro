import { type ReactNode, type Ref, useId } from "react";
import styles from "./settings.module.css";

/**
 * One setting on the Settings screen: a native checkbox with its label, its
 * help text below (linked with `aria-describedby`), an error line, and room
 * under the row for a confirm or a note. A change applies at once, so there is
 * no Save; the row is disabled while a change is being saved.
 */
export function SwitchRow({
  label,
  help,
  notes = [],
  checked,
  pending,
  error,
  onChange,
  inputRef,
  children,
}: {
  label: string;
  /** The help under the switch; it changes with the state, the label never does. */
  help: string;
  /** Further lines under the help, read with it. */
  notes?: string[];
  checked: boolean;
  /** A change is being saved, or counted: the row can't be used. */
  pending: boolean;
  /** Why the last change failed; shown under the row. */
  error?: string | null;
  onChange: (on: boolean) => void;
  inputRef?: Ref<HTMLInputElement>;
  /** Whatever opens under the row (a confirm, a status). */
  children?: ReactNode;
}) {
  const helpId = useId();
  const noteIds = notes.map((_, index) => `${helpId}-${index}`);
  return (
    <div className={styles.row}>
      <label className={styles.label}>
        <input
          ref={inputRef}
          type="checkbox"
          className={styles.checkbox}
          checked={checked}
          disabled={pending}
          aria-describedby={[helpId, ...noteIds].join(" ")}
          onChange={(event) => onChange(event.target.checked)}
        />
        {label}
      </label>
      <p id={helpId} className={styles.help}>
        {help}
      </p>
      {notes.map((note, index) => (
        <p key={noteIds[index]} id={noteIds[index]} className={styles.help}>
          {note}
        </p>
      ))}
      {error && (
        <p role="alert" className={styles.error}>
          {error}
        </p>
      )}
      {children}
    </div>
  );
}
