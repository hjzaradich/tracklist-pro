import { QueryClientProvider } from "@tanstack/react-query";
import { clearMocks, mockConvertFileSrc, mockIPC } from "@tauri-apps/api/mocks";
import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createQueryClient } from "../app/queryClient";
import type { PreviewRefusal, Waveform as WaveformAnswer } from "../bindings";
import { PlayerBar } from "../shell/PlayerBar";
import { tx } from "../test/tx";
import { formatTime } from "./formatTime";
import { PlayButton } from "./PlayButton";
import { createPlayer, usePlayer } from "./playerStore";
import { browserVolumePrefs, VOLUME_STORAGE_KEY } from "./volumePrefs";

const A = { id: 11, name: "Synthetic Tune A" };
const B = { id: 12, name: "Synthetic Tune B" };

/** The audio element the player made, once it loads something. */
let element: HTMLAudioElement | null;
let asked: number[];
let answers: Record<number, PreviewRefusal | null>;
/** What the waveform command says per file; "not ready" when a test says nothing. */
let waveforms: Record<number, WaveformAnswer>;

beforeEach(() => {
  element = null;
  asked = [];
  answers = {};
  waveforms = {};
  // jsdom plays nothing; these record what the player asks of the element.
  vi.spyOn(HTMLMediaElement.prototype, "load").mockImplementation(function (this: HTMLMediaElement) {
    element = this as HTMLAudioElement;
  });
  vi.spyOn(HTMLMediaElement.prototype, "play").mockResolvedValue(undefined);
  vi.spyOn(HTMLMediaElement.prototype, "pause").mockImplementation(() => {});
  mockIPC((cmd, args) => {
    if (cmd === "preview_check") {
      const id = (args as { fileId: number }).fileId;
      asked.push(id);
      return answers[id] ?? null;
    }
    if (cmd === "waveform_first") return null;
    if (cmd === "waveform_peaks") {
      const id = (args as { fileId: number }).fileId;
      return waveforms[id] ?? { state: "notReady" };
    }
    throw new Error(`unexpected command ${cmd}`);
  });
  mockConvertFileSrc("windows");
  usePlayer.setState({ file: null, status: "idle", refusal: null, position: 0, duration: 0, playing: false });
  usePlayer.getState().setVolume(1, false);
});

afterEach(() => {
  cleanup();
  clearMocks();
});

/** What the browser does once the file's header is read. */
function loaded(duration: number) {
  Object.defineProperty(element, "duration", { value: duration, configurable: true });
  act(() => {
    element?.dispatchEvent(new Event("loadedmetadata"));
  });
}

/** The waveform that is the seek bar, laid out 400 px wide from the left edge of the page. */
function seekBar() {
  const bar = screen.getByRole("slider", { name: tx("player:position") });
  vi.spyOn(bar, "getBoundingClientRect").mockReturnValue({ left: 0, width: 400, right: 400, x: 0 } as DOMRect);
  return bar;
}

/** A click: the pointer goes down and comes up at `x`. */
function click(bar: HTMLElement, x: number) {
  fireEvent.pointerDown(bar, { clientX: x, button: 0, pointerId: 1 });
  fireEvent.pointerUp(bar, { clientX: x, button: 0, pointerId: 1 });
}

function renderPlayer() {
  return render(
    <QueryClientProvider client={createQueryClient()}>
      <PlayButton file={A} />
      <PlayButton file={B} />
      <PlayerBar />
    </QueryClientProvider>,
  );
}

