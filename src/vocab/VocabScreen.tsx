import { useId, useState, type FormEvent, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { errorMessage } from "../api/errors";
import type { Tag, TagGroup, VocabItem } from "../bindings";
import stage from "../shell/StageScreen.module.css";
import { colourOf, DEFAULT_COLOUR, PALETTE } from "./colours";
import styles from "./vocab.module.css";
import {
  useColourItem,
  useCreateGroup,
  useCreateTag,
  useImportTags,
  useRenameItem,
  useVocab,
} from "./useVocab";

/** A name typed into a one-line form; the backend trims it and refuses what it can't use. */
function NameForm({
  label,
  action,
  initial = "",
  onDone,
  run,
}: {
  label: string;
  action: string;
  initial?: string;
  onDone?: () => void;
  run: (name: string) => Promise<unknown>;
}) {
  const { t } = useTranslation("vocab");
  const [name, setName] = useState(initial);
  const [error, setError] = useState<string | null>(null);
  const submit = (event: FormEvent) => {
    event.preventDefault();
    setError(null);
    run(name).then(
      () => {
        setName("");
        onDone?.();
      },
      (failure: unknown) => setError(errorMessage(failure)),
    );
  };
  return (
    <form className={styles.form} onSubmit={submit}>
      <input
        type="text"
        aria-label={label}
        placeholder={label}
        className={styles.input}
        value={name}
        autoComplete="off"
        spellCheck={false}
        onChange={(event) => setName(event.target.value)}
      />
      <button type="submit" className={styles.button} disabled={name.trim() === ""}>
        {action}
      </button>
      {onDone !== undefined && (
        <button type="button" className={styles.button} onClick={onDone}>
          {t("cancel")}
        </button>
      )}
      {error !== null && (
        <p role="alert" className={styles.error}>
          {error}
        </p>
      )}
    </form>
  );
}

/** The colour choice of a group or tag: the plain palette, or the default. */
function ColourSelect({ item, colour, name }: { item: VocabItem; colour: string | null; name: string }) {
  const { t } = useTranslation("vocab");
  const set = useColourItem();
  return (
    <select
      className={styles.select}
      aria-label={t("colour.label", { name })}
      value={colour ?? ""}
      onChange={(event) => set.mutate({ item, colour: event.target.value === "" ? null : event.target.value })}
    >
      <option value="">{t("colour.default")}</option>
      {colour !== null && !PALETTE.includes(colour) && <option value={colour}>{colour}</option>}
      {PALETTE.map((hex) => (
        <option key={hex} value={hex}>
          {hex}
        </option>
      ))}
    </select>
  );
}

/** A group's or tag's name with Rename and its colour. */
function ItemRow({
  item,
  name,
  colour,
  shown,
  children,
}: {
  item: VocabItem;
  name: string;
  colour: string | null;
  shown: string;
  children?: ReactNode;
}) {
  const { t } = useTranslation("vocab");
  const rename = useRenameItem();
  const [renaming, setRenaming] = useState(false);
  return (
    <div className={styles.itemRow}>
      <span className={styles.swatch} style={{ background: shown }} aria-hidden="true" />
      {renaming ? (
        <NameForm
          label={t("rename.label", { name })}
          action={t("rename.confirm")}
          initial={name}
          onDone={() => setRenaming(false)}
          run={(next) => rename.mutateAsync({ item, name: next })}
        />
      ) : (
        <>
          <span className={styles.itemName}>{name}</span>
          {children}
          <button type="button" className={styles.button} onClick={() => setRenaming(true)}>
            {t("rename.action")}
          </button>
        </>
      )}
      <ColourSelect item={item} colour={colour} name={name} />
    </div>
  );
}

function TagRow({ tag, group }: { tag: Tag; group: TagGroup }) {
  const { t } = useTranslation("vocab");
  return (
    <li>
      <ItemRow item={{ kind: "tag", id: tag.id }} name={tag.name} colour={tag.colour} shown={colourOf(tag, group)}>
        <span className={styles.quiet}>{t("trackCount", { count: tag.trackCount })}</span>
        {tag.goneFromRekordbox && <span className={styles.flag}>{t("goneFromRekordbox")}</span>}
      </ItemRow>
    </li>
  );
}

function GroupSection({ group }: { group: TagGroup }) {
  const { t } = useTranslation("vocab");
  const createTag = useCreateTag();
  const headingId = useId();
  return (
    <section className={styles.group} aria-labelledby={headingId}>
      <h2 id={headingId} className={styles.groupHeading}>
        <ItemRow item={{ kind: "group", id: group.id }} name={group.name} colour={group.colour} shown={group.colour ?? DEFAULT_COLOUR} />
      </h2>
      {group.tags.length === 0 ? (
        <p className={styles.quiet}>{t("group.empty")}</p>
      ) : (
        <ul className={styles.tags}>
          {group.tags.map((tag) => (
            <TagRow key={tag.id} tag={tag} group={group} />
          ))}
        </ul>
      )}
      <NameForm
        label={t("newTag.label", { group: group.name })}
        action={t("newTag.action")}
        run={(name) => createTag.mutateAsync({ group: group.id, name })}
      />
    </section>
  );
}

/**
 * The tag vocabulary (3A-T; ROADMAP 3.1), at `/tags`: the groups and their
 * tags, each with Rename and a colour, a form for a new group or tag, and "Copy
 * from rekordbox", which copies the My Tags rekordbox has as tags. Every change
 * is one operation (Undo, in the top bar). A tag imported from a My Tag that
 * rekordbox no longer has stays, marked. Plain on purpose.
 */
export function VocabScreen() {
  const { t } = useTranslation("vocab");
  const vocab = useVocab();
  const createGroup = useCreateGroup();
  const importTags = useImportTags();
  const done = importTags.data;
  return (
    <div className={stage.screen}>
      <div className={stage.header}>
        <h1 className={stage.title}>{t("title")}</h1>
        <button
          type="button"
          className={styles.button}
          disabled={importTags.isPending}
          onClick={() => importTags.mutate(t("import.group"))}
        >
          {t("import.action")}
        </button>
      </div>
      <div className={stage.body}>
        <div className={styles.column}>
          {importTags.isError && (
            <p role="alert" className={styles.error}>
              {errorMessage(importTags.error)}
            </p>
          )}
          {done !== undefined && !importTags.isError && (
            <p role="status" className={styles.quiet}>
              {done.operationId === null
                ? t("import.nothing")
                : t("import.done", { added: done.added, gone: done.gone, back: done.back, skipped: done.skipped })}
            </p>
          )}
          {vocab.isError && (
            <p role="alert" className={styles.error}>
              {errorMessage(vocab.error)}
            </p>
          )}
          {vocab.data?.length === 0 && <p className={styles.quiet}>{t("empty")}</p>}
          {vocab.data?.map((group) => <GroupSection key={group.id} group={group} />)}
          <NameForm label={t("newGroup.label")} action={t("newGroup.action")} run={(name) => createGroup.mutateAsync(name)} />
        </div>
      </div>
    </div>
  );
}
