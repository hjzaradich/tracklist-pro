import { useEffect, useId, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { errorMessage } from "../api/errors";
import type { Crate, FolderContents } from "../bindings";
import styles from "./CratesScreen.module.css";
import { descendantIds, folderChoices } from "./tree";

/** One line to type a crate's or folder's name in. Enter confirms, Escape goes back. */
export function NameForm({
  label,
  confirm,
  cancel,
  initial,
  pending,
  onSubmit,
  onCancel,
}: {
  label: string;
  confirm: string;
  cancel: string;
  initial: string;
  pending: boolean;
  onSubmit: (name: string) => void;
  onCancel: () => void;
}) {
  const [name, setName] = useState(initial);
  const input = useRef<HTMLInputElement>(null);
  const id = useId();
  useEffect(() => input.current?.focus(), []);
  return (
    <form
      className={styles.form}
      onSubmit={(event) => {
        event.preventDefault();
        onSubmit(name);
      }}
      onKeyDown={(event) => {
        if (event.key === "Escape") onCancel();
      }}
    >
      <label htmlFor={id} className={styles.label}>
        {label}
      </label>
      <input
        ref={input}
        id={id}
        className={styles.input}
        value={name}
        onChange={(event) => setName(event.target.value)}
      />
      <div className={styles.actions}>
        <button type="submit" className={styles.primary} disabled={pending}>
          {confirm}
        </button>
        <button type="button" className={styles.button} onClick={onCancel}>
          {cancel}
        </button>
      </div>
    </form>
  );
}

/**
 * Asks before a crate or folder is deleted. Nothing happens until the user
 * confirms. For a folder it says what goes with it: how many crates and
 * folders, and how many different tracks those crates hold (the tracks
 * stay in the Library). Deleting waits until those counts are known.
 */
export function ConfirmDelete({
  crate,
  contents,
  contentsError,
  pending,
  onConfirm,
  onCancel,
}: {
  crate: Crate;
  /** A folder's contents; `undefined` while they are being counted. */
  contents: FolderContents | undefined;
  contentsError: unknown;
  pending: boolean;
  onConfirm: () => void;
  onCancel: () => void;
}) {
  const { t } = useTranslation("crates");
  const folder = crate.kind === "folder";
  // The dialog takes focus, on the safe choice.
  const cancel = useRef<HTMLButtonElement>(null);
  useEffect(() => cancel.current?.focus(), []);
  const titleId = useId();
  const detailId = useId();
  const empty = contents !== undefined && contents.crates + contents.folders === 0;
  return (
    <div
      role="alertdialog"
      aria-labelledby={titleId}
      aria-describedby={detailId}
      className={styles.confirm}
      onKeyDown={(event) => {
        if (event.key === "Escape") onCancel();
      }}
    >
      <p id={titleId} className={styles.confirmTitle}>
        {t(folder ? "delete.folderTitle" : "delete.title", { name: crate.name })}
      </p>
      <div id={detailId} className={styles.confirmDetail}>
        {folder && contentsError !== null && contentsError !== undefined && (
          <p role="alert" className={styles.error}>
            {errorMessage(contentsError)}
          </p>
        )}
        {folder && contents === undefined && contentsError == null && <p>{t("delete.counting")}</p>}
        {folder && contents !== undefined && (
          <>
            <p>{empty ? t("delete.holdsNothing") : t("delete.holds")}</p>
            {!empty && (
              <ul className={styles.holds}>
                <li>{t("delete.crates", { count: contents.crates })}</li>
                <li>{t("delete.folders", { count: contents.folders })}</li>
                <li>{t("delete.tracks", { count: contents.tracks })}</li>
              </ul>
            )}
          </>
        )}
        <p>{t("delete.detail")}</p>
      </div>
      <div className={styles.actions}>
        <button
          type="button"
          className={styles.primary}
          disabled={pending || (folder && contents === undefined)}
          onClick={onConfirm}
        >
          {t("delete.confirm")}
        </button>
        <button ref={cancel} type="button" className={styles.button} onClick={onCancel}>
          {t("delete.cancel")}
        </button>
      </div>
    </div>
  );
}

/**
 * "Move to folder": the folders a crate or folder can go to, and the top
 * level. A folder is not offered itself or any folder inside it (the
 * backend refuses those too). The place it is in already is left in, so
 * the list is the whole tree.
 */
export function MoveForm({
  crate,
  all,
  pending,
  onSubmit,
  onCancel,
}: {
  crate: Crate;
  all: Crate[];
  pending: boolean;
  /** `null`: the top level. */
  onSubmit: (folder: number | null) => void;
  onCancel: () => void;
}) {
  const { t } = useTranslation("crates");
  const id = useId();
  const select = useRef<HTMLSelectElement>(null);
  useEffect(() => select.current?.focus(), []);
  const except = descendantIds(all, crate.id);
  except.add(crate.id);
  const choices = folderChoices(all, except);
  const [chosen, choose] = useState<string>(crate.parentId === null ? "" : String(crate.parentId));
  return (
    <form
      className={styles.form}
      onSubmit={(event) => {
        event.preventDefault();
        onSubmit(chosen === "" ? null : Number(chosen));
      }}
      onKeyDown={(event) => {
        if (event.key === "Escape") onCancel();
      }}
    >
      <label htmlFor={id} className={styles.label}>
        {t("move.label", { name: crate.name })}
      </label>
      <select
        ref={select}
        id={id}
        className={styles.input}
        value={chosen}
        onChange={(event) => choose(event.target.value)}
      >
        <option value="">{t("move.top")}</option>
        {choices.map((choice) => (
          <option key={choice.id} value={choice.id}>
            {choice.label}
          </option>
        ))}
      </select>
      <div className={styles.actions}>
        <button type="submit" className={styles.primary} disabled={pending}>
          {t("move.confirm")}
        </button>
        <button type="button" className={styles.button} onClick={onCancel}>
          {t("move.cancel")}
        </button>
      </div>
    </form>
  );
}
