import { convertFileSrc } from "@tauri-apps/api/core";
import { create } from "zustand";
import { commands, type PreviewRefusal } from "../bindings";
import { browserVolumePrefs, clampLevel, DEFAULT_VOLUME, type VolumePrefs } from "./volumePrefs";

/** The protocol the Rust side serves previews on (`audio_preview::SCHEME`). */
export const PREVIEW_SCHEME = "audio-preview";

/** A file the player can be given: its id and the name to show. */
export interface PlayerFile {
  id: number;
  name: string;
  /**
   * The track the file was chosen from, when it was chosen from a track
   * table: the player bar's previous and next find the track's row again
   * from it, in whatever order the table has now (1cB-10).
   */
  recordingId?: number;
}

/**
 * - `idle`: no file chosen yet;
 * - `loading`: the file is being checked and opened;
 * - `ready`: the file is loaded (playing or paused);
 * - `refused`: the file can't be played; `refusal` says why.
 */
export type PlayerStatus = "idle" | "loading" | "ready" | "refused";

export interface PlayerState {
  file: PlayerFile | null;
  status: PlayerStatus;
  /** Why the file can't be played, when `status` is `refused`. */
  refusal: PreviewRefusal | null;
  /** Seconds. Carried to the next file when the file changes. */
  position: number;
  /** Seconds; 0 until the file's length is known. */
  duration: number;
  /**
   * Whether the user wants it playing: what the button shows. It survives a
   * file that can't be played, so the next file carries on the same way.
   */
  playing: boolean;
  /** 0 to 1; kept per user (`volumePrefs.ts`). */
  volume: number;
  /** Whether the sound is off with the volume kept as it was. */
  muted: boolean;
  /**
   * Plays `file`. The first file starts playing; after that a new file
   * keeps what the last one was doing: the same position (by time; a file
   * shorter than that starts from its beginning), and playing or paused as
   * it was. The file already loaded is left as it is; one that couldn't be
   * played is checked again.
   *
   * `fresh` is for stepping to another track (the player bar's next and
   * previous, 1cB-10): it starts the file from its beginning, playing,
   * whatever the last file was doing. Carrying the position is for
   * comparing the files of one track, not for moving on to the next one.
   */
  select: (file: PlayerFile, options?: { fresh?: boolean }) => void;
  /** Play or pause. */
  toggle: () => void;
  /** Moves to `seconds` in the loaded file. */
  seek: (seconds: number) => void;
  /**
   * Sets the volume (0 to 1); sound that was muted comes back. It is kept
   * for the next run at once, unless `keep` is false (a slider being
   * dragged: {@link PlayerState.keepVolume} is called when it is let go).
   */
  setVolume: (level: number, keep?: boolean) => void;
  /** Keeps the volume and mute as they are now for the next run. */
  keepVolume: () => void;
  /** Mutes, or brings the sound back at the volume it had (full if that was none). */
  toggleMute: () => void;
}

/** The part of an audio element the player uses, so a test can stand in. */
export interface AudioLike {
  currentTime: number;
  readonly duration: number;
  volume: number;
  muted: boolean;
  src: string;
  play: () => Promise<void>;
  pause: () => void;
  load: () => void;
  removeAttribute: (name: string) => void;
  addEventListener: (type: string, listener: () => void) => void;
}

export interface PlayerDeps {
  /** The one audio element; asked for when the first file is chosen. */
  audio: () => AudioLike;
  /** Whether a file plays, and why not (`preview_check`). */
  check: (fileId: number) => Promise<PreviewRefusal | null>;
  /** The address a file is played from. */
  urlOf: (fileId: number) => string;
  /** Where the volume is kept between runs; without it the volume starts at full each time. */
  volume?: VolumePrefs;
}

/**
 * The player's state and rules. There is one player for the whole app
 * ({@link usePlayer}): one audio element, so one file is audible at a time.
 * Only the position, the length and play/pause live here: the Rust side
 * keeps nothing.
 */
