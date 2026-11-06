import { useTranslation } from "react-i18next";
import type { InboxItem } from "../bindings";
import { PlayButton } from "../player/PlayButton";
import { fileNameOf, reasonText } from "./inboxText";
import type { Result } from "./inboxText";
import { ResultLine } from "./ResultLine";
import styles from "./inbox.module.css";

/** Where in the file an Inbox preview starts: about a third of the way in (ROADMAP 2.1). */
export const PREVIEW_START = 1 / 3;

/** The labels whose file can be added to the Library as it is. */
export const ADDABLE = ["new_to_you", "other_cut", "rework"] as const;

export type RowAction = "add" | "addAnyway" | "notNow" | "hide" | "clear";

/**
 * One Inbox file: its name, label and the reason for it, and the actions
 * its label allows. A file still being checked has none; a better file can
 * be set aside or hidden (Upgrade comes later); a file already in the
 * Library can be cleared.
 */
export function InboxRow({
  item,
  busy,
  result,
  names,
  onAct,
}: {
  item: InboxItem;
  busy: boolean;
  result: Result | undefined;
  names: ReadonlyMap<number, string>;
  onAct: (action: RowAction) => void;
}) {
  const { t } = useTranslation("inbox");
  const name = fileNameOf(item);
  const addable = (ADDABLE as readonly string[]).includes(item.label);
  const button = (action: RowAction, text: string, className = styles.action) => (
    <button type="button" className={className} disabled={busy} onClick={() => onAct(action)}>
      {text}
    </button>
  );
  return (
    <li className={styles.row} data-label={item.label}>
      <div className={styles.rowMain}>
        <PlayButton file={{ id: item.fileId, name }} startAtFraction={PREVIEW_START} />
        <span className={styles.name}>{name}</span>
        <span className={styles.label}>{t(`label.${item.label}`)}</span>
        <span className={styles.reason}>{reasonText(t, item.reason)}</span>
        <span className={styles.actions}>
          {addable && button("add", t("action.add"), styles.primary)}
          {(addable || item.label === "better_file") && button("notNow", t("action.notNow"))}
          {(addable || item.label === "better_file") && button("hide", t("action.hide"), styles.quiet)}
          {item.label === "already_have" && button("clear", t("action.clear"))}
        </span>
      </div>
      {result && (
        <ResultLine
          result={result}
          names={names}
          onAddAnyway={() => onAct("addAnyway")}
          addAnywayDisabled={busy}
        />
      )}
    </li>
  );
}
