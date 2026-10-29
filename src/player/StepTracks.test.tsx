import { QueryClientProvider } from "@tanstack/react-query";
import { clearMocks, mockConvertFileSrc, mockIPC } from "@tauri-apps/api/mocks";
import { act, cleanup, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createQueryClient } from "../app/queryClient";
import type { TableRow } from "../bindings";
import { PlayerBar } from "../shell/PlayerBar";
import { tableRow } from "../test/trackTableIpc";
import { tx } from "../test/tx";
import { TrackTable } from "../trackTable/TrackTable";
import { usePlayer } from "./playerStore";
import { useStepPlace } from "./stepList";

// Previous and next track (1cB-10): the player bar steps through the rows of
// the track table on screen. These tests render a real table beside the bar.

const ROW = 28;

/** The audio element the player made, once it loads something. */
let element: HTMLAudioElement | null;
/** The ids of the files the player was asked to check, in order: which files it opened. */
let opened: number[];

beforeEach(() => {
  element = null;
  opened = [];
  vi.spyOn(HTMLMediaElement.prototype, "load").mockImplementation(function (this: HTMLMediaElement) {
    element = this as HTMLAudioElement;
  });
  vi.spyOn(HTMLMediaElement.prototype, "play").mockResolvedValue(undefined);
  vi.spyOn(HTMLMediaElement.prototype, "pause").mockImplementation(() => {});
  vi.spyOn(HTMLElement.prototype, "clientHeight", "get").mockReturnValue(ROW * 11);
  mockIPC((cmd, args) => {
    if (cmd === "key_notation") return "camelot";
    if (cmd === "preview_check") {
      opened.push((args as { fileId: number }).fileId);
      return null;
    }
    if (cmd === "waveform_first") return null;
    if (cmd === "waveform_peaks") return { state: "notReady" };
    throw new Error(`unexpected command ${cmd}`);
  });
  mockConvertFileSrc("windows");
  usePlayer.setState({ file: null, status: "idle", refusal: null, position: 0, duration: 0, playing: false });
});

afterEach(() => {
  cleanup();
  clearMocks();
});

/** Rows 1 to `count`; each plays the file with its own number, and is titled "Tune N". */
const tunes = (count: number) => (index: number) =>
  index < count ? tableRow(index + 1, { title: `Tune ${index + 1}` }) : undefined;

function Screen({
  rows = tunes(5),
  total = 5,
  resetKey = "a",
  onSelectionChange,
}: {
  rows?: (index: number) => TableRow | undefined;
  total?: number;
  resetKey?: string;
  onSelectionChange?: (row: TableRow | null) => void;
}) {
  return (
    <QueryClientProvider client={createQueryClient()}>
      <TrackTable
        label="Under test"
        columns={["title"]}
        total={total}
        getRow={rows}
        onRangeChange={() => {}}
        sort={null}
        onSort={() => {}}
        resetKey={resetKey}
        onSelectionChange={onSelectionChange}
      />
      <PlayerBar />
    </QueryClientProvider>
  );
}

const bar = () => screen.getByRole("region", { name: tx("shell:player.label") });
const next = () => within(bar()).getByRole("button", { name: tx("player:next") });
const previous = () => within(bar()).getByRole("button", { name: tx("player:previous") });
const play = (n: number) => screen.getByRole("button", { name: tx("player:playNamed", { name: `Tune ${n}` }) });

/** The title of the row that is selected (what the Details panel would show), or null. */
function selectedTune() {
  const selected = screen.queryAllByRole("row").filter((row) => row.getAttribute("aria-selected") === "true");
  expect(selected.length).toBeLessThanOrEqual(1);
  return selected[0]?.textContent?.trim() ?? null;
}

/** What the browser does once the file's header is read, `duration` seconds long. */
function loaded(duration: number) {
  Object.defineProperty(element, "duration", { value: duration, configurable: true });
  act(() => {
    element?.dispatchEvent(new Event("loadedmetadata"));
  });
}

