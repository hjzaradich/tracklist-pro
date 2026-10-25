import { useEffect, useRef, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { errorMessage } from "../api/errors";
import type {
  ErrorKind,
  ReviewComparison,
  ReviewFileRow,
  ReviewKind,
  SwitchEffect,
  SwitchRefusal,
} from "../bindings";
import {
  useConfirmRelink,
  useDismissBetterFile,
  useKeepAnyway,
  useNotSame,
  useRejectRelink,
  useSameTrack,
  useSwitchLinkedFile,
  useWantBetterFile,
} from "./useReviewActions";
import { useReviewKeys, type ReviewKey } from "./useReviewKeys";
import { SameRecordingActions } from "./SameRecordingActions";
import styles from "./ReviewComparison.module.css";
import screenStyles from "./ReviewScreen.module.css";

/** The error kind that says why a switch is refused: its words are the error's. */
const REFUSAL_KIND: Record<SwitchRefusal, ErrorKind> = {
  linkedOnDisk: "reviewSwitchLinkedOnDisk",
  notInTrack: "reviewSwitchNotInTrack",
  targetGone: "reviewSwitchTargetGone",
  targetLinked: "reviewSwitchTargetLinked",
  entryOfAnotherTrack: "reviewSwitchEntryTaken",
};

/**
 * What a switch means in rekordbox, as a key of the review namespace. A track
 * rekordbox doesn't know has nothing to say: nothing of rekordbox's changes.
 */
const EFFECT_TEXT: Record<SwitchEffect, "switch.keepsEntry" | "switch.newTrack" | "switch.otherEntry" | null> = {
  notInRekordbox: null,
  keepsEntry: "switch.keepsEntry",
  newTrack: "switch.newTrack",
  otherEntry: "switch.otherEntry",
};

/**
 * Whether items of `kind` take "Same track" (Enter) and S, and have rows to
 * select: **the one place that says it** on this side (the backend's is
 * `review::split::decides_tracks`). A possible version mix-up and "Unclear
 * duplicates" for now; the owner is asked about "Clearly best file".
 */
export function decidesTracks(kind: ReviewKind): boolean {
  return kind === "versionDoubt" || kind === "heldSet";
}

/** Starts an action; `onSettled` says it was answered, whatever the answer. */
type Start = (settled: { onSettled: () => void }) => void;

/**
 * The actions for the selected item (1bE-5, 1bE-6, 1bE-7), under the
 * comparison table. Buttons only. Which ones depends on the item's kind:
 * - "Better file available": Dismiss.
 * - "Quality problem": Keep anyway, Find a better file.
 * - "Linked file gone, copy found": Switch file, with what the switch means
 *   in rekordbox said first, for a track rekordbox knows.
 * - "Confirm relink": Confirm (Confirm again, for a match confirmed before
 *   whose file holds other audio now) and Not the same.
 * - "Same track twice in your Library": Merge (which opens the choice of the
 *   track that stays and its confirmation) and Keep both
 *   ({@link SameRecordingActions}).
 *
 * The layout is the designer's (docs/design-style.md, "Review screen"): one
 * row of buttons directly under the table; the one filled button (`primary`)
 * is the one that takes the suggested file, the others are quiet. What a
 * switch means in rekordbox, and why a button is greyed out, are lines under
 * the row.
 *
 * The keys (1bE-2, ROADMAP 1.12 "Keyboard"): Enter does the item's suggested
 * action and nothing else, so a habit of pressing it never decides what
 * wasn't offered as the suggestion: the switch to the suggested copy on
 * "File missing, copy found", confirming on a relink match. On a quality
 * problem and on "Better file available" it does nothing, and those keep
 * their buttons. A number key is a row of the table (1 is the first row, up
 * to 9) and acts only where there is a choice of file: on "File missing,
 * copy found" with more than one copy on disk, it switches to the copy in
 * that row. The `.keys` line says which keys work on the item. "These are the
 * same track" on a possible version mix-up and on "Unclear duplicates" is the
 * "Same track" button, and S there is "Not the same" (two files) or "Split
 * off" (three or more files: the selected row, `selectedFile`; with no row
 * selected S does nothing and the keys line says to select one). Split off
 * takes the selected file's identical copies along (the backend finds them).
 * On a possible version mix-up whose reasons say the names give different
 * versions (`namesSayVersions`), the filled button, and Enter, is "Not the
 * same", or "Split off" once a row is selected (none before). After a
 * split that leaves the item in the queue (its other question still open)
 * the screen stays on it. A refusal shows its reason under the buttons at
 * once.
 *
 * A key press and a click go through the same guard: nothing starts while an
 * action is under way or once the item is decided, so an action is never made
 * twice. `onDecided` is called when an action worked (the screen goes on to
 * the next item); after a refusal nothing is called and the screen stays.
 *
 * Every action can be undone from the top bar's Undo. Give it a `key` per
 * item, so a refusal shown for one item doesn't stay under the next.
 */
export function ReviewActions({
  comparison,
  selectedFile = null,
  onDecided,
}: {
  comparison: ReviewComparison;
  /** The file of the selected row, if a row is selected. */
  selectedFile?: number | null;
  onDecided?: () => void;
}) {
  if (comparison.kind === "sameRecording" && comparison.sameRecording) {
    return (
      <SameRecordingActions
        comparison={comparison}
        offer={comparison.sameRecording}
        onDecided={onDecided}
      />
    );
  }
  return <FileActions comparison={comparison} selectedFile={selectedFile} onDecided={onDecided} />;
}

/** The actions of every kind but "Same track twice" ({@link SameRecordingActions}). */
function FileActions({
  comparison,
  selectedFile,
  onDecided,
}: {
  comparison: ReviewComparison;
  selectedFile: number | null;
  onDecided?: () => void;
}) {
  const { t } = useTranslation("review");
  // Set when an action worked; the item is about to leave the queue.
  const decided = useRef(false);
  // Set from the moment an action is started until it is answered. The mutation's
  // own `isPending` comes a tick later, so a second key press in between would
  // start the action again.
  const started = useRef(false);
  const done = () => {
    decided.current = true;
    onDecided?.();
  };
  // A fresh read of the item (it was asked for again after an action) is a
  // new item to decide, even if it has the same id (a dismissed better file
  // is followed by the next one).
  useEffect(() => {
    decided.current = false;
  }, [comparison]);
  const dismiss = useDismissBetterFile(done);
  const keep = useKeepAnyway(done);
  const want = useWantBetterFile(done);
  const switchFile = useSwitchLinkedFile(done);
  const confirm = useConfirmRelink(done);
  const reject = useRejectRelink(done);
  const same = useSameTrack(done);
  // A split that leaves the item in the queue: the screen stays on it.
  const notSame = useNotSame((answer) => {
    if (!answer.itemStays) done();
  });
  // A relink item names rekordbox's entry; every other kind names a track.
  const recordingId = comparison.id.a;
  const entryId = comparison.id.a;
  const busy =
    dismiss.isPending ||
    keep.isPending ||
    want.isPending ||
    switchFile.isPending ||
    confirm.isPending ||
    reject.isPending ||
    same.isPending ||
    notSame.isPending;
  const failure =
    dismiss.error ??
    keep.error ??
    want.error ??
    switchFile.error ??
    confirm.error ??
    reject.error ??
    same.error ??
    notSame.error;

  // Nothing starts while an action is under way or once the item is decided.
  const run = (start: Start): boolean => {
    if (busy || decided.current || started.current) return false;
    started.current = true;
    start({ onSettled: () => (started.current = false) });
    return true;
  };

  let buttons: ReactNode = null;
  let note: string | null = null;
  let greyedOutBecause: string | null = null;
  // What Enter does, and what the number keys do (a row of the table, 1 to 9).
  let enter: { label: string; start: Start } | null = null;
  let pick: ((row: ReviewFileRow) => Start | null) | null = null;
  // What S does (null: nothing), and the words for it on the keys line.
  let s: { label: string; start: Start | null } | null = null;
  if (comparison.kind === "betterFile" && comparison.suggestion) {
    const fileId = comparison.suggestion.what;
    buttons = (
      <button
        type="button"
        className={styles.button}
        disabled={busy}
        onClick={() => run((o) => dismiss.mutate({ recordingId, fileId }, o))}
      >
        {t("action.dismiss")}
      </button>
    );
  } else if (comparison.kind === "qualityProblem") {
    buttons = (
      <>
        <button
          type="button"
          className={styles.button}
          disabled={busy}
          onClick={() => run((o) => keep.mutate(recordingId, o))}
        >
          {t("action.keepAnyway")}
        </button>
        <button
          type="button"
          className={styles.button}
          disabled={busy}
          onClick={() => run((o) => want.mutate(recordingId, o))}
        >
          {t("action.findBetter")}
        </button>
      </>
    );
  } else if (comparison.kind === "clearBestFile" && comparison.switch) {
    const { fileId, effect, refusal } = comparison.switch;
    const effectText = effect === null ? null : EFFECT_TEXT[effect];
    note = effectText === null ? null : t(effectText);
    greyedOutBecause =
      refusal === null ? null : errorMessage({ kind: REFUSAL_KIND[refusal], params: {} });
    const start: Start = (o) => {
      if (effect !== null) switchFile.mutate({ recordingId, fileId, effect }, o);
    };
    if (effect !== null) enter = { label: t("action.switch"), start };
    // A digit is a row: it switches to that row's file, where the choice is
    // real (more than one copy on disk) and the switch is allowed.
    if (comparison.copies.length > 1) {
      pick = (row) => {
        const offer = comparison.copies.find((copy) => copy.fileId === row.fileId);
        if (row.fileId === null || offer === undefined || offer.effect === null) return null;
        const { fileId: to, effect: shown } = offer;
        return (o) => switchFile.mutate({ recordingId, fileId: to, effect: shown }, o);
      };
    }
    buttons = (
      <button
        type="button"
        className={styles.primary}
        disabled={busy || effect === null}
        onClick={() => run(start)}
      >
        {t("action.switch")}
      </button>
    );
  } else if (decidesTracks(comparison.kind)) {
    const id = comparison.id;
    const recordingIds = comparison.tracks.flatMap((track) =>
      track.recordingId === null ? [] : [track.recordingId],
    );
    const files =
      comparison.rows.filter((row) => row.source === "file").length + comparison.moreFiles;
    const two = files === 2;
    const sameStart: Start = (o) => same.mutate({ id, recordingIds }, o);
    const sLabel = two ? t("action.notSame") : t("action.splitOff");
    // With two files the file sent matters only where they share a track (it
    // is the one split off): the selected row, else the one that isn't the
    // suggestion, never the Library file if another is there (the backend
    // keeps the linked file in its track whatever is sent).
    const fallback =
      comparison.rows.find(
        (row) => row.source === "file" && row.fileId !== comparison.suggestion?.what && !row.linked,
      ) ?? comparison.rows.find((row) => row.source === "file" && row.fileId !== comparison.suggestion?.what);
    const fileId = two ? (selectedFile ?? fallback?.fileId ?? null) : selectedFile;
    s = {
      label: sLabel,
      start: two || fileId !== null ? (o) => notSame.mutate({ id, recordingIds, fileId }, o) : null,
    };
    const sStart = s.start;
    // The filled button follows the evidence (owner decision, 2026-10-24;
    // DOGFOOD B-4): where a version mix-up's own reasons say the names give
    // different versions, it is "Not the same" (two files) or, once a row
    // is selected, "Split off", and none is filled until then. Everywhere
    // else it is "Same track". Enter always does the filled button.
    const filled: "same" | "s" | null = !comparison.namesSayVersions
      ? "same"
      : sStart !== null
        ? "s"
        : null;
    if (filled === "same") enter = { label: t("action.sameTrack"), start: sameStart };
    if (filled === "s" && sStart !== null) enter = { label: sLabel, start: sStart };
    buttons = (
      <>
        <button
          type="button"
          className={filled === "same" ? styles.primary : styles.button}
          disabled={busy}
          onClick={() => run(sameStart)}
        >
          {t("action.sameTrack")}
        </button>
        <button
          type="button"
          className={filled === "s" ? styles.primary : styles.button}
          disabled={busy || sStart === null}
          onClick={() => sStart !== null && run(sStart)}
        >
          {sLabel}
        </button>
      </>
    );
  } else if (comparison.kind === "relink" && comparison.relink) {
    const { fileId, confirmedBefore } = comparison.relink;
    const label = confirmedBefore ? t("action.confirmAgain") : t("action.confirm");
    const start: Start = (o) => confirm.mutate({ entryId, fileId }, o);
    enter = { label, start };
    buttons = (
      <>
        <button type="button" className={styles.primary} disabled={busy} onClick={() => run(start)}>
          {label}
        </button>
        <button
          type="button"
          className={styles.button}
          disabled={busy}
          onClick={() => run((o) => reject.mutate({ entryId, fileId }, o))}
        >
          {t("action.rejectMatch")}
        </button>
      </>
    );
  }

  // The keys. The handler lives as long as the item is on screen, even when it
  // has no buttons; it says whether the key means anything here.
  useReviewKeys((key: ReviewKey) => {
    if (key === "enter") {
      if (enter === null) return false;
      run(enter.start);
      return true;
    }
    if (key === "s") {
      if (s === null || s.start === null) return false;
      run(s.start);
      return true;
    }
    const row = comparison.rows[key - 1];
    const start = row === undefined || pick === null ? null : pick(row);
    if (start === null) return false;
    run(start);
    return true;
  });

  if (buttons === null) return null;

  return (
    <>
      <div role="group" aria-label={t("action.label")} className={styles.actions}>
        {buttons}
        {(enter !== null || pick !== null || s !== null) && (
          <p className={styles.keys}>
            {enter !== null && <span>{t("keys.enter", { action: enter.label })}</span>}
            {pick !== null && <span>{t("keys.rows")}</span>}
            {s !== null && (
              <span>{s.start === null ? t("keys.selectRow") : t("keys.s", { action: s.label })}</span>
            )}
          </p>
        )}
      </div>
      {note !== null && (
        <p className={styles.doubt}>
          <span className={styles.doubtLabel}>{note}</span>
        </p>
      )}
      {greyedOutBecause !== null && <p className={styles.doubt}>{greyedOutBecause}</p>}
      {failure && (
        <p role="alert" className={screenStyles.error}>
          {errorMessage(failure)}
        </p>
      )}
    </>
  );
}
