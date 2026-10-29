import { describe, expect, it, vi } from "vitest";
import type { PreviewRefusal } from "../bindings";
import { createPlayer, type AudioLike } from "./playerStore";
import { VOLUME_STORAGE_KEY, volumePrefs, type VolumePrefs } from "./volumePrefs";

/** An audio element that does what a test tells it: nothing plays. */
class FakeAudio implements AudioLike {
  currentTime = 0;
  duration = Number.NaN;
  volume = 1;
  muted = false;
  src = "";
  /** What it was asked to do, in order. */
  log: string[] = [];
  /** Play requests that were answered. */
  plays = 0;
  playError: Error | null = null;
  private listeners = new Map<string, (() => void)[]>();

  play() {
    this.log.push("play");
    this.plays += 1;
    return this.playError ? Promise.reject(this.playError) : Promise.resolve();
  }
  pause() {
    this.log.push("pause");
  }
  load() {
    this.log.push(`load ${this.src}`);
  }
  removeAttribute(name: string) {
    this.log.push(`remove ${name}`);
    if (name === "src") this.src = "";
  }
  addEventListener(type: string, listener: () => void) {
    this.listeners.set(type, [...(this.listeners.get(type) ?? []), listener]);
  }
  /** What the browser does once the file's header is read. */
  loaded(duration: number) {
    this.duration = duration;
    this.fire("loadedmetadata");
  }
  fire(type: string) {
    for (const listener of this.listeners.get(type) ?? []) listener();
  }
  /** Time passes in the file. */
  playTo(seconds: number) {
    this.currentTime = seconds;
    this.fire("timeupdate");
  }
}

/** The player over a fake element. `refusals` says what each file's check answers. */
function setup(refusals: Record<number, PreviewRefusal | null> = {}, volume?: VolumePrefs) {
  const audio = new FakeAudio();
  let made = 0;
  const checked: number[] = [];
  const held: Partial<Record<number, () => void>> = {};
  const player = createPlayer({
    audio: () => {
      made += 1;
      return audio;
    },
    check: (id) => {
      checked.push(id);
      const answer = refusals[id] ?? null;
      const hold = held[id];
      return hold ? new Promise((resolve) => (held[id] = () => resolve(answer))) : Promise.resolve(answer);
    },
    urlOf: (id) => `media://${id}`,
    volume,
  });
  return { audio, player, checked, made: () => made, held, state: () => player.getState() };
}

const A = { id: 1, name: "Track A" };
const B = { id: 2, name: "Track B" };
const C = { id: 3, name: "Track C" };

/** Lets the checks that were asked for answer. */
const settle = () => new Promise((resolve) => setTimeout(resolve, 0));

/** Opens `file` and loads it as the browser would, `duration` seconds long. */
async function open(t: ReturnType<typeof setup>, file: typeof A, duration: number) {
  t.state().select(file);
  await settle();
  t.audio.loaded(duration);
}

describe("the player", () => {
  it("is one player: one audio element however many files are chosen", async () => {
    const t = setup();
    await open(t, A, 100);
    await open(t, B, 100);
    await open(t, C, 100);
    expect(t.made()).toBe(1);
  });

  it("makes one file audible at a time: the old one stops before the next is opened", async () => {
    const t = setup();
    await open(t, A, 100);
    t.audio.log.length = 0;
    t.state().select(B);
    // Stopped at once, before anything is known about the next file.
    expect(t.audio.log).toEqual(["pause"]);
    await settle();
    expect(t.audio.src).toBe("media://2");
  });

  it("starts the first file it is given, on its metadata", async () => {
    const t = setup();
    expect(t.state().status).toBe("idle");
    t.state().select(A);
    expect(t.state().status).toBe("loading");
    await settle();
    expect(t.audio.src).toBe("media://1");
    t.audio.loaded(200);
    expect(t.state()).toMatchObject({ status: "ready", duration: 200, position: 0, playing: true });
    expect(t.audio.plays).toBe(1);
  });

  it("plays and pauses", async () => {
    const t = setup();
    await open(t, A, 100);
    t.audio.log.length = 0;
    t.state().toggle();
    expect(t.state().playing).toBe(false);
    expect(t.audio.log).toEqual(["pause"]);
    t.state().toggle();
    expect(t.state().playing).toBe(true);
    expect(t.audio.log).toEqual(["pause", "play"]);
  });

  it("scrubs: moves to a time, and never past either end", async () => {
    const t = setup();
    await open(t, A, 100);
    t.state().seek(42.5);
    expect(t.audio.currentTime).toBe(42.5);
    expect(t.state().position).toBe(42.5);
    t.state().seek(500);
    expect(t.audio.currentTime).toBe(100);
    t.state().seek(-3);
    expect(t.audio.currentTime).toBe(0);
  });

  it("follows the playback position", async () => {
    const t = setup();
    await open(t, A, 100);
    t.audio.playTo(12.25);
    expect(t.state().position).toBe(12.25);
  });

  it("stops at the end of the file", async () => {
    const t = setup();
    await open(t, A, 100);
    t.audio.playTo(100);
    t.audio.fire("ended");
    expect(t.state()).toMatchObject({ playing: false, position: 100 });
  });

  it("choosing the file that is already loaded changes nothing", async () => {
    const t = setup();
    await open(t, A, 100);
    t.audio.playTo(30);
    t.audio.log.length = 0;
    t.state().select(A);
    expect(t.audio.log).toEqual([]);
    expect(t.checked).toEqual([1]);
  });
});