describe("the player bar", () => {
  it("says nothing is playing until a file is chosen", () => {
    render(<PlayerBar />);
    expect(screen.getByText(tx("shell:player.empty"))).toBeInTheDocument();
    expect(screen.queryByRole("slider")).toBeNull();
  });

  it("plays a file from its play button and shows its name, play/pause and a seek bar", async () => {
    const user = userEvent.setup();
    renderPlayer();
    await user.click(screen.getByRole("button", { name: tx("player:playNamed", { name: A.name }) }));
    await waitFor(() => expect(element).not.toBeNull());
    expect(asked).toEqual([A.id]);
    // By id, on the app's own protocol.
    expect(element?.getAttribute("src")).toContain(`audio-preview.localhost/${A.id}`);
    loaded(300);

    const bar = screen.getByRole("region", { name: tx("shell:player.label") });
    expect(bar).toHaveTextContent(A.name);
    expect(bar).toHaveTextContent("0:00 / 5:00");
    expect(screen.getByRole("slider", { name: tx("player:position") })).toHaveAttribute("aria-disabled", "false");
    expect(HTMLMediaElement.prototype.play).toHaveBeenCalledTimes(1);
  });

  it("pauses and plays again from the bar", async () => {
    const user = userEvent.setup();
    renderPlayer();
    await user.click(screen.getByRole("button", { name: tx("player:playNamed", { name: A.name }) }));
    await waitFor(() => expect(element).not.toBeNull());
    loaded(300);

    const bar = screen.getByRole("region", { name: tx("shell:player.label") });
    const toggle = () =>
      within(bar).getByRole("button", { name: (name) => name === tx("player:play") || name === tx("player:pause") });
    // An icon: the name is the label, not text.
    expect(toggle()).toHaveAccessibleName(tx("player:pause"));
    await user.click(toggle());
    expect(toggle()).toHaveAccessibleName(tx("player:play"));
    expect(HTMLMediaElement.prototype.pause).toHaveBeenCalled();
    await user.click(toggle());
    expect(toggle()).toHaveAccessibleName(tx("player:pause"));
    expect(HTMLMediaElement.prototype.play).toHaveBeenCalledTimes(2);
  });

  it("seeks to the time clicked on the waveform", async () => {
    const user = userEvent.setup();
    renderPlayer();
    await user.click(screen.getByRole("button", { name: tx("player:playNamed", { name: A.name }) }));
    await waitFor(() => expect(element).not.toBeNull());
    loaded(300);

    // 120 px across a 400 px bar of a 300 s file is 90 s.
    click(seekBar(), 120);
    expect(element?.currentTime).toBe(90);
    expect(screen.getByRole("region", { name: tx("shell:player.label") })).toHaveTextContent("1:30 / 5:00");
  });

  it("is one player: choosing a second file shows the first as stopped", async () => {
    const user = userEvent.setup();
    renderPlayer();
    await user.click(screen.getByRole("button", { name: tx("player:playNamed", { name: A.name }) }));
    await waitFor(() => expect(element).not.toBeNull());
    loaded(300);
    expect(screen.getByRole("button", { name: tx("player:pauseNamed", { name: A.name }) })).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: tx("player:playNamed", { name: B.name }) }));
    await waitFor(() => expect(asked).toEqual([A.id, B.id]));
    loaded(300);
    // Only the second file's button says it is playing; the bar names it.
    expect(screen.getByRole("button", { name: tx("player:pauseNamed", { name: B.name }) })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: tx("player:playNamed", { name: A.name }) })).toBeInTheDocument();
    expect(screen.getByRole("region", { name: tx("shell:player.label") })).toHaveTextContent(B.name);
  });

  it("pressing the button of the file that is playing pauses it", async () => {
    const user = userEvent.setup();
    renderPlayer();
    await user.click(screen.getByRole("button", { name: tx("player:playNamed", { name: A.name }) }));
    await waitFor(() => expect(element).not.toBeNull());
    loaded(300);
    await user.click(screen.getByRole("button", { name: tx("player:pauseNamed", { name: A.name }) }));
    expect(screen.getByRole("button", { name: tx("player:playNamed", { name: A.name }) })).toBeInTheDocument();
    expect(asked).toEqual([A.id]);
  });
});

