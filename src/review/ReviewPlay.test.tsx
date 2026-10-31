import { QueryClientProvider } from "@tanstack/react-query";
import { clearMocks, mockConvertFileSrc, mockIPC } from "@tauri-apps/api/mocks";
import { act, cleanup, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createQueryClient } from "../app/queryClient";
import type { PreviewRefusal, ReviewComparison as Comparison, ReviewFileRow } from "../bindings";
import "../i18n";
import { usePlayer } from "../player/playerStore";
import { PlayerBar } from "../shell/PlayerBar";
import { tx } from "../test/tx";
import { ReviewComparison } from "./ReviewComparison";
import { quietAnswer } from "../test/quietCommands";

function row(fileId: number | null, name: string, fields: Partial<ReviewFileRow> = {}): ReviewFileRow {
  return {
    fileId,
    source: fileId === null ? "rekordboxEntry" : "file",
    recordingId: 1,
    name,
    path: "E:\\Music\\" + name,
    onDisk: true,
    driveConnected: true,
    linked: false,
    format: "mp3",
    verdict: null,
    cutoffHz: null,
    durationMs: 215000,
    tagsFilled: 3,
    tagsTotal: 6,
    hasArt: false,
    rekordbox: "unknown",
    bitrateKbps: 320,
    sampleRateHz: 44100,
    addedAt: "2026-09-30T10:00:00.000Z",
    ...fields,
  };
}

const COMPARISON: Comparison = {
  id: { kind: "versionDoubt", a: 10, b: 11 },
  kind: "versionDoubt",
  group: "needsCloserLook",
  tracks: [{ recordingId: 10, title: "Synthetic Tune", artist: "Made Up Artist" }],
  doubt: [],
  namesSayVersions: false,
  suggestion: null,
  rows: [
    row(21, "tune.flac"),
    row(22, "tune.mp3", { linked: true }),
    row(23, "tune gone.mp3", { onDisk: false }),
    row(null, "tune entry.mp3"),
  ],
  moreFiles: 0,
  switch: null,
  copies: [],
  relink: null,
  sameRecording: null,
};

let element: HTMLAudioElement | null;
let asked: number[];
let answers: Record<number, PreviewRefusal | null>;

beforeEach(() => {
  element = null;
  asked = [];
  answers = {};
  vi.spyOn(HTMLMediaElement.prototype, "load").mockImplementation(function (this: HTMLMediaElement) {
    element = this as HTMLAudioElement;
  });
  vi.spyOn(HTMLMediaElement.prototype, "play").mockResolvedValue(undefined);
  vi.spyOn(HTMLMediaElement.prototype, "pause").mockImplementation(() => {});
  mockConvertFileSrc("windows");
  mockIPC((cmd, args) => {
    if (cmd === "preview_check") {
      const id = (args as { fileId: number }).fileId;
      asked.push(id);
      return answers[id] ?? null;
    }
    return quietAnswer(cmd);
  });
  usePlayer.setState({ file: null, status: "idle", refusal: null, position: 0, duration: 0, playing: false });
});

afterEach(() => {
  cleanup();
  clearMocks();
});

function renderReview() {
  return render(
    <QueryClientProvider client={createQueryClient()}>
      <ReviewComparison comparison={COMPARISON} details={false} onDetails={() => {}} />
      <PlayerBar />
    </QueryClientProvider>,
  );
}

const playOf = (name: string) =>
  screen.getByRole("button", { name: tx("player:playNamed", { name }) });

/** The browser has read the header of what was loaded last. */
function loaded(duration: number) {
  Object.defineProperty(element, "duration", { value: duration, configurable: true });
  act(() => {
    element?.dispatchEvent(new Event("loadedmetadata"));
  });
}

/** Time passes in the loaded file. */
function listenTo(seconds: number) {
  act(() => {
    if (element) element.currentTime = seconds;
    element?.dispatchEvent(new Event("timeupdate"));
  });
}

/** Waits for `count` checks, and for `loads` of them to have reached the element. */
async function nextLoad(count: number, loads = count) {
  await waitFor(() => expect(asked).toHaveLength(count));
  // The element is told to load after the check answers.
  await waitFor(() => expect(HTMLMediaElement.prototype.load).toHaveBeenCalledTimes(loads));
}