describe("switching files while listening", () => {
  it("carries the position by time: 1:30 stays 1:30", async () => {
    const t = setup();
    await open(t, A, 300);
    t.audio.playTo(90);
    await open(t, B, 400);
    expect(t.audio.currentTime).toBe(90);
    expect(t.state()).toMatchObject({ status: "ready", position: 90, duration: 400 });
  });

  it("starts a file shorter than the position from its beginning", async () => {
    const t = setup();
    await open(t, A, 300);
    t.audio.playTo(200);
    await open(t, B, 120);
    expect(t.audio.currentTime).toBe(0);
    expect(t.state().position).toBe(0);
    // A file exactly as long as the position is shorter than it, too.
    t.audio.playTo(60);
    await open(t, C, 60);
    expect(t.audio.currentTime).toBe(0);
  });

  it("keeps playing what was playing", async () => {
    const t = setup();
    await open(t, A, 300);
    t.audio.playTo(10);
    t.audio.plays = 0;
    await open(t, B, 300);
    expect(t.state().playing).toBe(true);
    expect(t.audio.plays).toBe(1);
  });

  it("keeps paused what was paused", async () => {
    const t = setup();
    await open(t, A, 300);
    t.audio.playTo(10);
    t.state().toggle();
    t.audio.plays = 0;
    await open(t, B, 300);
    expect(t.state()).toMatchObject({ playing: false, status: "ready", position: 10 });
    expect(t.audio.plays).toBe(0);
  });

  it("carries the position of a paused file by time too", async () => {
    const t = setup();
    await open(t, A, 300);
    t.state().seek(75);
    t.state().toggle();
    await open(t, B, 300);
    expect(t.audio.currentTime).toBe(75);
  });

  it("keeps position and play state across a file that can't be played", async () => {
    const t = setup({ 2: "missing" });
    await open(t, A, 300);
    t.audio.playTo(90);
    t.state().select(B);
    await settle();
    expect(t.state()).toMatchObject({ status: "refused", refusal: "missing", position: 90, playing: true });
    expect(t.audio.src).toBe("");
    await open(t, C, 300);
    expect(t.audio.currentTime).toBe(90);
    expect(t.state().playing).toBe(true);
  });

  it("answers only the last file chosen when the checks come back out of order", async () => {
    const t = setup();
    t.held[2] = () => {};
    t.state().select(B);
    await settle();
    t.state().select(C);
    await settle();
    expect(t.audio.src).toBe("media://3");
    // B's answer arrives late and is dropped.
    t.held[2]?.();
    await settle();
    expect(t.audio.src).toBe("media://3");
    expect(t.state().file).toEqual(C);
  });
});