describe("a file that can't be played", () => {
  const lines: [PreviewRefusal, string][] = [
    ["missing", "player:refused.missing"],
    ["drive_not_connected", "player:refused.driveNotConnected"],
    ["online_only", "player:refused.onlineOnly"],
    ["unknown_file", "player:refused.cantPlay"],
    ["unreadable", "player:refused.cantPlay"],
    ["not_decodable", "player:refused.cantPlay"],
  ];

  it.each(lines)("%s gets its line, with no scrub bar and nothing playing", async (code, key) => {
    answers[A.id] = code;
    const user = userEvent.setup();
    renderPlayer();
    await user.click(screen.getByRole("button", { name: tx("player:playNamed", { name: A.name }) }));
    expect(await screen.findByRole("status")).toHaveTextContent(tx(key));
    expect(screen.queryByRole("slider", { name: tx("player:position") })).toBeNull();
    expect(element).toBeNull();
    expect(HTMLMediaElement.prototype.play).not.toHaveBeenCalled();
    // The file stays named, so the line says which one.
    expect(screen.getByRole("region", { name: tx("shell:player.label") })).toHaveTextContent(A.name);
  });

  it("shows a refused file's button as play, and pressing it asks again", async () => {
    answers[A.id] = "drive_not_connected";
    const user = userEvent.setup();
    renderPlayer();
    await user.click(screen.getByRole("button", { name: tx("player:playNamed", { name: A.name }) }));
    expect(await screen.findByRole("status")).toHaveTextContent(tx("player:refused.driveNotConnected"));
    // Not "Pause" beside a line saying it can't play.
    expect(screen.getByRole("button", { name: tx("player:playNamed", { name: A.name }) })).toBeInTheDocument();

    answers[A.id] = null;
    await user.click(screen.getByRole("button", { name: tx("player:playNamed", { name: A.name }) }));
    await waitFor(() => expect(element).not.toBeNull());
    expect(asked).toEqual([A.id, A.id]);
    loaded(300);
    expect(screen.queryByRole("status")).toBeNull();
    expect(screen.getByRole("slider", { name: tx("player:position") })).not.toHaveAttribute("aria-disabled", "true");
  });

  it("uses four different lines for the six codes", () => {
    expect(new Set(lines.map(([, key]) => tx(key))).size).toBe(4);
  });
});

describe("the waveform in the player bar", () => {
  const ready: WaveformAnswer = {
    state: "ready",
    peaks: [10, 200, 120, 255],
    levels: [5, 100, 60, 128],
    durationMs: 300_000,
  };

  async function play(name: string) {
    const user = userEvent.setup();
    renderPlayer();
    await user.click(screen.getByRole("button", { name: tx("player:playNamed", { name }) }));
    await waitFor(() => expect(element).not.toBeNull());
    loaded(300);
    return user;
  }

  it("stands in with a placeholder until the waveform is drawn", async () => {
    await play(A.name);
    expect(await screen.findByRole("img", { name: tx("player:waveform.notReady") })).toBeInTheDocument();
    expect(screen.queryByRole("img", { name: tx("player:waveform.label") })).toBeNull();
  });

  it("draws the waveform of the loaded file with the playhead where playback is", async () => {
    waveforms[A.id] = ready;
    await play(A.name);
    const wave = await screen.findByRole("img", { name: tx("player:waveform.label") });
    expect(wave.querySelectorAll("path")).toHaveLength(2);
    const playhead = wave.querySelector<HTMLElement>("[data-playhead]");
    expect(playhead?.style.left).toBe("0%");
    act(() => {
      if (element) element.currentTime = 75;
      element?.dispatchEvent(new Event("timeupdate"));
    });
    await waitFor(() => expect(playhead?.style.left).toBe("25%"));
  });

  it("says so for a file whose audio can't be decoded", async () => {
    waveforms[A.id] = { state: "unavailable" };
    await play(A.name);
    expect(await screen.findByRole("img", { name: tx("player:waveform.unavailable") })).toBeInTheDocument();
  });

  it("has no waveform where the file can't be played at all", async () => {
    waveforms[B.id] = ready;
    answers[B.id] = "missing";
    const user = userEvent.setup();
    renderPlayer();
    await user.click(screen.getByRole("button", { name: tx("player:playNamed", { name: B.name }) }));
    await waitFor(() => expect(screen.getByRole("status")).toBeInTheDocument());
    expect(screen.queryByRole("img", { name: tx("player:waveform.label") })).toBeNull();
    expect(screen.queryByRole("img", { name: tx("player:waveform.notReady") })).toBeNull();
  });
});

