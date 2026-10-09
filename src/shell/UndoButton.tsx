import type { TFunction } from "i18next";
import { useTranslation } from "react-i18next";
import { errorMessage } from "../api/errors";
import type { OperationInfo, UndoRefusal } from "../bindings";
import styles from "./TopBar.module.css";
import { useKeepNextUndoCurrent, useNextUndo, useUndoOperation, useUndoShortcut } from "./useUndo";

/**
 * The button's text: "Undo" plus what it takes back. The Rust side stores an
 * operation's kind and details, never text; the wording is here. A kind this
 * build has no words for, or one recorded without the details its words
 * need (an older build's), is named plainly.
 */
function undoLabel(t: TFunction<"shell">, operation: OperationInfo): string {
  const { name, from, tracks } = operation.details;
  switch (operation.kind) {
    case "promote":
      return t("undo.action.addToLibrary");
    case "remove_from_library":
      return t("undo.action.removeFromLibrary");
    case "add_rekordbox_tracks":
      if (tracks !== null) return t("undo.action.addFromRekordbox", { count: tracks });
      break;
    case "create_crate":
      if (name !== null) return t("undo.action.createCrate", { name });
      break;
    case "rename_crate":
      if (name !== null && from !== null) return t("undo.action.renameCrate", { from, name });
      break;
    case "delete_crate":
      if (name !== null) return t("undo.action.deleteCrate", { name });
      break;
    case "add_to_crate":
      if (name !== null && tracks !== null) {
        return t("undo.action.addToCrate", { count: tracks, name });
      }
      break;
    case "remove_from_crate":
      if (name !== null && tracks !== null) {
        return t("undo.action.removeFromCrate", { count: tracks, name });
      }
      break;
  }
  return t("undo.action.other");
}

function refusalText(t: TFunction<"shell">, refusal: UndoRefusal): string {
  switch (refusal.code) {
    case "sentSince":
      return t("undo.refused.sentSince");
    case "sourceGone":
      return t("undo.refused.sourceGone");
    case "crateNameTaken":
      return t("undo.refused.crateNameTaken", { name: refusal.name });
    case "changedSince":
      return t("undo.refused.changedSince");
  }
}

/**
 * The top bar's Undo (1bA-12): takes back the newest action not yet undone,
 * and names it. Pressing it again takes back the one before, and so on.
 * Greyed out when there is nothing to undo, and when the next step can't be
 * undone: then the reason is shown beside it, and the steps before it stay
 * out of reach. Ctrl+Z does the same, except in a text field.
 */
export function UndoButton() {
  const { t } = useTranslation("shell");
  useKeepNextUndoCurrent();
  const next = useNextUndo();
  const undo = useUndoOperation();

  const operation = next.data?.operation ?? null;
  // Known ahead of time, or found when Undo was last used on this operation.
  const refusal =
    next.data?.refusal ??
    (undo.data?.status === "refused" && undo.data.operation.id === operation?.id
      ? undo.data.reason
      : null);
  const available = operation !== null && refusal === null && !undo.isPending;
  const run = () => {
    if (available) undo.mutate(operation.id);
  };
  useUndoShortcut(run);

  return (
    <div className={styles.undo}>
      <button
        type="button"
        className={styles.undoButton}
        disabled={!available}
        onClick={run}
      >
        {operation === null ? t("undo.button") : undoLabel(t, operation)}
      </button>
      <kbd className={styles.undoShortcut}>{t("undo.shortcut")}</kbd>
      {operation !== null && refusal !== null && (
        <span role="status" className={styles.undoReason}>
          {refusalText(t, refusal)}
        </span>
      )}
      {undo.isError && (
        <span role="alert" className={styles.undoError}>
          {errorMessage(undo.error)}
        </span>
      )}
    </div>
  );
}
