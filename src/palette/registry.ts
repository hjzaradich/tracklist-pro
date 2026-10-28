import { useEffect, useRef } from "react";
import { create } from "zustand";

// The one list of actions the command palette (1cB-7) reads. Anything that
// can be done in one step registers its action here, from the component that
// owns the handler, so the palette runs the very handler the screen's own
// button runs and has no behavior of its own. The hotkeys (1cB-8) register
// the same way: an action with a `shortcut` is shown with it.

export interface PaletteAction {
  /** Unique among all actions. */
  id: string;
  /** What the palette lists and matches. */
  label: string;
  /** The keys that do the same, as shown beside the label. */
  shortcut?: string;
  /** False while it can't run: it is then not listed. */
  available?: boolean;
  run: () => void;
}

interface Registry {
  /** The actions each source registered, by source name. */
  sources: Readonly<Record<string, readonly PaletteAction[]>>;
  set: (source: string, actions: readonly PaletteAction[]) => void;
  clear: (source: string) => void;
}

export const usePaletteRegistry = create<Registry>((set) => ({
  sources: {},
  set: (source, actions) => set((state) => ({ sources: { ...state.sources, [source]: actions } })),
  clear: (source) =>
    set((state) => {
      const { [source]: _gone, ...rest } = state.sources;
      return { sources: rest };
    }),
}));

/**
 * Every action that can run now, by source name and then in the order the
 * source gave them, so the list doesn't depend on which component mounted
 * first. (The app's own come before Undo: the first row is never a change.)
 */
export function listedActions(sources: Registry["sources"]): PaletteAction[] {
  return Object.keys(sources)
    .sort()
    .flatMap((source) => sources[source] ?? [])
    .filter((action) => action.available !== false);
}

/**
 * Registers `actions` under `source` while the calling component is mounted.
 * The registry is told again only when an action's id, label, shortcut or
 * availability changes; a changed `run` is picked up without that, so the
 * palette always runs the handler of the latest render.
 */
export function useRegisterActions(source: string, actions: readonly PaletteAction[]) {
  const latest = useRef(actions);
  useEffect(() => {
    latest.current = actions;
  });
  const signature = JSON.stringify(
    actions.map(({ id, label, shortcut, available }) => [id, label, shortcut, available]),
  );
  // An update keeps the source's place in the list; only leaving removes it.
  useEffect(() => {
    usePaletteRegistry.getState().set(
      source,
      latest.current.map((action) => ({
        ...action,
        run: () => latest.current.find((now) => now.id === action.id)?.run(),
      })),
    );
  }, [source, signature]);
  useEffect(() => () => usePaletteRegistry.getState().clear(source), [source]);
}
