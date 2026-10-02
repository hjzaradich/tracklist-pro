import { useEffect, useRef, useState } from "react";
import {
  isCancelledError,
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

// Undo (1bA-12): one step at a time, newest first, from the top bar, Ctrl+Z
// or the Undo offered right after an action. All three ask the Rust side,
// which keeps the history; nothing about it is remembered here.

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
        }
      }),
    [queryClient],
  );
  useWhenJobsEnd(() => {
    void queryClient.invalidateQueries({ queryKey: NEXT_UNDO_QUERY_KEY });
  });
}

/** The lists an undo can change: asked for again, so they show what's back. */
function reloadLists(queryClient: QueryClient) {
  return Promise.all(
    [
      LIBRARY_TRACKS_QUERY_KEY,
      CRATES_QUERY_KEY,
      ALL_MUSIC_QUERY_KEY,
      REKORDBOX_OFFER_QUERY_KEY,
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

/**
 * The Undo offered right after an action. Call `arm()` when the action has
 * finished, with the operation it recorded if the command said: `undo` takes
 * back only that one. Without it, `arm` asks which operation is then the
 * next to undo; until that's known `ready` is false and the caller keeps its
 * Undo greyed out (for good if the ask fails: the top bar's Undo is still
 * there), so a press can never take back whatever happens to be newest.
 * `offered` turns false once that operation isn't
 * the next to undo any more (the top bar's Undo or Ctrl+Z took it back, or
 * something else was done), so this Undo never takes back a different action.
 */
export function useUndoAfterAction() {
  const queryClient = useQueryClient();
  const next = useNextUndo();
  const [armed, setArmed] = useState<number | null>(null);
  const undo = useMutation<UndoOutcome>({
    mutationFn: () =>
      // Never the newest operation, whatever it is: only the one armed.
      armed === null
        ? Promise.resolve<UndoOutcome>({ status: "nothingToUndo" })
        : unwrap(commands.undoLastOperation(armed)),
    onSettled: () => reloadLists(queryClient),
  });
  // Whether the next step to undo has been asked for since arming. Only
  // an answer from after the action can say its operation isn't next any more.
  const [checked, setChecked] = useState(false);
  const arming = useRef(0);
  const arm = (operationId?: number | null) => {
    const known = typeof operationId === "number" ? operationId : null;
    const mine = ++arming.current;
    setArmed(known);
    setChecked(false);
    // The shell asks the same question when the action settles, which can
    // cancel this ask; then it's asked again.
    const ask = (): Promise<void> =>
      queryClient
        .fetchQuery({ queryKey: NEXT_UNDO_QUERY_KEY, queryFn: askNextUndo, staleTime: 0 })
        .then(
          (found) => {
            if (mine !== arming.current) return;
            if (known === null) setArmed(found.operation?.id ?? null);
            setChecked(true);
          },
          (error: unknown) => (isCancelledError(error) ? ask() : undefined),
        );
    void ask();
  };
  const offered = !checked || next.data === undefined || next.data.operation?.id === armed;
  return { arm, offered, ready: armed !== null, undo };
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