describe("a file that can't be played", () => {
  const codes: PreviewRefusal[] = [
    "unknown_file",
    "missing",
    "drive_not_connected",
    "online_only",
    "unreadable",
    "not_decodable",
  ];

  it.each(codes)("is refused as %s and nothing plays", async (code) => {
    const t = setup({ 1: code });
    t.state().select(A);
    await settle();
    expect(t.state()).toMatchObject({ status: "refused", refusal: code, duration: 0 });
    expect(t.audio.plays).toBe(0);
    expect(t.audio.src).toBe("");
    // Nothing to scrub.
    t.state().seek(30);
    expect(t.audio.currentTime).toBe(0);
  });

  it("is refused when the element fails to load it, with the reason the Rust side gives", async () => {
    // The drive went away while it played: the element only says "error".
    const refusals: Record<number, PreviewRefusal | null> = {};
    const t = setup(refusals);
    await open(t, A, 100);
    refusals[1] = "drive_not_connected";
    t.audio.fire("error");
    await settle();
    expect(t.state()).toMatchObject({ status: "refused", refusal: "drive_not_connected" });
    expect(t.checked).toEqual([1, 1]);
  });

  it("is can't-be-decoded when the element fails but the check passes", async () => {
    const t = setup();
    t.state().select(A);
    await settle();
    t.audio.fire("error");
    await settle();
    expect(t.state()).toMatchObject({ status: "refused", refusal: "not_decodable" });
  });

  it("is refused as unreadable when the check itself fails", async () => {
    const audio = new FakeAudio();
    const player = createPlayer({
      audio: () => audio,
      check: () => Promise.reject(new Error("the IPC call failed")),
      urlOf: (id) => `media://${id}`,
    });
    player.getState().select(A);
    await settle();
    expect(player.getState()).toMatchObject({ status: "refused", refusal: "unreadable" });
  });

  it("is asked about again when it is chosen again, and plays once it can", async () => {
    // The drive was unplugged; it is connected by the second try.
    const refusals: Record<number, PreviewRefusal | null> = { 1: "drive_not_connected" };
    const t = setup(refusals);
    t.state().select(A);
    await settle();
    expect(t.state()).toMatchObject({ status: "refused", refusal: "drive_not_connected" });
    expect(t.audio.src).toBe("");

    refusals[1] = null;
    t.state().select(A);
    expect(t.state()).toMatchObject({ status: "loading", refusal: null });
    await settle();
    expect(t.checked).toEqual([1, 1]);
    expect(t.audio.src).toBe("media://1");
    t.audio.loaded(100);
    expect(t.state()).toMatchObject({ status: "ready", playing: true });
    expect(t.audio.plays).toBe(1);
  });

  it("keeps the play/pause choice, so the next file carries on", async () => {
    const t = setup({ 1: "online_only" });
    t.state().select(A);
    await settle();
    t.state().toggle();
    expect(t.state().playing).toBe(false);
    await open(t, B, 100);
    expect(t.state()).toMatchObject({ status: "ready", playing: false });
    expect(t.audio.plays).toBe(0);
  });

  it("does not turn playing off when a play request is only interrupted", async () => {
    const t = setup();
    t.audio.playError = Object.assign(new Error("interrupted"), { name: "AbortError" });
    await open(t, A, 100);
    await settle();
    expect(t.state().playing).toBe(true);
    // But a refused play (no permission to start sound) is the user not hearing it.
    const blocked = setup();
    blocked.audio.playError = Object.assign(new Error("blocked"), { name: "NotAllowedError" });
    await open(blocked, A, 100);
    await settle();
    expect(blocked.state().playing).toBe(false);
  });
});

describe("stepping to another track", () => {
  it("starts the new file from its beginning and playing, though the last was paused at 1:30", async () => {
    const t = setup();
    await open(t, A, 300);
    t.state().seek(90);
    t.state().toggle();
    expect(t.state()).toMatchObject({ playing: false, position: 90 });
    t.state().select(B, { fresh: true });
    await settle();
    t.audio.loaded(300);
    expect(t.state()).toMatchObject({ file: B, status: "ready", position: 0, playing: true });
    expect(t.audio.currentTime).toBe(0);
    expect(t.audio.plays).toBe(2);
  });

  it("leaves switching between the files of one track as it was: the position carries", async () => {
    const t = setup();
    await open(t, A, 300);
    t.state().seek(90);
    t.state().select(B);
    await settle();
    t.audio.loaded(300);
    expect(t.state()).toMatchObject({ file: B, position: 90, playing: true });
  });
});

/** A storage that keeps what it is given, as the webview's does. */
function memoryStorage() {
  const kept = new Map<string, string>();
  return {
    getItem: (key: string) => kept.get(key) ?? null,
    setItem: (key: string, value: string) => void kept.set(key, value),
  };
}