/** Plays row `n` from its own button, as a click would, and waits for the player to load it. */
async function playRow(user: ReturnType<typeof userEvent.setup>, n: number) {
  await user.click(play(n));
  await waitFor(() => expect(usePlayer.getState().file?.id).toBe(n));
  await waitFor(() => expect(element?.getAttribute("src")).toContain(`/${n}`));
  loaded(300);
  await waitFor(() => expect(usePlayer.getState().status).toBe("ready"));
}

/** Time passes in the playing file. */
function playTo(seconds: number) {
  act(() => {
    if (element) element.currentTime = seconds;
    element?.dispatchEvent(new Event("timeupdate"));
  });
}

describe("next track", () => {
  it("plays the row below and moves the table's selection to it, so the Details panel follows", async () => {
    const user = userEvent.setup();
    const shown = vi.fn();
    render(<Screen onSelectionChange={shown} />);
    await playRow(user, 2);
    expect(selectedTune()).toContain("Tune 2");

    await user.click(next());
    await waitFor(() => expect(usePlayer.getState().file).toMatchObject({ id: 3, name: "Tune 3" }));
    expect(selectedTune()).toContain("Tune 3");
    // What the Details panel is told is the row it now shows.
    expect(shown.mock.calls.at(-1)?.[0]).toMatchObject({ title: "Tune 3" });
    expect(opened).toEqual([2, 3]);
  });

  it("starts the next track from the beginning, not at the time the last one was at", async () => {
    const user = userEvent.setup();
    render(<Screen />);
    await playRow(user, 1);
    playTo(90);
    await user.click(next());
    await waitFor(() => expect(element?.getAttribute("src")).toContain("/2"));
    loaded(300);
    expect(usePlayer.getState()).toMatchObject({ position: 0, status: "ready" });
    expect(element?.currentTime).toBe(0);
  });

  it("plays it even when the last track was paused", async () => {
    const user = userEvent.setup();
    render(<Screen />);
    await playRow(user, 1);
    await user.click(within(bar()).getByRole("button", { name: tx("player:pause") }));
    expect(usePlayer.getState().playing).toBe(false);
    await user.click(next());
    await waitFor(() => expect(element?.getAttribute("src")).toContain("/2"));
    loaded(300);
    expect(usePlayer.getState().playing).toBe(true);
    expect(HTMLMediaElement.prototype.play).toHaveBeenCalledTimes(2);
  });

  it("stops at the last row: the button is off and nothing changes", async () => {
    const user = userEvent.setup();
    render(<Screen />);
    await playRow(user, 5);
    expect(next()).toBeDisabled();
    await user.click(next());
    expect(usePlayer.getState().file?.id).toBe(5);
    expect(selectedTune()).toContain("Tune 5");
    expect(opened).toEqual([5]);
  });

  it("is on again when a row above the last is selected", async () => {
    const user = userEvent.setup();
    render(<Screen />);
    await playRow(user, 5);
    expect(next()).toBeDisabled();
    await playRow(user, 4);
    expect(next()).toBeEnabled();
  });

  it("starts at the first row when no row is selected", async () => {
    const user = userEvent.setup();
    render(<Screen />);
    await playRow(user, 3);
    // The same screen opened afresh: no row is selected in it.
    cleanup();
    render(<Screen resetKey="b" />);
    expect(useStepPlace.getState().focus).toBeNull();
    await user.click(next());
    await waitFor(() => expect(usePlayer.getState().file?.id).toBe(1));
    expect(selectedTune()).toContain("Tune 1");
  });

  it("follows the table's order, whatever it is sorted or filtered to", async () => {
    const user = userEvent.setup();
    // The table's rows, in a sort that puts the files in the order 4, 9, 2.
    const order = [4, 9, 2];
    const filtered = (index: number) =>
      index < order.length ? tableRow(order[index], { title: `Tune ${order[index]}` }) : undefined;
    render(<Screen rows={filtered} total={3} />);
    await playRow(user, 4);
    await user.click(next());
    await waitFor(() => expect(usePlayer.getState().file?.id).toBe(9));
    await user.click(next());
    await waitFor(() => expect(usePlayer.getState().file?.id).toBe(2));
    expect(next()).toBeDisabled();
  });

  it("passes over a row that has no file", async () => {
    const user = userEvent.setup();
    const gaps = (index: number) =>
      index === 1 ? tableRow(2, { title: "Tune 2", file: null }) : tunes(4)(index);
    render(<Screen rows={gaps} total={4} />);
    await playRow(user, 1);
    await user.click(next());
    await waitFor(() => expect(usePlayer.getState().file?.id).toBe(3));
    expect(selectedTune()).toContain("Tune 3");
  });

  it("does nothing when the row below has not arrived from the backend yet", async () => {
    const user = userEvent.setup();
    const loading = (index: number) => (index === 1 ? undefined : tunes(5)(index));
    render(<Screen rows={loading} />);
    await playRow(user, 1);
    await user.click(next());
    expect(usePlayer.getState().file?.id).toBe(1);
    expect(selectedTune()).toContain("Tune 1");
  });

  it("is off when no table is on screen", async () => {
    const user = userEvent.setup();
    const view = render(<Screen />);
    await playRow(user, 1);
    expect(next()).toBeEnabled();
    // The table goes (another screen): the bar has no list to step through.
    view.rerender(
      <QueryClientProvider client={createQueryClient()}>
        <PlayerBar />
      </QueryClientProvider>,
    );
    expect(next()).toBeDisabled();
    expect(previous()).toBeDisabled();
  });
});

