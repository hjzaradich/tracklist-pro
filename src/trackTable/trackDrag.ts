import { create } from "zustand";

/**
 * Tracks being dragged out of a track table (1cB-13; ROADMAP 1.14, "Adding"):
 * the table starts the drag ({@link useTrackDrag}`.begin`) and ends it; the
 * sidebar's crates read it to light up and to take the drop. It is made of
 * pointer events, not the browser's drag and drop, which the webview of the
 * window can swallow, and which the column headings (1cB-21) don't use either.
 */
interface TrackDragState {
  /** The tracks being dragged (`TableRow.recordingId`), or `null` when no drag is on. */
  ids: readonly number[] | null;
  /** Where the pointer is, in the window. */
  x: number;
  y: number;
  /** The crate or folder the pointer is over in the sidebar, if any. */
  over: number | null;
  begin: (ids: readonly number[], x: number, y: number) => void;
  moveTo: (x: number, y: number) => void;
  hover: (over: number | null) => void;
  end: () => void;
}

export const useTrackDrag = create<TrackDragState>((set) => ({
  ids: null,
  x: 0,
  y: 0,
  over: null,
  begin: (ids, x, y) => set({ ids, x, y, over: null }),
  moveTo: (x, y) => set({ x, y }),
  hover: (over) => set({ over }),
  end: () => set({ ids: null, over: null }),
}));
