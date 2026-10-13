/**
 * The shell's zones that are drawn. A zone whose feature doesn't exist yet
 * is `false`: it isn't rendered at all (no "coming soon"), and its space
 * goes to the screen in the center. Its component, its strings and its
 * tests stay, so bringing it back is setting it to `true` here when the
 * feature is built:
 *
 * - `search`: the top bar's search and command palette (1.11, Ctrl+K);
 * - `details`: the Details panel on the right (the selected track);
 * - `player`: the preview player along the bottom (1.11), drawn since the
 *   preview engine (1cA-7).
 */
export interface Zones {
  search: boolean;
  details: boolean;
  player: boolean;
}

export const ZONES: Zones = { search: false, details: false, player: true };
