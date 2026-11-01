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
import { listedActions, usePaletteRegistry } from "../palette/registry";
import { browserPlayNextPrefs, PLAY_NEXT_STORAGE_KEY } from "./playNextPrefs";
import { createPlayer, usePlayer } from "./playerStore";
import { useStepPlace } from "./stepList";

// Previous and next track (1cB-10): the player bar steps through the rows of
// the track table on screen. These tests render a real table beside the bar.

const ROW = 28;

/** The audio element the player made, once it loads something. */
let element: HTMLAudioElement | null;
/** The ids of the files the player was asked to check, in order: which files it opened. */
let opened: number[];
/** The files the player is told cannot be played, by id. */
let refused: Record<number, string>;

beforeEach(() => {
  element = null;
  opened = [];
  refused = {};
  vi.spyOn(HTMLMediaElement.prototype, "load").mockImplementation(function (this: HTMLMediaElement) {
    element = this as HTMLAudioElement;
  });
  vi.spyOn(HTMLMediaElement.prototype, "play").mockResolvedValue(undefined);
  vi.spyOn(HTMLMediaElement.prototype, "pause").mockImplementation(() => {});
  vi.spyOn(HTMLElement.prototype, "clientHeight", "get").mockReturnValue(ROW * 11);
  mockIPC((cmd, args) => {
    if (cmd === "key_notation") return "camelot";
    if (cmd === "preview_check") {
      const id = (args as { fileId: number }).fileId;
      opened.push(id);
      return refused[id] ?? null;
    }
    if (cmd === "waveform_first") return null;
    if (cmd === "waveform_peaks") return { state: "notReady" };
    throw new Error(`unexpected command ${cmd}`);
  });
  mockConvertFileSrc("windows");
  usePlayer.setState({ file: null, status: "idle", refusal: null, position: 0, duration: 0, playing: false });
  usePlayer.getState().setPlayNext(false);
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
  positionOf,
}: {
  rows?: (index: number) => TableRow | undefined;
  total?: number;
  resetKey?: string;
  onSelectionChange?: (row: TableRow | null) => void;
  positionOf?: (recordingId: number) => Promise<number | null>;
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
        positionOf={positionOf}
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

  it("starts at the first row when the playing track is not in the table and no row is selected", async () => {
    const user = userEvent.setup();
    render(<Screen />);
    // Playing from elsewhere (Review, say): a file that is not one of the rows.
    usePlayer.getState().select({ id: 99, name: "Elsewhere" });
    await waitFor(() => expect(element?.getAttribute("src")).toContain("/99"));
    loaded(300);
    expect(useStepPlace.getState()).toMatchObject({ focus: null, playing: null });
    await user.click(next());
    await waitFor(() => expect(usePlayer.getState().file?.id).toBe(1));
    expect(selectedTune()).toContain("Tune 1");
  });

  it("steps from the selected row when the playing track is not in the table", async () => {
    const user = userEvent.setup();
    render(<Screen />);
    usePlayer.getState().select({ id: 99, name: "Elsewhere" });
    await waitFor(() => expect(element?.getAttribute("src")).toContain("/99"));
    loaded(300);
    await user.click(screen.getByText("Tune 3"));
    expect(selectedTune()).toContain("Tune 3");
    await user.click(next());
    await waitFor(() => expect(usePlayer.getState().file?.id).toBe(4));
    expect(selectedTune()).toContain("Tune 4");
  });

  it("steps from the playing track's row, not from a row clicked while it plays", async () => {
    const user = userEvent.setup();
    render(<Screen />);
    await playRow(user, 2);
    // Browsing: the selection moves to row 5, the track keeps playing.
    await user.click(screen.getByText("Tune 5"));
    expect(selectedTune()).toContain("Tune 5");
    expect(usePlayer.getState().file?.id).toBe(2);
    expect(next()).toBeEnabled();
    await user.click(next());
    await waitFor(() => expect(usePlayer.getState().file?.id).toBe(3));
    expect(selectedTune()).toContain("Tune 3");
  });

  it("steps from the playing track after the table is sorted again, though its selection has not come back yet", async () => {
    const user = userEvent.setup();
    const view = render(<Screen />);
    await playRow(user, 3);
    // Sorted the other way: 5, 4, 3, 2, 1. The backend has not yet said where the selected track went.
    const reversed = (index: number) => tableRow(5 - index, { title: `Tune ${5 - index}` });
    view.rerender(<Screen rows={reversed} resetKey="b" positionOf={() => new Promise(() => {})} />);
    await user.click(next());
    // After Tune 3 in the new order comes Tune 2.
    await waitFor(() => expect(usePlayer.getState().file?.id).toBe(2));
    expect(selectedTune()).toContain("Tune 2");
  });

  it("steps from the playing track after the table is filtered", async () => {
    const user = userEvent.setup();
    const view = render(<Screen />);
    await playRow(user, 3);
    // Filtered to the tunes 3, 4 and 5.
    const filtered = (index: number) => (index < 3 ? tableRow(index + 3, { title: `Tune ${index + 3}` }) : undefined);
    view.rerender(<Screen rows={filtered} total={3} resetKey="c" positionOf={async () => 0} />);
    await user.click(next());
    await waitFor(() => expect(usePlayer.getState().file?.id).toBe(4));
  });

  it("asks the backend where the playing track is when it is not among the rows held, and scrolls to the row after it", async () => {
    const user = userEvent.setup();
    const view = render(<Screen />);
    await playRow(user, 4);
    // 1000 rows, only the first ten held; the playing track (recording 104) is row 500 of this order.
    const held = (index: number) => (index < 10 ? tableRow(index + 11, { title: `Tune ${index + 11}` }) : undefined);
    const asked: number[] = [];
    view.rerender(
      <Screen
        rows={held}
        total={1000}
        resetKey="d"
        positionOf={async (recordingId) => {
          asked.push(recordingId);
          return recordingId === 104 ? 500 : null;
        }}
      />,
    );
    await waitFor(() => expect(useStepPlace.getState().playing).toBe(500));
    expect(asked).toContain(104);
    await user.click(next());
    // Row 501 has not arrived: the selection goes to it, and it plays when it does.
    await waitFor(() => expect(useStepPlace.getState().focus).toBe(501));
    expect(usePlayer.getState().file?.id).toBe(4);
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

  it("plays the row below even when it has not arrived yet: it scrolls to it and plays it when it does", async () => {
    const user = userEvent.setup();
    const view = render(<Screen rows={(index) => (index === 1 ? undefined : tunes(5)(index))} />);
    await playRow(user, 1);
    await user.click(next());
    // The selection has moved to the row, which is being fetched; the track has not changed yet.
    await waitFor(() => expect(useStepPlace.getState().focus).toBe(1));
    expect(usePlayer.getState().file?.id).toBe(1);
    // Its window arrives.
    view.rerender(<Screen rows={tunes(5)} />);
    await waitFor(() => expect(usePlayer.getState().file?.id).toBe(2));
    expect(selectedTune()).toContain("Tune 2");
  });

  it("drops a step that waits for a row when the selection moves on meanwhile", async () => {
    const user = userEvent.setup();
    const view = render(<Screen rows={(index) => (index === 1 ? undefined : tunes(5)(index))} />);
    await playRow(user, 1);
    await user.click(next());
    await waitFor(() => expect(useStepPlace.getState().focus).toBe(1));
    await user.click(screen.getByText("Tune 4"));
    view.rerender(<Screen rows={tunes(5)} />);
    // Row 2 has arrived, but the user has gone elsewhere: it is not played.
    expect(usePlayer.getState().file?.id).toBe(1);
    expect(selectedTune()).toContain("Tune 4");
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

describe("the commands in the palette", () => {
  const listed = () => listedActions(usePaletteRegistry.getState().sources);
  const command = (id: string) => listed().find((action) => action.id === id);

  it("lists Previous track and Next track, with no key of their own, and runs the same steps as the buttons", async () => {
    const user = userEvent.setup();
    render(<Screen />);
    await playRow(user, 2);
    const nextCommand = command("player:next");
    const previousCommand = command("player:previous");
    expect(nextCommand).toMatchObject({ label: tx("player:next") });
    expect(previousCommand).toMatchObject({ label: tx("player:previous") });
    expect(nextCommand?.shortcut).toBeUndefined();
    expect(previousCommand?.shortcut).toBeUndefined();

    act(() => nextCommand?.run());
    await waitFor(() => expect(usePlayer.getState().file?.id).toBe(3));
    await waitFor(() => expect(element?.getAttribute("src")).toContain("/3"));
    loaded(300);
    act(() => command("player:previous")?.run());
    await waitFor(() => expect(usePlayer.getState().file?.id).toBe(2));
  });

  it("leaves a command out where its button is off: Next at the last row, both with nothing playing", async () => {
    const user = userEvent.setup();
    render(<Screen />);
    expect(command("player:next")).toBeUndefined();
    expect(command("player:previous")).toBeUndefined();
    await playRow(user, 5);
    expect(command("player:next")).toBeUndefined();
    expect(command("player:previous")).toBeDefined();
  });
});

describe("Play next", () => {
  const playNextSwitch = () => within(bar()).getByRole("button", { name: tx("player:playNext") });
  /** The playing file plays to its end, as the browser reports it. */
  function ends() {
    act(() => {
      element?.dispatchEvent(new Event("ended"));
    });
  }
  /** Waits for the player to have opened file `n`, and loads it. */
  async function opens(n: number) {
    await waitFor(() => expect(usePlayer.getState().file?.id).toBe(n));
    await waitFor(() => expect(element?.getAttribute("src")).toContain(`/${n}`));
    loaded(300);
  }

  it("is a switch beside Next, off to begin with, that shows whether it is on", async () => {
    const user = userEvent.setup();
    render(<Screen />);
    await playRow(user, 1);
    expect(playNextSwitch()).toHaveAttribute("aria-pressed", "false");
    expect(playNextSwitch()).toHaveAttribute("title", tx("player:playNextTip"));
    await user.click(playNextSwitch());
    expect(playNextSwitch()).toHaveAttribute("aria-pressed", "true");
    expect(usePlayer.getState().playNext).toBe(true);
    await user.click(playNextSwitch());
    expect(playNextSwitch()).toHaveAttribute("aria-pressed", "false");
  });

  it("plays the next row when a track ends, if it is on, from the start and playing", async () => {
    const user = userEvent.setup();
    render(<Screen />);
    await playRow(user, 2);
    await user.click(playNextSwitch());
    ends();
    await opens(3);
    expect(usePlayer.getState()).toMatchObject({ position: 0, playing: true });
  });

  it("leaves the table's selection and focus alone when a track ends and the next one plays", async () => {
    const user = userEvent.setup();
    const shown = vi.fn();
    render(<Screen onSelectionChange={shown} />);
    await playRow(user, 2);
    await user.click(playNextSwitch());
    await user.click(screen.getByText("Tune 1"));
    const focused = document.activeElement;
    const told = shown.mock.calls.length;
    ends();
    await opens(3);
    expect(usePlayer.getState().file?.id).toBe(3);
    expect(selectedTune()).toContain("Tune 1");
    expect(document.activeElement).toBe(focused);
    // The Details panel is never told of another row.
    expect(shown.mock.calls.slice(told).every((call) => call[0]?.title === "Tune 1")).toBe(true);
    // And on again, down the list, from the playing track and not the selection.
    ends();
    await opens(4);
    expect(selectedTune()).toContain("Tune 1");
    expect(shown.mock.calls.slice(told).every((call) => call[0]?.title === "Tune 1")).toBe(true);
  });

  it("goes on from the playing row, in the table's order, whatever row is selected", async () => {
    const user = userEvent.setup();
    const order = [4, 9, 2];
    const rows = (index: number) =>
      index < order.length ? tableRow(order[index], { title: `Tune ${order[index]}` }) : undefined;
    render(<Screen rows={rows} total={3} />);
    await playRow(user, 4);
    await user.click(playNextSwitch());
    await user.click(screen.getByText("Tune 2"));
    ends();
    await opens(9);
  });

  it("stops when a track ends, if it is off", async () => {
    const user = userEvent.setup();
    render(<Screen />);
    await playRow(user, 2);
    ends();
    expect(usePlayer.getState()).toMatchObject({ playing: false, position: 300 });
    expect(usePlayer.getState().file?.id).toBe(2);
    expect(opened).toEqual([2]);
  });

  it("stops at the last row", async () => {
    const user = userEvent.setup();
    render(<Screen />);
    await playRow(user, 4);
    await user.click(playNextSwitch());
    ends();
    await opens(5);
    ends();
    expect(usePlayer.getState()).toMatchObject({ playing: false, position: 300 });
    expect(usePlayer.getState().file?.id).toBe(5);
    expect(opened).toEqual([4, 5]);
  });

  it("stops at a next row that can't be played, instead of looping or skipping on", async () => {
    refused[3] = "missing";
    const user = userEvent.setup();
    render(<Screen />);
    await playRow(user, 2);
    await user.click(playNextSwitch());
    ends();
    // Row 3 is chosen and shows why it can't play; nothing after it is opened.
    expect(await screen.findByRole("status")).toHaveTextContent(tx("player:refused.missing"));
    expect(usePlayer.getState().file?.id).toBe(3);
    ends();
    expect(opened).toEqual([2, 3]);
    expect(usePlayer.getState().file?.id).toBe(3);
    // Next, pressed by hand, still goes past it.
    await user.click(next());
    await opens(4);
  });

  it("survives a reload: the switch is kept in storage and a new player starts with it", async () => {
    const user = userEvent.setup();
    render(<Screen />);
    await playRow(user, 1);
    await user.click(playNextSwitch());
    expect(localStorage.getItem(PLAY_NEXT_STORAGE_KEY)).toBe("true");
    const reloaded = createPlayer({
      audio: () => document.createElement("audio"),
      check: () => Promise.resolve(null),
      urlOf: (id) => `media://${id}`,
      playNext: browserPlayNextPrefs,
    });
    expect(reloaded.getState().playNext).toBe(true);
  });

  it("still switches when storage throws", async () => {
    vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => {
      throw new DOMException("full", "QuotaExceededError");
    });
    vi.spyOn(Storage.prototype, "getItem").mockImplementation(() => {
      throw new DOMException("blocked", "SecurityError");
    });
    const user = userEvent.setup();
    render(<Screen />);
    await playRow(user, 1);
    await user.click(playNextSwitch());
    expect(playNextSwitch()).toHaveAttribute("aria-pressed", "true");
    ends();
    await opens(2);
    const started = createPlayer({
      audio: () => document.createElement("audio"),
      check: () => Promise.resolve(null),
      urlOf: (id) => `media://${id}`,
      playNext: browserPlayNextPrefs,
    });
    expect(started.getState().playNext).toBe(false);
  });

  it("is a palette command that turns it on and off, with no key of its own", async () => {
    const user = userEvent.setup();
    render(<Screen />);
    const command = () => listedActions(usePaletteRegistry.getState().sources).find((a) => a.id === "player:playNext");
    // Listed before anything plays, and says what it will do.
    expect(command()).toMatchObject({ label: tx("player:playNextTurnOn") });
    expect(command()?.shortcut).toBeUndefined();
    act(() => command()?.run());
    expect(usePlayer.getState().playNext).toBe(true);
    await waitFor(() => expect(command()?.label).toBe(tx("player:playNextTurnOff")));
    await playRow(user, 1);
    expect(playNextSwitch()).toHaveAttribute("aria-pressed", "true");
    act(() => command()?.run());
    expect(usePlayer.getState().playNext).toBe(false);
  });
});