describe("the playing bars", () => {
  /** The bars beside a file's play button, and in the player bar. */
  function barsOf(name: string) {
    const button = screen.getByRole("button", { name });
    return button.parentElement?.querySelector("[data-playing]") as HTMLElement;
  }
  function barsInBar() {
    const bar = screen.getByRole("region", { name: tx("shell:player.label") });
    return bar.querySelector("[data-playing]") as HTMLElement;
  }

  it("show beside the playing file and in the bar, and nowhere else", async () => {
    const user = userEvent.setup();
    renderPlayer();
    // Nothing chosen: every file's bars are there to take room, and hidden.
    expect(barsOf(tx("player:playNamed", { name: A.name }))).toHaveAttribute("data-playing", "false");
    expect(barsOf(tx("player:playNamed", { name: B.name }))).toHaveAttribute("data-playing", "false");

    await user.click(screen.getByRole("button", { name: tx("player:playNamed", { name: A.name }) }));
    await waitFor(() => expect(element).not.toBeNull());
    loaded(300);
    expect(barsOf(tx("player:pauseNamed", { name: A.name }))).toHaveAttribute("data-playing", "true");
    expect(barsOf(tx("player:playNamed", { name: B.name }))).toHaveAttribute("data-playing", "false");
    expect(barsInBar()).toHaveAttribute("data-playing", "true");
  });

  it("go away when the track is paused, and stay away from a file that can't be played", async () => {
    const user = userEvent.setup();
    renderPlayer();
    await user.click(screen.getByRole("button", { name: tx("player:playNamed", { name: A.name }) }));
    await waitFor(() => expect(element).not.toBeNull());
    loaded(300);
    await user.click(screen.getByRole("button", { name: tx("player:pause") }));
    expect(barsOf(tx("player:playNamed", { name: A.name }))).toHaveAttribute("data-playing", "false");
    expect(barsInBar()).toHaveAttribute("data-playing", "false");

    answers[B.id] = "missing";
    await user.click(screen.getByRole("button", { name: tx("player:playNamed", { name: B.name }) }));
    await waitFor(() => expect(screen.getByRole("status")).toBeInTheDocument());
    expect(barsOf(tx("player:playNamed", { name: B.name }))).toHaveAttribute("data-playing", "false");
    expect(barsInBar()).toHaveAttribute("data-playing", "false");
  });

  it("are decoration: no text, and hidden from a screen reader", () => {
    renderPlayer();
    for (const name of [A.name, B.name]) {
      const bars = barsOf(tx("player:playNamed", { name }));
      expect(bars).toHaveAttribute("aria-hidden", "true");
      expect(bars.textContent).toBe("");
    }
  });
});

describe("time", () => {
  it("is written m:ss, and h:mm:ss from an hour", () => {
    expect(formatTime(0)).toBe("0:00");
    expect(formatTime(90)).toBe("1:30");
    expect(formatTime(599.9)).toBe("9:59");
    expect(formatTime(3600)).toBe("1:00:00");
    expect(formatTime(3725)).toBe("1:02:05");
    expect(formatTime(Number.NaN)).toBe("0:00");
    expect(formatTime(-4)).toBe("0:00");
  });
});