describe("volume and mute", () => {
  it("sets the volume on the element, never outside 0 to 1", async () => {
    const t = setup();
    await open(t, A, 100);
    t.state().setVolume(0.4);
    expect(t.state().volume).toBe(0.4);
    expect(t.audio.volume).toBe(0.4);
    t.state().setVolume(7);
    expect(t.audio.volume).toBe(1);
    t.state().setVolume(-1);
    expect(t.audio.volume).toBe(0);
  });

  it("can follow a slider without keeping every step, and keeps the volume when told to", async () => {
    const storage = memoryStorage();
    const t = setup({}, volumePrefs(() => storage));
    await open(t, A, 100);
    t.state().setVolume(0.2, false);
    t.state().setVolume(0.5, false);
    expect(t.audio.volume).toBe(0.5);
    expect(storage.getItem(VOLUME_STORAGE_KEY)).toBeNull();
    t.state().keepVolume();
    expect(JSON.parse(storage.getItem(VOLUME_STORAGE_KEY) ?? "null")).toEqual({ level: 0.5, muted: false });
  });

  it("mutes without losing the volume, and brings the sound back at the same level", async () => {
    const t = setup();
    await open(t, A, 100);
    t.state().setVolume(0.4);
    t.state().toggleMute();
    expect(t.state()).toMatchObject({ muted: true, volume: 0.4 });
    expect(t.audio.muted).toBe(true);
    t.state().toggleMute();
    expect(t.state()).toMatchObject({ muted: false, volume: 0.4 });
    expect(t.audio.muted).toBe(false);
    expect(t.audio.volume).toBe(0.4);
  });

  it("brings the sound back when the volume is moved while muted", async () => {
    const t = setup();
    await open(t, A, 100);
    t.state().toggleMute();
    t.state().setVolume(0.7);
    expect(t.state()).toMatchObject({ muted: false, volume: 0.7 });
    expect(t.audio.muted).toBe(false);
  });

  it("un-muting a volume that was turned all the way down gives full volume, not silence", async () => {
    const t = setup();
    await open(t, A, 100);
    t.state().setVolume(0);
    t.state().toggleMute();
    expect(t.state()).toMatchObject({ muted: false, volume: 1 });
    expect(t.audio.volume).toBe(1);
  });

  it("applies a volume set before any file was chosen when the element is made", async () => {
    const t = setup();
    t.state().setVolume(0.25);
    t.state().toggleMute();
    expect(t.made()).toBe(0);
    await open(t, A, 100);
    expect(t.audio.volume).toBe(0.25);
    expect(t.audio.muted).toBe(true);
  });

  it("survives a reload: a new player starts at the volume and mute the last one ended with", async () => {
    const storage = memoryStorage();
    const first = setup({}, volumePrefs(() => storage));
    first.state().setVolume(0.3);
    first.state().toggleMute();
    expect(JSON.parse(storage.getItem(VOLUME_STORAGE_KEY) ?? "null")).toEqual({ level: 0.3, muted: true });

    const reloaded = setup({}, volumePrefs(() => storage));
    expect(reloaded.state()).toMatchObject({ volume: 0.3, muted: true });
    await open(reloaded, A, 100);
    expect(reloaded.audio.volume).toBe(0.3);
    expect(reloaded.audio.muted).toBe(true);
  });

  it("writes nothing to storage until the volume is changed", () => {
    const storage = memoryStorage();
    const setItem = vi.spyOn(storage, "setItem");
    setup({}, volumePrefs(() => storage));
    expect(setItem).not.toHaveBeenCalled();
  });

  it("starts at full volume, unmuted, when nothing was kept or what was kept is damaged", () => {
    const storage = memoryStorage();
    expect(setup({}, volumePrefs(() => storage)).state()).toMatchObject({ volume: 1, muted: false });
    for (const damaged of ["{", "null", '"loud"', '{"level":"high"}', '{"level":null}']) {
      storage.setItem(VOLUME_STORAGE_KEY, damaged);
      expect(setup({}, volumePrefs(() => storage)).state(), damaged).toMatchObject({ volume: 1, muted: false });
    }
    // A level outside 0 to 1 is held to it.
    storage.setItem(VOLUME_STORAGE_KEY, '{"level":4,"muted":true}');
    expect(setup({}, volumePrefs(() => storage)).state()).toMatchObject({ volume: 1, muted: true });
  });

  it("works when storage throws: reading and writing are skipped, the sound still changes", async () => {
    const broken = {
      getItem: () => {
        throw new Error("blocked");
      },
      setItem: () => {
        throw new Error("full");
      },
    };
    const t = setup({}, volumePrefs(() => broken));
    expect(t.state()).toMatchObject({ volume: 1, muted: false });
    await open(t, A, 100);
    t.state().setVolume(0.5);
    t.state().toggleMute();
    expect(t.state()).toMatchObject({ volume: 0.5, muted: true });
    expect(t.audio.volume).toBe(0.5);
    expect(t.audio.muted).toBe(true);
  });

  it("works when reaching storage at all throws", () => {
    const prefs = volumePrefs(() => {
      throw new Error("access denied");
    });
    const t = setup({}, prefs);
    t.state().setVolume(0.2);
    expect(t.state().volume).toBe(0.2);
  });
});