export function createPlayer(deps: PlayerDeps) {
  let element: AudioLike | null = null;
  /** Goes up with every file chosen, so a late answer for an old file is dropped. */
  let choice = 0;
  /** Where to start in the file being opened. */
  let startAt = 0;

  const store = create<PlayerState>()((set, get) => {
    const audio = (): AudioLike => {
      if (element === null) {
        element = deps.audio();
        element.volume = get().volume;
        element.muted = get().muted;
        wire(element);
      }
      return element;
    };

    const refuse = (refusal: PreviewRefusal) => {
      const a = audio();
      a.pause();
      a.removeAttribute("src");
      set({ status: "refused", refusal, duration: 0 });
    };

    const play = () => {
      audio()
        .play()
        .catch((error: unknown) => {
          // Loading another file interrupts a play(): not the user stopping.
          if (error instanceof Error && error.name === "AbortError") return;
          set({ playing: false });
        });
    };

    const wire = (a: AudioLike) => {
      a.addEventListener("loadedmetadata", () => {
        const { status, playing } = get();
        if (status !== "loading") return;
        const duration = Number.isFinite(a.duration) ? a.duration : 0;
        // Time carries over; a file shorter than that starts at the beginning.
        const position = duration > 0 && startAt < duration ? startAt : 0;
        a.currentTime = position;
        set({ status: "ready", duration, position });
        if (playing) play();
      });
      a.addEventListener("durationchange", () => {
        if (get().status === "ready" && Number.isFinite(a.duration)) set({ duration: a.duration });
      });
      a.addEventListener("timeupdate", () => {
        if (get().status === "ready") set({ position: a.currentTime });
      });
      a.addEventListener("ended", () => {
        if (get().status === "ready") set({ playing: false, position: get().duration });
      });
      a.addEventListener("error", () => {
        const { file, status } = get();
        if (file === null || (status !== "loading" && status !== "ready")) return;
        const mine = choice;
        // A media element can't say why; the Rust side can.
        deps
          .check(file.id)
          .catch(() => "unreadable" as const)
          .then((refusal) => {
            if (mine === choice) refuse(refusal ?? "not_decodable");
          });
      });
    };

    const kept = deps.volume?.load() ?? DEFAULT_VOLUME;

    /** Sets the volume and mute on the element (once there is one) and keeps them. */
    const sound = (level: number, muted: boolean, keep = true) => {
      if (element !== null) {
        element.volume = level;
        element.muted = muted;
      }
      set({ volume: level, muted });
      if (keep) deps.volume?.save({ level, muted });
    };

    return {
      file: null,
      status: "idle",
      refusal: null,
      position: 0,
      duration: 0,
      playing: false,
      volume: clampLevel(kept.level),
      muted: kept.muted,

      select: (file, options) => {
        const before = get();
        // The file already loaded is left alone. One that was refused is
        // asked about again: the drive may be connected by now.
        if (before.file?.id === file.id && before.status !== "refused") return;
        const a = audio();
        const first = before.file === null;
        const fresh = options?.fresh === true;
        const position = fresh ? 0 : before.status === "ready" ? a.currentTime : before.position;
        const playing = first || fresh ? true : before.playing;
        const mine = ++choice;
        startAt = position;
        // One file is audible at a time: the old one stops at once.
        a.pause();
        set({ file, status: "loading", refusal: null, position, duration: 0, playing });
        deps
          .check(file.id)
          .catch(() => "unreadable" as const)
          .then((refusal) => {
            if (mine !== choice) return;
            if (refusal !== null) {
              refuse(refusal);
              return;
            }
            a.src = deps.urlOf(file.id);
            a.load();
          });
      },

      toggle: () => {
        const { status, playing } = get();
        if (status === "idle") return;
        set({ playing: !playing });
        // While loading, or refused, the choice is kept for the next file.
        if (status !== "ready") return;
        if (playing) audio().pause();
        else play();
      },

      seek: (seconds) => {
        const { status, duration } = get();
        if (status !== "ready") return;
        const to = Math.min(Math.max(seconds, 0), duration);
        audio().currentTime = to;
        set({ position: to });
      },

      setVolume: (level, keep = true) => sound(clampLevel(level), false, keep),

      keepVolume: () => deps.volume?.save({ level: get().volume, muted: get().muted }),

      toggleMute: () => {
        const { volume, muted } = get();
        if (muted || volume === 0) sound(volume === 0 ? DEFAULT_VOLUME.level : volume, false);
        else sound(volume, true);
      },
    };
  });

  return store;
}

/** The app's one player. */
export const usePlayer = createPlayer({
  audio: () => new Audio(),
  check: (fileId) => commands.previewCheck(fileId),
  urlOf: (fileId) => convertFileSrc(String(fileId), PREVIEW_SCHEME),
  volume: browserVolumePrefs,
});
