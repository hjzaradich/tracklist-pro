import { useEffect, useRef } from "react";
import { create } from "zustand";
import { type PlayerFile, usePlayer } from "./playerStore";

/**
 * Previous and next track (1cB-10; ROADMAP 1.11, "Listening"). The tracks
 * to step through are the rows of the track table on screen, in the order
 * and filter it has now, so the table offers itself here
 * ({@link useOfferStepList}) and the player bar steps through it.
 *
 * A step starts from the row of the track that is playing, wherever the
 * table has put it (after a sort or a filter too), and from the table's
 * selected row only when the playing track is not in the table. It moves
 * the selection to the row it reaches, then plays that row's file from the
 * start. Next stops at the last row. Previous restarts the track when it is
 * more than {@link RESTART_AFTER} seconds in, and otherwise goes to the row
 * above. Rows that have no file are passed over; a row whose window has not
 * arrived yet is scrolled to, and played once it has.
 */
export const RESTART_AFTER = 3;

/** What a track table tells the player bar about its rows. */
export interface StepList {
  /** How many rows there are. */
  total: number;
  /** The selected row's index, if a row is selected. */
  focus: number | null;
  /** Changes when the rows are another order or another list. */
  listKey: string;
  /** The file the row at `index` plays: `null` for a row with no file, `undefined` for a row that hasn't arrived. */
  fileAt: (index: number) => PlayerFile | null | undefined;
  /** Selects the row at `index` and brings it into view (so its window is fetched). */
  select: (index: number) => void;
  /** Where the track that `file` was chosen from is in the rows now; `null` if it is not one of them. */
  locate: (file: PlayerFile) => number | null | Promise<number | null>;
}

interface Place {
  total: number;
  /** The selected row. */
  focus: number | null;
  /** The playing track's row. */
  playing: number | null;
}

/** The part the player bar draws from: reactive, and small. */
export const useStepPlace = create<Place>(() => ({ total: 0, focus: null, playing: null }));

const NO_PLACE: Place = { total: 0, focus: null, playing: null };

/** The table's own functions: read when a step is made, so not state. */
type Offer = Pick<StepList, "fileAt" | "select" | "locate"> & { listKey: () => string };
let offered: Offer | null = null;
/** Goes up with every step, so an answer for an older one is dropped. */
let stepping = 0;
/** A step that reached a row not loaded yet, and waits for it. */
let pending: { index: number; direction: 1 | -1; listKey: string } | null = null;

/** Where steps start: the playing track's row, else the selected one. */
function anchorOf({ focus, playing }: Pick<Place, "focus" | "playing">): number | null {
  return playing ?? focus;
}

/** Moves to the nearest row from `index` in `direction` that has a file, and plays it. */
function walk(table: Offer, from: number, direction: 1 | -1): void {
  const { total } = useStepPlace.getState();
  for (let index = from; index >= 0 && index < total; index += direction) {
    const file = table.fileAt(index);
    if (file === undefined) {
      // Not fetched yet: scrolling to the row fetches it, and it is played on arrival.
      pending = { index, direction, listKey: table.listKey() };
      table.select(index);
      return;
    }
    if (file !== null) {
      pending = null;
      table.select(index);
      usePlayer.getState().select(file, { fresh: true });
      return;
    }
  }
  pending = null;
}

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
    const mine: Offer = {
      fileAt: (index) => latest.current.fileAt(index),
      select: (index) => latest.current.select(index),
      locate: (file) => latest.current.locate(file),
      listKey: () => latest.current.listKey,
    };
    offered = mine;
    return () => {
      if (offered !== mine) return;
      offered = null;
      pending = null;
      useStepPlace.setState(NO_PLACE);
    };
  }, []);
  const { total, focus, listKey } = list;
  useEffect(() => {
    useStepPlace.setState({ total, focus });
  }, [total, focus]);

  // Where the playing track is, for the bar's buttons: worked out again when
  // the track, the rows' order or the selection change.
  const playingId = usePlayer((state) => state.file?.id ?? null);
  useEffect(() => {
    const file = usePlayer.getState().file;
    if (file === null) {
      useStepPlace.setState({ playing: null });
      return;
    }
    let current = true;
    const found = latest.current.locate(file);
    if (found instanceof Promise) {
      found.then(
        (index) => current && useStepPlace.setState({ playing: index }),
        () => current && useStepPlace.setState({ playing: null }),
      );
    } else {
      useStepPlace.setState({ playing: found });
    }
    return () => {
      current = false;
    };
  }, [playingId, total, focus, listKey]);

  // A step that was waiting for a row carries on once the row has arrived.
  // It is dropped if the selection or the list moved on meanwhile.
  useEffect(() => {
    if (pending === null || offered === null) return;
    if (pending.listKey !== listKey || focus !== pending.index || pending.index >= total) {
      pending = null;
      return;
    }
    if (list.fileAt(pending.index) === undefined) return;
    const { index, direction } = pending;
    walk(offered, index, direction);
  });
}

/** Whether a row below the playing track's (or the selected one, or the top if none) can be stepped to. */
export function hasRowBelow(place: Place): boolean {
  const anchor = anchorOf(place);
  return anchor === null ? place.total > 0 : anchor < place.total - 1;
}

/** Whether there is a row above the playing track's (or the selected one). */
export function hasRowAbove(place: Place): boolean {
  const anchor = anchorOf(place);
  return anchor !== null && anchor > 0;
}

function restartsTrack({ status, position }: { status: string; position: number }): boolean {
  return status === "ready" && position > RESTART_AFTER;
}

/** Whether Previous does anything: a row above, or a track far enough in to restart. */
export function canGoPrevious(place: Place, player: { status: string; position: number }): boolean {
  return hasRowAbove(place) || restartsTrack(player);
}

function step(direction: 1 | -1): void {
  const table = offered;
  if (table === null) return;
  pending = null;
  const mine = ++stepping;
  const go = (found: number | null) => {
    if (mine !== stepping || offered !== table) return;
    const { focus } = useStepPlace.getState();
    // With no row to start from, Next starts at the top and Previous has nowhere to go.
    const from = found ?? focus ?? (direction === 1 ? -1 : 0);
    walk(table, from + direction, direction);
  };
  const file = usePlayer.getState().file;
  const located = file === null ? null : table.locate(file);
  if (located instanceof Promise) located.then(go, () => go(null));
  else go(located);
}

/** The next row's track. */
export function playNext(): void {
  step(1);
}

/** Back to the start of the track when it is well under way, otherwise the row above. */
export function playPrevious(): void {
  const player = usePlayer.getState();
  if (restartsTrack(player)) {
    player.seek(0);
    return;
  }
  step(-1);
}