describe("seeking from the waveform", () => {
  const ready: WaveformAnswer = {
    state: "ready",
    peaks: [10, 200, 120, 255],
    levels: [5, 100, 60, 128],
    durationMs: 300_000,
  };

  async function playA() {
    const user = userEvent.setup();
    renderPlayer();
    await user.click(screen.getByRole("button", { name: tx("player:playNamed", { name: A.name }) }));
    await waitFor(() => expect(element).not.toBeNull());
    loaded(300);
    return user;
  }
  const playhead = () =>
    screen.getByRole("slider", { name: tx("player:position") }).querySelector<HTMLElement>("[data-playhead]");

  it("seeks from the drawn waveform: a click a quarter of the way along moves to a quarter of the file", async () => {
    waveforms[A.id] = ready;
    await playA();
    await screen.findByRole("img", { name: tx("player:waveform.label") });
    click(seekBar(), 100);
    expect(element?.currentTime).toBe(75);
    await waitFor(() => expect(playhead()?.style.left).toBe("25%"));
  });

  it("seeks from the placeholder too, while the waveform is not drawn yet", async () => {
    await playA();
    expect(await screen.findByRole("img", { name: tx("player:waveform.notReady") })).toBeInTheDocument();
    click(seekBar(), 200);
    expect(element?.currentTime).toBe(150);
    expect(screen.getByRole("region", { name: tx("shell:player.label") })).toHaveTextContent("2:30 / 5:00");
  });

  it("seeks from the placeholder of a file whose waveform will never come", async () => {
    waveforms[A.id] = { state: "unavailable" };
    await playA();
    expect(await screen.findByRole("img", { name: tx("player:waveform.unavailable") })).toBeInTheDocument();
    click(seekBar(), 40);
    expect(element?.currentTime).toBe(30);
  });

  it("drags: the playhead follows the pointer, and the audio moves where it is let go", async () => {
    waveforms[A.id] = ready;
    await playA();
    await screen.findByRole("img", { name: tx("player:waveform.label") });
    const bar = seekBar();
    fireEvent.pointerDown(bar, { clientX: 40, button: 0, pointerId: 1 });
    fireEvent.pointerMove(bar, { clientX: 200, pointerId: 1 });
    expect(playhead()?.style.left).toBe("50%");
    expect(screen.getByRole("region", { name: tx("shell:player.label") })).toHaveTextContent("0:00 / 5:00");
    // Not moved yet: the sound is not chased across the file with every pixel.
    expect(element?.currentTime).toBe(0);
    fireEvent.pointerMove(bar, { clientX: 300, pointerId: 1 });
    fireEvent.pointerUp(bar, { clientX: 300, button: 0, pointerId: 1 });
    expect(element?.currentTime).toBe(225);
    expect(playhead()?.style.left).toBe("75%");
  });

  it("holds a drag that leaves the bar to its ends", async () => {
    await playA();
    const bar = seekBar();
    fireEvent.pointerDown(bar, { clientX: 200, button: 0, pointerId: 1 });
    fireEvent.pointerUp(bar, { clientX: 2000, button: 0, pointerId: 1 });
    expect(element?.currentTime).toBe(300);
    fireEvent.pointerDown(bar, { clientX: 200, button: 0, pointerId: 1 });
    fireEvent.pointerUp(bar, { clientX: -80, button: 0, pointerId: 1 });
    expect(element?.currentTime).toBe(0);
  });

  it("seeks nothing in the new file when the file changes in the middle of a drag", async () => {
    const user = userEvent.setup();
    renderPlayer();
    await user.click(screen.getByRole("button", { name: tx("player:playNamed", { name: A.name }) }));
    await waitFor(() => expect(element).not.toBeNull());
    loaded(300);
    const bar = seekBar();
    fireEvent.pointerDown(bar, { clientX: 300, button: 0, pointerId: 1 });
    // Another file is chosen while the pointer is held.
    await user.click(screen.getByRole("button", { name: tx("player:playNamed", { name: B.name }) }));
    await waitFor(() => expect(asked).toEqual([A.id, B.id]));
    loaded(240);
    expect(usePlayer.getState().position).toBe(0);
    fireEvent.pointerUp(bar, { clientX: 300, button: 0, pointerId: 1 });
    expect(usePlayer.getState().position).toBe(0);
    expect(element?.currentTime).toBe(0);
  });

  it("seeks nowhere when the drag is cancelled", async () => {
    await playA();
    const bar = seekBar();
    fireEvent.pointerDown(bar, { clientX: 300, button: 0, pointerId: 1 });
    fireEvent.pointerCancel(bar, { pointerId: 1 });
    fireEvent.pointerUp(bar, { clientX: 300, button: 0, pointerId: 1 });
    expect(element?.currentTime).toBe(0);
  });

  it("ignores any button but the main one", async () => {
    await playA();
    const bar = seekBar();
    fireEvent.pointerDown(bar, { clientX: 300, button: 2, pointerId: 1 });
    fireEvent.pointerUp(bar, { clientX: 300, button: 2, pointerId: 1 });
    expect(element?.currentTime).toBe(0);
  });

  it("cannot seek until the file is loaded, and says so", async () => {
    const user = userEvent.setup();
    renderPlayer();
    await user.click(screen.getByRole("button", { name: tx("player:playNamed", { name: A.name }) }));
    await waitFor(() => expect(element).not.toBeNull());
    // The element has the file, but its header has not been read.
    const bar = seekBar();
    expect(bar).toHaveAttribute("aria-disabled", "true");
    click(bar, 200);
    expect(usePlayer.getState().position).toBe(0);
    loaded(300);
    expect(bar).toHaveAttribute("aria-disabled", "false");
  });

  it("seeks from the keyboard: arrows a few seconds, Page Up and Down more, Home and End to the ends", async () => {
    const user = await playA();
    const bar = seekBar();
    bar.focus();
    await user.keyboard("{ArrowRight}");
    expect(element?.currentTime).toBe(5);
    await user.keyboard("{PageUp}");
    expect(element?.currentTime).toBe(35);
    await user.keyboard("{ArrowLeft}");
    expect(element?.currentTime).toBe(30);
    await user.keyboard("{End}");
    expect(element?.currentTime).toBe(300);
    await user.keyboard("{Home}");
    expect(element?.currentTime).toBe(0);
  });

  it("reads out the time to a screen reader, with the length as the end of the scale", async () => {
    await playA();
    click(seekBar(), 120);
    const bar = screen.getByRole("slider", { name: tx("player:position") });
    expect(bar).toHaveAttribute("aria-valuetext", "1:30");
    expect(bar).toHaveAttribute("aria-valuenow", "90");
    expect(bar).toHaveAttribute("aria-valuemax", "300");
  });
});

