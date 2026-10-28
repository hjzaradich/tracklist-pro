import type { TFunction } from "i18next";
import { useTranslation } from "react-i18next";
import { errorMessage } from "../api/errors";
import type { OperationInfo, UndoRefusal } from "../bindings";
import { useRegisterActions } from "../palette/registry";
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
    case "create_crate_folder":
      if (name !== null) return t("undo.action.createFolder", { name });
      break;
    case "rename_crate_folder":
      if (name !== null && from !== null) return t("undo.action.renameFolder", { from, name });
      break;
    case "delete_crate_folder":
      if (name !== null) return t("undo.action.deleteFolder", { name });
      break;
    case "move_crate":
      if (name !== null) return t("undo.action.moveCrate", { name });
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
    case "dismiss_better_file":
      return t("undo.action.dismissBetterFile");
    case "keep_anyway":
      return t("undo.action.keepAnyway");
    case "want_better_file":
      return t("undo.action.wantBetterFile");
    case "switch_linked_file":
      return t("undo.action.switchLinkedFile");
    case "accept_clear_best_files":
      if (tracks !== null) return t("undo.action.acceptClearBestFiles", { count: tracks });
      break;
    case "confirm_relink":
      return t("undo.action.confirmRelink");
    case "reject_relink":
      return t("undo.action.rejectRelink");
    case "withdraw_relink":
      return t("undo.action.withdrawRelink");
    case "keep_both_library_tracks":
      return t("undo.action.keepBoth");
    case "merge_library_tracks":
      return t("undo.action.mergeTracks");
    case "same_track":
      return t("undo.action.sameTrack");
    case "not_same_track":
      return t("undo.action.notSameTrack");
    case "split_off_file":
      return t("undo.action.splitOffFile");
    case "show_file_again":
      return t("undo.action.showFileAgain");
    case "use_this_file":
      return t("undo.action.useThisFile");
    case "resolve_conflict":
      return t("undo.action.resolveConflict");
    case "resolve_conflicts":
      return t("undo.action.resolveConflicts");
  }
  return t("undo.action.other");
}

export function refusalText(t: TFunction<"shell">, refusal: UndoRefusal): string {
  switch (refusal.code) {
    case "sentSince":
      return t("undo.refused.sentSince");
    case "sourceGone":
      return t("undo.refused.sourceGone");
    case "crateNameTaken":
      return t("undo.refused.crateNameTaken", { name: refusal.name });
    case "changedSince":
      return t("undo.refused.changedSince");
    case "switchSentSince":
      return t("undo.refused.switchSentSince");
    case "mergeLinkChanged":
      return t("undo.refused.mergeLinkChanged");
  }
}

/**
 * The top bar's Undo (1bA-12): takes back the newest action not yet undone,
 * and names it. Pressing it again takes back the one before, and so on.
 * Greyed out when there is nothing to undo, and when the next step can't be
 * undone: then the reason is shown beside it, and the steps before it stay
 * out of reach. Ctrl+Z does the same, except in a text field, and so does
 * the command palette (1cB-7).
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
  // The command palette lists this step and runs this same handler (1cB-7).
  const label = operation === null ? t("undo.button") : undoLabel(t, operation);
  useRegisterActions("undo", [{ id: "undo", label, shortcut: t("undo.shortcut"), available, run }]);

  return (
    <div className={styles.undo}>
      <button
        type="button"
        className={styles.undoButton}
        disabled={!available}
        onClick={run}
      >
        {label}
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
