import { create } from "zustand";
import type { AcceptedAll } from "../bindings";

/** What the last "Accept all" switched, for the line at the top of Review (1bE-3). */
export interface Accepted {
  /** The one operation Undo takes back. */
  operationId: number;
  switched: AcceptedAll["switched"];
}

interface AcceptedState {
  accepted: Accepted | null;
  /** The next "Accept all" replaces the line; one that switched nothing leaves it as it is. */
  record: (result: AcceptedAll) => void;
  /** The user closed the line, or its Undo took the switch back. */
  dismiss: () => void;
}

/**
 * The line stays until it is dismissed or the next "Accept all" (owner
 * decision, 2026-10-13), so it outlives the screen: leaving Review and coming
 * back shows it still.
 */
export const useAcceptedStore = create<AcceptedState>((set) => ({
  accepted: null,
  record: (result) => {
    if (result.operationId === null || result.switched.length === 0) return;
    set({ accepted: { operationId: result.operationId, switched: result.switched } });
  },
  dismiss: () => set({ accepted: null }),
}));