describe("the position still carries to another file", () => {
  it("keeps 1:30 when another file's play button is pressed, playing as it was", async () => {
    const user = userEvent.setup();
    renderPlayer();
    await user.click(screen.getByRole("button", { name: tx("player:playNamed", { name: A.name }) }));
    await waitFor(() => expect(element).not.toBeNull());
    loaded(300);
    click(seekBar(), 120);

    await user.click(screen.getByRole("button", { name: tx("player:playNamed", { name: B.name }) }));
    await waitFor(() => expect(asked).toEqual([A.id, B.id]));
    loaded(240);
    const bar = screen.getByRole("region", { name: tx("shell:player.label") });
    expect(bar).toHaveTextContent(B.name);
    expect(bar).toHaveTextContent("1:30 / 4:00");
    expect(HTMLMediaElement.prototype.play).toHaveBeenCalledTimes(2);
  });
});

describe("volume and mute on the bar", () => {
  async function playA() {
    const user = userEvent.setup();
    renderPlayer();
    await user.click(screen.getByRole("button", { name: tx("player:playNamed", { name: A.name }) }));
    await waitFor(() => expect(element).not.toBeNull());
    loaded(300);
    return user;
  }
  const slider = () => screen.getByRole("slider", { name: tx("player:volume") });
  /** What the app does on its next start: a new player over the same storage. */
  const started = () =>
    createPlayer({
      audio: () => document.createElement("audio"),
      check: () => Promise.resolve(null),
      urlOf: (id) => `media://${id}`,
      volume: browserVolumePrefs,
    });

  it("sets the sound of the one audio element from the slider", async () => {
    await playA();
    fireEvent.change(slider(), { target: { value: "0.4" } });
    expect(element?.volume).toBe(0.4);
    expect(slider()).toHaveValue("0.4");
  });

  it("mutes and brings the sound back at the same volume, the button naming what it will do", async () => {
    const user = await playA();
    fireEvent.change(slider(), { target: { value: "0.4" } });
    await user.click(screen.getByRole("button", { name: tx("player:mute") }));
    expect(element?.muted).toBe(true);
    // The slider shows the sound is off; the volume it will come back at is kept.
    expect(slider()).toHaveValue("0");
    expect(usePlayer.getState().volume).toBe(0.4);

    await user.click(screen.getByRole("button", { name: tx("player:unmute") }));
    expect(element?.muted).toBe(false);
    expect(element?.volume).toBe(0.4);
    expect(slider()).toHaveValue("0.4");
    expect(screen.getByRole("button", { name: tx("player:mute") })).toBeInTheDocument();
  });

  it("survives a reload: the volume and mute are kept in storage and a new player starts from them", async () => {
    const user = await playA();
    fireEvent.change(slider(), { target: { value: "0.3" } });
    fireEvent.pointerUp(slider());
    await user.click(screen.getByRole("button", { name: tx("player:mute") }));
    expect(JSON.parse(localStorage.getItem(VOLUME_STORAGE_KEY) ?? "null")).toEqual({ level: 0.3, muted: true });
    expect(started().getState()).toMatchObject({ volume: 0.3, muted: true });
  });

  it("keeps the volume when the slider is let go, not at every step of a drag", async () => {
    const setItem = vi.spyOn(Storage.prototype, "setItem");
    await playA();
    for (const value of ["0.2", "0.4", "0.6"]) fireEvent.change(slider(), { target: { value } });
    // The sound followed the slider, nothing was written yet.
    expect(element?.volume).toBe(0.6);
    expect(setItem).not.toHaveBeenCalled();
    fireEvent.pointerUp(slider());
    expect(setItem).toHaveBeenCalledTimes(1);
    expect(JSON.parse(localStorage.getItem(VOLUME_STORAGE_KEY) ?? "null")).toEqual({ level: 0.6, muted: false });
    // Letting go without having moved it writes nothing more.
    fireEvent.pointerUp(slider());
    fireEvent.blur(slider());
    expect(setItem).toHaveBeenCalledTimes(1);
  });

  it("keeps the volume set with the arrow keys when the key is let go", async () => {
    await playA();
    // What the browser does for an arrow key on a slider: a change, then the key comes up.
    fireEvent.change(slider(), { target: { value: "0.99" } });
    expect(localStorage.getItem(VOLUME_STORAGE_KEY)).toBeNull();
    fireEvent.keyUp(slider(), { key: "ArrowLeft" });
    expect(JSON.parse(localStorage.getItem(VOLUME_STORAGE_KEY) ?? "null")).toEqual({ level: 0.99, muted: false });
  });

  it("works when storage throws: the slider and the mute button still change the sound", async () => {
    vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => {
      throw new DOMException("full", "QuotaExceededError");
    });
    vi.spyOn(Storage.prototype, "getItem").mockImplementation(() => {
      throw new DOMException("blocked", "SecurityError");
    });
    const user = await playA();
    fireEvent.change(slider(), { target: { value: "0.6" } });
    fireEvent.pointerUp(slider());
    expect(element?.volume).toBe(0.6);
    await user.click(screen.getByRole("button", { name: tx("player:mute") }));
    expect(element?.muted).toBe(true);
    // And a player started now does not fall over reading it.
    expect(started().getState()).toMatchObject({ volume: 1, muted: false });
  });

  it("is on the bar of a file that cannot be played, so the sound can still be set", async () => {
    answers[A.id] = "missing";
    const user = userEvent.setup();
    renderPlayer();
    await user.click(screen.getByRole("button", { name: tx("player:playNamed", { name: A.name }) }));
    expect(await screen.findByRole("status")).toBeInTheDocument();
    expect(slider()).toBeInTheDocument();
    expect(screen.getByRole("button", { name: tx("player:mute") })).toBeInTheDocument();
  });
});
