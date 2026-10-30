/**
 * Whether "Play next" is on (owner, 2026-10-30; ROADMAP 1.11 "Listening"),
 * kept per user in the webview's `localStorage`, like the volume. It is off
 * until the user turns it on, and the app works when storage is missing,
 * full or refuses to answer: it then starts off every time and says nothing.
 */
export const PLAY_NEXT_STORAGE_KEY = "tracklist-pro.player.playNext";

/** Where the choice is read from and written to. */
export interface PlayNextPrefs {
  load: () => boolean;
  save: (on: boolean) => void;
}

/** The part of `Storage` it uses. */
type Store = Pick<Storage, "getItem" | "setItem">;

/**
 * The choice kept in `storage`, which is asked for each time because even
 * reaching `localStorage` can throw. Anything but a stored `true` is off.
 */
export function playNextPrefs(storage: () => Store): PlayNextPrefs {
  return {
    load: () => {
      try {
        return storage().getItem(PLAY_NEXT_STORAGE_KEY) === "true";
      } catch {
        return false;
      }
    },
    save: (on) => {
      try {
        storage().setItem(PLAY_NEXT_STORAGE_KEY, String(on));
      } catch {
        // Not worth stopping for: the choice is just forgotten at the next start.
      }
    },
  };
}

/** The app's choice, kept in the webview's `localStorage`. */
export const browserPlayNextPrefs: PlayNextPrefs = playNextPrefs(() => localStorage);
