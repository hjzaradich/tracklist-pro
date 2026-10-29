import { useEffect, useRef } from "react";
import { create } from "zustand";
import { type PlayerFile, usePlayer } from "./playerStore";

/**
 * Previous and next track (1cB-10; ROADMAP 1.11, "Listening"). The tracks
 * to step through are the rows of the track table on screen, in the order
 * and filter it has now, so the table offers itself here
 * ({@link useOfferStepList}) and the player bar steps through it.
 *
 * A step starts from the table's selected row (the one the Details panel
 * shows; playing a row from its button selects it). It moves the selection
 * to the row it reaches, then plays that row's file from the start. Next
 * stops at the last row. Previous restarts the track when it is more than
 * {@link RESTART_AFTER} seconds in, and otherwise goes to the row above.
 * Rows that have no file are passed over.
 */
export const RESTART_AFTER = 3;

/** What a track table tells the player bar about its rows. */
export interface StepList {
  /** How many rows there are. */
  total: number;
  /** The selected row's index, if a row is selected. */
  focus: number | null;
  /** The file the row at `index` plays: `null` for a row with no file, `undefined` for a row that hasn't arrived. */
  fileAt: (index: number) => PlayerFile | null | undefined;
  /** Selects the row at `index` and brings it into view. */
  select: (index: number) => void;
}

/** The part the player bar draws from: reactive, and small. */
export const useStepPlace = create<{ total: number; focus: number | null }>(() => ({ total: 0, focus: null }));

/** The table's own functions: read when a step is made, so not state. */
let offered: Pick<StepList, "fileAt" | "select"> | null = null;

/**
 * For a track table on screen: lets the player bar step through its rows.
 * Gives them up when the table goes.
 */
export function useOfferStepList(list: StepList) {
  const latest = useRef(list);
  useEffect(() => {
    latest.current = list;
  });
  useEffect(() => {
    const mine = {
      fileAt: (index: number) => latest.current.fileAt(index),
      select: (index: number) => latest.current.select(index),
    };
    offered = mine;
    return () => {
      if (offered !== mine) return;
      offered = null;
      useStepPlace.setState({ total: 0, focus: null });
    };
  }, []);
  const { total, focus } = list;
  useEffect(() => {
    useStepPlace.setState({ total, focus });
  }, [total, focus]);
}

/** Whether a row below the selected one (the first row, when none is selected) can be stepped to. */
export function hasRowBelow({ total, focus }: { total: number; focus: number | null }): boolean {
  return focus === null ? total > 0 : focus < total - 1;
}

/** Whether there is a row above the selected one. */
export function hasRowAbove({ focus }: { focus: number | null }): boolean {
  return focus !== null && focus > 0;
}

function restartsTrack({ status, position }: { status: string; position: number }): boolean {
  return status === "ready" && position > RESTART_AFTER;
}

/** Whether Previous does anything: a row above, or a track far enough in to restart. */
export function canGoPrevious(
  place: { focus: number | null },
  player: { status: string; position: number },
): boolean {
  return hasRowAbove(place) || restartsTrack(player);
}

/**
 * Moves to the nearest row in `direction` that has a file, selects it in the
 * table and plays it from its start. Returns whether it moved. It stops at
 * the ends of the list, and at a row that hasn't arrived yet.
 */
function step(direction: 1 | -1): boolean {
  const table = offered;
  if (table === null) return false;
  const { total, focus } = useStepPlace.getState();
  // With no row selected, Next starts at the top and Previous has nowhere to go.
  let index = (focus ?? (direction === 1 ? -1 : 0)) + direction;
  while (index >= 0 && index < total) {
    const file = table.fileAt(index);
    if (file === undefined) return false;
    if (file !== null) {
      table.select(index);
      usePlayer.getState().select(file, { fresh: true });
      return true;
    }
    index += direction;
  }
  return false;
}

/** The next row's track. */
export function playNext(): boolean {
  return step(1);
}

/** Back to the start of the track when it is well under way, otherwise the row above. */
export function playPrevious(): boolean {
  const player = usePlayer.getState();
  if (restartsTrack(player)) {
    player.seek(0);
    return true;
  }
  return step(-1);
}
