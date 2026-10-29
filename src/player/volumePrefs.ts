/**
 * The player's volume and mute, kept per user (1.11, "Listening"). They live
 * in the webview's `localStorage`, like the theme, and the app works when
 * storage is missing, full or refuses to answer: it then starts at full
 * volume every time and says nothing.
 */
export const VOLUME_STORAGE_KEY = "tracklist-pro.player.volume";

export interface Volume {
  /** 0 (silent) to 1 (full). */
  level: number;
  muted: boolean;
}

export const DEFAULT_VOLUME: Volume = { level: 1, muted: false };

/** Where the volume is read from and written to. */
export interface VolumePrefs {
  load: () => Volume;
  save: (volume: Volume) => void;
}

/** `level` kept between 0 and 1; anything that isn't a number is full volume. */
export function clampLevel(level: number): number {
  if (!Number.isFinite(level)) return DEFAULT_VOLUME.level;
  return Math.min(1, Math.max(0, level));
}

/** The part of `Storage` the volume uses. */
type Store = Pick<Storage, "getItem" | "setItem">;

/**
 * The volume kept in `storage`. `storage` is asked for each time, because
 * even reaching `localStorage` can throw (blocked site data). A stored
 * value that is damaged is ignored.
 */
export function volumePrefs(storage: () => Store): VolumePrefs {
  return {
    load: () => {
      try {
        const raw = storage().getItem(VOLUME_STORAGE_KEY);
        if (raw === null) return DEFAULT_VOLUME;
        const stored = JSON.parse(raw) as { level?: unknown; muted?: unknown } | null;
        if (typeof stored?.level !== "number" || !Number.isFinite(stored.level)) return DEFAULT_VOLUME;
        return { level: clampLevel(stored.level), muted: stored.muted === true };
      } catch {
        return DEFAULT_VOLUME;
      }
    },
    save: (volume) => {
      try {
        storage().setItem(VOLUME_STORAGE_KEY, JSON.stringify(volume));
      } catch {
        // Not worth stopping for: the volume is just forgotten at the next start.
      }
    },
  };
}

/** The app's volume, kept in the webview's `localStorage`. */
export const browserVolumePrefs: VolumePrefs = volumePrefs(() => localStorage);