describe("previous track", () => {
  it("goes to the row above and plays it from the start when the track is less than 3 s in", async () => {
    const user = userEvent.setup();
    const shown = vi.fn();
    render(<Screen onSelectionChange={shown} />);
    await playRow(user, 3);
    playTo(2.5);
    await user.click(previous());
    await waitFor(() => expect(usePlayer.getState().file).toMatchObject({ id: 2, name: "Tune 2" }));
    expect(selectedTune()).toContain("Tune 2");
    expect(shown.mock.calls.at(-1)?.[0]).toMatchObject({ title: "Tune 2" });
  });

  it("goes to the row above at exactly 3 s: only more than 3 s restarts the track", async () => {
    const user = userEvent.setup();
    render(<Screen />);
    await playRow(user, 3);
    playTo(3);
    await user.click(previous());
    await waitFor(() => expect(usePlayer.getState().file?.id).toBe(2));
  });

  it("restarts the track, and stays on its row, when it is more than 3 s in", async () => {
    const user = userEvent.setup();
    render(<Screen />);
    await playRow(user, 3);
    playTo(3.5);
    await user.click(previous());
    expect(element?.currentTime).toBe(0);
    expect(usePlayer.getState()).toMatchObject({ position: 0, playing: true });
    expect(usePlayer.getState().file?.id).toBe(3);
    expect(selectedTune()).toContain("Tune 3");
    expect(opened).toEqual([3]);
  });

  it("restarts at the first row too when more than 3 s in, then stops: nothing is above it", async () => {
    const user = userEvent.setup();
    render(<Screen />);
    await playRow(user, 1);
    expect(previous()).toBeDisabled();
    playTo(10);
    expect(previous()).toBeEnabled();
    await user.click(previous());
    expect(element?.currentTime).toBe(0);
    expect(usePlayer.getState().file?.id).toBe(1);
    // Back at the start of the first row: no more to go back to.
    expect(previous()).toBeDisabled();
    await user.click(previous());
    expect(usePlayer.getState().file?.id).toBe(1);
    expect(opened).toEqual([1]);
  });

  it("restarts a track played from a screen without a table (Review), and goes nowhere else", async () => {
    const user = userEvent.setup();
    render(<Screen />);
    await playRow(user, 2);
    playTo(40);
    cleanup();
    render(
      <QueryClientProvider client={createQueryClient()}>
        <PlayerBar />
      </QueryClientProvider>,
    );
    expect(previous()).toBeEnabled();
    await user.click(previous());
    expect(usePlayer.getState().position).toBe(0);
    expect(usePlayer.getState().file?.id).toBe(2);
    expect(previous()).toBeDisabled();
  });
});
