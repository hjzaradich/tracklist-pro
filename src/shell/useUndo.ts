import { useEffect, useRef } from "react";
import {
  type QueryClient,
  useMutation,
  useQuery,
  useQueryClient,
} from "@tanstack/react-query";
import { useWhenJobsEnd } from "../activity/useWhenJobsEnd";
import { ALL_MUSIC_QUERY_KEY } from "../allMusic/useAllMusic";
import { unwrap } from "../api/errors";
import { commands, type NextUndo, type UndoOutcome } from "../bindings";
import { CRATES_QUERY_KEY } from "../crates/useCrates";
import { REKORDBOX_OFFER_QUERY_KEY } from "../firstRun/queryKeys";
import { LIBRARY_TRACKS_QUERY_KEY } from "../library/useLibraryTracks";
import { OVERVIEW_QUERY_KEY } from "../overview/useOverview";
import { MISSING_TRACKS_QUERY_KEY } from "../review/useMissingTracks";
import { useAcceptedStore } from "../review/acceptedStore";
import { TRACK_TAG_SOURCES_QUERY_KEY } from "./useTrackTagSources";
import { useUndoneCount } from "./undoneStore";
import { REVIEW_QUERY_KEYS } from "../review/useReview";

// Undo (1bA-12): one step at a time, newest first, from the top bar or
// Ctrl+Z, and only there (owner decision, 2026-10-27): no screen offers an
// Undo of its own after an action. Both ask the Rust side, which keeps the
// history; nothing about it is remembered here.

export const NEXT_UNDO_QUERY_KEY = ["nextUndo"] as const;

const askNextUndo = (): Promise<NextUndo> => unwrap(commands.nextUndoOperation());

/** What Undo would take back now, and why it would be refused, if known. */
export function useNextUndo() {
  return useQuery({ queryKey: NEXT_UNDO_QUERY_KEY, queryFn: askNextUndo });
}

/**
 * Asks again what Undo would do whenever that can have changed: after any
 * action, undo or send (each is a mutation that settled) and when a
 * background task ends. Never on a timer. Call it once, in the shell.
 */
export function useKeepNextUndoCurrent() {
  const queryClient = useQueryClient();
  useEffect(
    () =>
      queryClient.getMutationCache().subscribe((event) => {
        if (
          event.type === "updated" &&
          (event.action.type === "success" || event.action.type === "error")
        ) {
          void queryClient.invalidateQueries({ queryKey: NEXT_UNDO_QUERY_KEY });
          // Whatever took an operation back (the top bar, Ctrl+Z), the line
          // about "Accept all" describes that operation only while it stands.
          const outcome =
            event.action.type === "success" ? (event.action.data as UndoOutcome | undefined) : undefined;
          if (outcome?.status === "undone") {
            useAcceptedStore.getState().undone(outcome.operation.id);
            // The lines of the other screens ("Removed…", "Added…") go too.
            useUndoneCount.getState().undone();
          }
        }
      }),
    [queryClient],
  );
  useWhenJobsEnd(() => {
    void queryClient.invalidateQueries({ queryKey: NEXT_UNDO_QUERY_KEY });
  });
}

/**
 * The lists an undo can change: asked for again, so they show what's back.
 * Review is one of them: undoing a Review action brings its item back.
 */
function reloadLists(queryClient: QueryClient) {
  return Promise.all(
    [
      LIBRARY_TRACKS_QUERY_KEY,
      CRATES_QUERY_KEY,
      ALL_MUSIC_QUERY_KEY,
      REKORDBOX_OFFER_QUERY_KEY,
      MISSING_TRACKS_QUERY_KEY,
      TRACK_TAG_SOURCES_QUERY_KEY,
      OVERVIEW_QUERY_KEY,
      ...REVIEW_QUERY_KEYS,
    ].map((queryKey) => queryClient.invalidateQueries({ queryKey })),
  );
}

/**
 * Undoes one operation: the one given, and only if it's still the next to
 * undo, so what the button named is what's taken back.
 */
export function useUndoOperation() {
  const queryClient = useQueryClient();
  return useMutation<UndoOutcome, Error, number>({
    mutationFn: (operationId) => unwrap(commands.undoLastOperation(operationId)),
    onSettled: () => reloadLists(queryClient),
  });
}

/** Whether typing happens in `target`, where Ctrl+Z belongs to the text. */
function isTextField(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  if (target instanceof HTMLTextAreaElement) return true;
  if (target instanceof HTMLInputElement) {
    return ![
      "button",
      "checkbox",
      "color",
      "file",
      "image",
      "radio",
      "range",
      "reset",
      "submit",
    ].includes(target.type);
  }
  return target.closest('[contenteditable]:not([contenteditable="false"])') !== null;
}

/** A dialog that is waiting for an answer: it has the keyboard. */
const OPEN_DIALOG = '[role="alertdialog"], [role="dialog"], dialog[open]';

/**
 * Calls `onUndo` on Ctrl+Z, except while the focus is in a text field (there
 * Ctrl+Z undoes typing, as everywhere else) and while a dialog is open. A
 * held key counts once.
 */
export function useUndoShortcut(onUndo: () => void) {
  const latest = useRef(onUndo);
  useEffect(() => {
    latest.current = onUndo;
  });
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (!event.ctrlKey || event.shiftKey || event.altKey || event.metaKey) return;
      if (event.key.toLowerCase() !== "z" || isTextField(event.target)) return;
      if (document.querySelector(OPEN_DIALOG) !== null) return;
      event.preventDefault();
      if (!event.repeat) latest.current();
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, []);
}