describe("playing the files of a Review item", () => {
  it("puts a play button on each file, and none on a rekordbox entry with no file", () => {
    renderReview();
    const table = screen.getByRole("table", { name: tx("review:compare.label") });
    for (const name of ["tune.flac", "tune.mp3", "tune gone.mp3"]) {
      expect(within(table).getByRole("button", { name: tx("player:playNamed", { name }) })).toBeInTheDocument();
    }
    expect(within(table).getAllByRole("button", { name: /./ }).filter((b) => b.hasAttribute("data-play-for"))).toHaveLength(3);
    expect(within(table).queryByRole("button", { name: tx("player:playNamed", { name: "tune entry.mp3" }) })).toBeNull();
  });

  it("plays the file of the row, by its id", async () => {
    const user = userEvent.setup();
    renderReview();
    await user.click(playOf("tune.mp3"));
    await nextLoad(1);
    expect(asked).toEqual([22]);
    expect(element?.getAttribute("src")).toContain("audio-preview.localhost/22");
    loaded(215);
    expect(screen.getByRole("button", { name: tx("player:pauseNamed", { name: "tune.mp3" }) })).toBeInTheDocument();
  });

  it("keeps the position when listening switches to another row: 1:30 stays 1:30", async () => {
    const user = userEvent.setup();
    renderReview();
    await user.click(playOf("tune.mp3"));
    await nextLoad(1);
    loaded(215);
    listenTo(90);

    await user.click(playOf("tune.flac"));
    await nextLoad(2);
    loaded(230);
    expect(element?.currentTime).toBe(90);
    expect(screen.getByRole("region", { name: tx("shell:player.label") })).toHaveTextContent("1:30 / 3:50");
  });

  it("starts from the beginning when the other file is shorter than the position", async () => {
    const user = userEvent.setup();
    renderReview();
    await user.click(playOf("tune.mp3"));
    await nextLoad(1);
    loaded(215);
    listenTo(150);

    await user.click(playOf("tune.flac"));
    await nextLoad(2);
    loaded(120);
    expect(element?.currentTime).toBe(0);
    expect(screen.getByRole("region", { name: tx("shell:player.label") })).toHaveTextContent("0:00 / 2:00");
  });

  it("keeps playing what was playing, and keeps paused what was paused", async () => {
    const user = userEvent.setup();
    renderReview();
    await user.click(playOf("tune.mp3"));
    await nextLoad(1);
    loaded(215);
    listenTo(40);
    const play = vi.mocked(HTMLMediaElement.prototype.play);
    expect(play).toHaveBeenCalledTimes(1);

    // Playing: the next file plays too.
    await user.click(playOf("tune.flac"));
    await nextLoad(2);
    loaded(215);
    expect(play).toHaveBeenCalledTimes(2);
    expect(screen.getByRole("button", { name: tx("player:pauseNamed", { name: "tune.flac" }) })).toBeInTheDocument();

    // Paused: the next file is loaded at the same time and stays paused.
    await user.click(screen.getByRole("button", { name: tx("player:pauseNamed", { name: "tune.flac" }) }));
    listenTo(75);
    await user.click(playOf("tune.mp3"));
    await nextLoad(3);
    loaded(215);
    expect(play).toHaveBeenCalledTimes(2);
    expect(element?.currentTime).toBe(75);
    expect(playOf("tune.mp3")).toBeInTheDocument();
  });

  it("never sets the audio source for a row whose file the player refuses", async () => {
    answers[23] = "drive_not_connected";
    const user = userEvent.setup();
    renderReview();
    await user.click(playOf("tune gone.mp3"));
    expect(await screen.findByRole("status")).toHaveTextContent(tx("player:refused.driveNotConnected"));
    // Asked about, then refused: the element was never given anything to load.
    expect(asked).toEqual([23]);
    expect(HTMLMediaElement.prototype.load).not.toHaveBeenCalled();
    expect(element).toBeNull();
    expect(HTMLMediaElement.prototype.play).not.toHaveBeenCalled();
  });

  it("says why a file can't be played, and the next row carries on from the same time", async () => {
    answers[23] = "missing";
    const user = userEvent.setup();
    renderReview();
    await user.click(playOf("tune.mp3"));
    await nextLoad(1);
    loaded(215);
    listenTo(90);

    await user.click(playOf("tune gone.mp3"));
    expect(await screen.findByRole("status")).toHaveTextContent(tx("player:refused.missing"));

    await user.click(playOf("tune.flac"));
    await nextLoad(3, 2);
    loaded(230);
    expect(element?.currentTime).toBe(90);
    expect(usePlayer.getState().playing).toBe(true);
  });
});
