import { clearMocks } from "@tauri-apps/api/mocks";
import { cleanup, fireEvent, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { renderApp } from "../app/testApp";
import type { BridgeReason, GapSuggestion } from "../bindings";
import i18n from "../i18n";
import { type FakePlaylist, playlistBackend } from "../test/playlistBackend";
import { tableRow } from "../test/trackTableIpc";
import { tx } from "../test/tx";
import { reasonText } from "./gapReasons";
import { addPoint, atOf, energyOf, entryAt, movePoint, nextPoint, runs, xOf, yOf } from "./timelineShape";

// The timeline and "Fill the gap" on the Playlists screen (3C-P2b; ROADMAP
// 3.5). The whole app is rendered: Undo is in the top bar. The chart is drawn
// 600 by 200 here, the size of its own coordinates, so a pointer at (x, y)
// lands on drawing coordinate (x, y).

beforeEach(() => {
  vi.spyOn(HTMLElement.prototype, "clientHeight", "get").mockReturnValue(28 * 11);
  vi.spyOn(Element.prototype, "getBoundingClientRect").mockImplementation(function (this: Element) {
    const box = this.getAttribute("role") === "group" ? [600, 200] : this.getAttribute("role") === "grid" ? [800, 400] : [0, 0];
    const [width, height] = box as [number, number];
    return { x: 0, y: 0, top: 0, left: 0, right: width, bottom: height, width, height, toJSON: () => ({}) };
  });
});

afterEach(() => {
  cleanup();
  clearMocks();
  vi.restoreAllMocks();
  document.body.innerHTML = "";
});

/** Tune 0 (5, 120 BPM, 8A, 3:00), Tune 1 (no energy, 124, 8A, 3:20), Tune 2 (8, 140, 3B, 4:00); Bridge is outside the playlist. */
const TUNES = [
  tableRow(0, { title: "Tune 0", artist: null, recordingId: 100, libraryTrackId: 1, energy: 5, bpm: 120, key: "8A", timeMs: 180_000 }),
  tableRow(1, { title: "Tune 1", artist: null, recordingId: 101, libraryTrackId: 2, energy: null, bpm: 124, key: "8A", timeMs: 200_000 }),
  tableRow(2, { title: "Tune 2", artist: null, recordingId: 102, libraryTrackId: 3, energy: 8, bpm: 140, key: "3B", timeMs: 240_000 }),
  tableRow(3, { title: "Bridge", artist: "Some Artist", recordingId: 103, libraryTrackId: 4, energy: 6, bpm: 130, key: "9A", timeMs: 200_000 }),
];
const ARC: FakePlaylist = {
  id: 1,
  name: "Arc",
  origin: "app",
  sourcePath: null,
  entries: [
    [1, 100],
    [2, 101],
    [3, 102],
  ],
};

const REASONS: BridgeReason[] = [
  { code: "keyBridge", from: "8A", via: "9A", to: "10A" },
  { code: "energyStep", from: 5, via: 6, to: 7 },
];
const BRIDGE: GapSuggestion = { row: TUNES[3]!, score: 80, reasons: REASONS };

async function openTimeline(options: Partial<Parameters<typeof playlistBackend>[0]> = {}) {
  const backend = playlistBackend({ rows: TUNES, playlists: [ARC], gap: () => [BRIDGE], ...options });
  renderApp("/playlists?playlist=1");
  await userEvent.click(await screen.findByRole("button", { name: tx("playlists:timeline.button") }));
  await screen.findByRole("table");
  return backend;
}

const chart = () => screen.getByRole("group", { name: tx("playlists:timeline.chart.label") });
const plot = () => chart().querySelector("[data-plot]") as SVGRectElement;
const row = (entry: number) => document.querySelector(`[data-entry-row="${entry}"]`) as HTMLElement;
const pointer = (x: number, y: number) => ({ button: 0, clientX: x, clientY: y, pointerId: 1 });
const handles = () => within(chart()).queryAllByRole("slider");

describe("the timeline", () => {
  it("lists the entries in order with when each starts and the total length", async () => {
    await openTimeline();
    const entries = Array.from(document.querySelectorAll("[data-entry-row]"));
    expect(entries.map((entry) => within(entry as HTMLElement).getAllByRole("cell")[2]?.textContent)).toEqual(["Tune 0", "Tune 1", "Tune 2"]);
    expect(entries.map((entry) => within(entry as HTMLElement).getAllByRole("cell")[1]?.textContent)).toEqual(["0:00", "3:00", "6:20"]);
    expect(screen.getByText(tx("playlists:timeline.total", { time: "10:20" }))).toBeInTheDocument();
  });

  it("shows the timeline only after it is asked for", async () => {
    const backend = playlistBackend({ rows: TUNES, playlists: [ARC] });
    renderApp("/playlists?playlist=1");
    await screen.findByRole("grid");
    expect(screen.queryByRole("table")).not.toBeInTheDocument();
    expect(backend.called("playlist_timeline")).toHaveLength(0);
  });

  it("draws an entry with no energy as a gap in the curve, never as a low point", async () => {
    await openTimeline();
    // Two known energies, a tick for the unknown one, and the curve broken in two.
    expect(Array.from(chart().querySelectorAll("[data-entry]")).map((dot) => dot.getAttribute("data-entry"))).toEqual(["1", "3"]);
    expect(chart().querySelectorAll("[data-gap]")).toHaveLength(1);
    expect(chart().querySelector('[data-gap="2"]')).not.toBeNull();
    expect(chart().querySelectorAll("[data-curve]")).toHaveLength(2);
    expect(within(row(2)).getByText(tx("playlists:timeline.unknown", {}), { selector: "span" })).toBeInTheDocument();
  });

  it("marks a clash differently from something unknown, in the chart and in words", async () => {
    await openTimeline();
    // Tune 0 to 1: only the energy is unknown. Tune 1 to 2: the key and the tempo clash.
    expect(chart().querySelector('[data-from="1"]')).toHaveAttribute("data-flag", "unknown");
    expect(chart().querySelector('[data-from="2"]')).toHaveAttribute("data-flag", "clash");
    const flags = (entry: number) =>
      Array.from(document.querySelectorAll(`[data-gap-row="${entry}"] [data-flow]`)).map((flag) => [flag.getAttribute("data-signal"), flag.getAttribute("data-flow")]);
    expect(flags(1)).toEqual([["energy", "unknown"]]);
    expect(flags(2)).toEqual([
      ["key", "clash"],
      ["bpm", "clash"],
      ["energy", "unknown"],
    ]);
    expect(screen.getByText(tx("playlists:timeline.flag.key.clash"))).toBeInTheDocument();
    expect(screen.getByText(tx("playlists:timeline.flag.bpm.clash", { delta: "+16" }))).toBeInTheDocument();
  });
});

describe("the target shape", () => {
  it("is placed by pressing the chart, as one change with a named Undo", async () => {
    const backend = await openTimeline();
    expect(screen.getByRole("button", { name: tx("playlists:timeline.target.clear") })).toBeDisabled();
    fireEvent.pointerDown(plot(), pointer(xOf(0.5), yOf(8)));

    await waitFor(() => expect(backend.called("set_playlist_target")).toHaveLength(1));
    expect(backend.called("set_playlist_target")[0]?.args).toMatchObject({ id: 1, points: [{ at: 0.5, energy: 8 }] });
    expect(await screen.findByRole("button", { name: tx("shell:undo.action.setPlaylistTarget", { name: "Arc" }) })).toBeInTheDocument();
    await waitFor(() => expect(within(chart()).getByRole("slider")).toHaveAttribute("aria-valuenow", "8"));
    expect(chart().querySelector("[data-target]")).not.toBeNull();
    expect(await screen.findByText(tx("playlists:done.targetSet"))).toBeInTheDocument();
  });

  it("moves a point by dragging, and saves only when it is let go", async () => {
    const backend = playlistBackend({ rows: TUNES, playlists: [ARC] });
    backend.state.targets[1] = [{ at: 0.2, energy: 4 }];
    renderApp("/playlists?playlist=1");
    await userEvent.click(await screen.findByRole("button", { name: tx("playlists:timeline.button") }));
    const handle = await waitFor(() => within(chart()).getByRole("slider"));

    fireEvent.pointerDown(handle, pointer(xOf(0.2), yOf(4)));
    fireEvent.pointerMove(chart(), pointer(xOf(0.6), yOf(7)));
    expect(backend.called("set_playlist_target")).toHaveLength(0);
    fireEvent.pointerUp(chart(), pointer(xOf(0.6), yOf(7)));

    await waitFor(() => expect(backend.called("set_playlist_target")).toHaveLength(1));
    expect(backend.called("set_playlist_target")[0]?.args).toMatchObject({ points: [{ at: 0.6, energy: 7 }] });
  });

  it("a point let go where it was changes nothing", async () => {
    const backend = playlistBackend({ rows: TUNES, playlists: [ARC] });
    backend.state.targets[1] = [{ at: 0.2, energy: 4 }];
    renderApp("/playlists?playlist=1");
    await userEvent.click(await screen.findByRole("button", { name: tx("playlists:timeline.button") }));
    const handle = await waitFor(() => within(chart()).getByRole("slider"));
    fireEvent.pointerDown(handle, pointer(xOf(0.2), yOf(4)));
    fireEvent.pointerUp(chart(), pointer(xOf(0.2), yOf(4)));
    expect(backend.called("set_playlist_target")).toHaveLength(0);
  });

  it("moves a point with the arrow keys and takes it out with Delete", async () => {
    const backend = playlistBackend({ rows: TUNES, playlists: [ARC] });
    backend.state.targets[1] = [
      { at: 0.2, energy: 4 },
      { at: 0.8, energy: 6 },
    ];
    renderApp("/playlists?playlist=1");
    await userEvent.click(await screen.findByRole("button", { name: tx("playlists:timeline.button") }));
    const [first] = await waitFor(() => {
      const found = handles();
      expect(found).toHaveLength(2);
      return found;
    });
    fireEvent.keyDown(first!, { key: "ArrowUp" });
    await waitFor(() => expect(backend.called("set_playlist_target")).toHaveLength(1));
    expect(backend.called("set_playlist_target")[0]?.args).toMatchObject({
      points: [
        { at: 0.2, energy: 5 },
        { at: 0.8, energy: 6 },
      ],
    });

    await waitFor(() => expect(handles()[0]).toHaveAttribute("aria-valuenow", "5"));
    fireEvent.keyDown(handles()[1]!, { key: "Delete" });
    await waitFor(() => expect(backend.called("set_playlist_target")).toHaveLength(2));
    expect(backend.called("set_playlist_target")[1]?.args).toMatchObject({ points: [{ at: 0.2, energy: 5 }] });
  });

  it("taking out the last point clears the target", async () => {
    const backend = playlistBackend({ rows: TUNES, playlists: [ARC] });
    backend.state.targets[1] = [{ at: 0.5, energy: 6 }];
    renderApp("/playlists?playlist=1");
    await userEvent.click(await screen.findByRole("button", { name: tx("playlists:timeline.button") }));
    fireEvent.keyDown(await waitFor(() => within(chart()).getByRole("slider")), { key: "Delete" });
    await waitFor(() => expect(backend.called("clear_playlist_target")).toHaveLength(1));
    expect(backend.called("set_playlist_target")).toHaveLength(0);
     expect(await screen.findByRole("button", { name: tx("shell:undo.action.clearPlaylistTarget", { name: "Arc" }) })).toBeInTheDocument();
  });

  it("shows the error when a change cannot be saved, and the picture goes back to what is saved", async () => {
    const backend = await openTimeline({ refusing: ["set_playlist_target"] });
    fireEvent.pointerDown(plot(), pointer(xOf(0.5), yOf(8)));
    expect(await screen.findByRole("alert")).toHaveTextContent(tx("playlists:error.notFound"));
    expect(backend.called("set_playlist_target")).toHaveLength(1);
    await waitFor(() => expect(handles()).toHaveLength(0));
    expect(screen.queryByText(tx("playlists:done.targetSet"))).not.toBeInTheDocument();
  });

  it("clears the whole target as one change with a named Undo, and Undo brings it back", async () => {
    const backend = playlistBackend({ rows: TUNES, playlists: [ARC] });
    backend.state.targets[1] = [
      { at: 0.2, energy: 4 },
      { at: 0.8, energy: 6 },
    ];
    renderApp("/playlists?playlist=1");
    await userEvent.click(await screen.findByRole("button", { name: tx("playlists:timeline.button") }));
    await waitFor(() => expect(handles()).toHaveLength(2));
    await userEvent.click(screen.getByRole("button", { name: tx("playlists:timeline.target.clear") }));

    await waitFor(() => expect(backend.called("clear_playlist_target")).toHaveLength(1));
    expect(backend.called("clear_playlist_target")[0]?.args).toMatchObject({ id: 1 });
    await waitFor(() => expect(handles()).toHaveLength(0));
    await userEvent.click(await screen.findByRole("button", { name: tx("shell:undo.action.clearPlaylistTarget", { name: "Arc" }) }));
    await waitFor(() => expect(handles()).toHaveLength(2));
  });

  it("adds a point with the Add point button in the widest empty stretch", async () => {
    const backend = await openTimeline();
    await userEvent.click(screen.getByRole("button", { name: tx("playlists:timeline.target.add") }));
    await waitFor(() => expect(backend.called("set_playlist_target")).toHaveLength(1));
    expect(backend.called("set_playlist_target")[0]?.args).toMatchObject({ points: [{ at: 0.5, energy: 5 }] });
  });
});

describe("fill the gap", () => {
  const suggestions = () => screen.findByRole("list", { name: tx("playlists:timeline.fill.list") });

  it("lists each suggestion with the reasons it fits, worded from the reason codes", async () => {
    const asked: (number | null)[] = [];
    await openTimeline({
      gap: (after) => {
        asked.push(after);
        return [BRIDGE];
      },
    });
    await userEvent.click(screen.getByRole("button", { name: tx("playlists:timeline.fill.between", { from: "Tune 0", to: "Tune 1" }) }));
    const list = await suggestions();
    expect(within(list).getByText("Some Artist - Bridge")).toBeInTheDocument();
    const reasons = [
      tx("playlists:gap.keyBridge", { from: "8A", via: "9A", to: "10A" }),
      tx("playlists:gap.energyStep", { from: 5, via: 6, to: 7 }),
    ].join(", ");
    expect(within(list).getByText(reasons)).toBeInTheDocument();
    expect(asked[0]).toBe(1);
  });

  it("inserts at the place of the gap between two tracks as one change", async () => {
    const backend = await openTimeline();
    await userEvent.click(screen.getByRole("button", { name: tx("playlists:timeline.fill.between", { from: "Tune 0", to: "Tune 1" }) }));
    await suggestions();
    await userEvent.click(screen.getByRole("button", { name: tx("playlists:timeline.fill.insertFor", { title: "Some Artist - Bridge" }) }));

    await waitFor(() => expect(backend.called("add_tracks_to_playlist")).toHaveLength(1));
    expect(backend.called("add_tracks_to_playlist")[0]?.args).toMatchObject({ id: 1, tracks: [4], at: 1 });
    expect(await screen.findByText(tx("playlists:done.inserted", { title: "Some Artist - Bridge", position: 2 }))).toBeInTheDocument();
    expect(await screen.findByRole("button", { name: tx("shell:undo.action.addToPlaylist", { count: 1, name: "Arc" }) })).toBeInTheDocument();
    expect(backend.state.playlists[0]?.entries.map(([, recording]) => recording)).toEqual([100, 103, 101, 102]);
  });

  it("shows the inserted track in the timeline and in the playlist's table", async () => {
    await openTimeline();
    await userEvent.click(screen.getByRole("button", { name: tx("playlists:timeline.fill.between", { from: "Tune 0", to: "Tune 1" }) }));
    await suggestions();
    await userEvent.click(screen.getByRole("button", { name: tx("playlists:timeline.fill.insertFor", { title: "Some Artist - Bridge" }) }));

    await waitFor(() => expect(document.querySelectorAll("[data-entry-row]")).toHaveLength(4));
    const order = Array.from(document.querySelectorAll("[data-entry-row]")).map((entry) => within(entry as HTMLElement).getAllByRole("cell")[2]?.textContent);
    expect(order).toEqual(["Tune 0", "Some Artist - Bridge", "Tune 1", "Tune 2"]);
    await waitFor(() => {
      const titles = within(screen.getByRole("grid")).getAllByRole("row").slice(1).map((r) => /Tune \d|Bridge/.exec(r.textContent ?? "")?.[0]);
      expect(titles).toEqual(["Tune 0", "Bridge", "Tune 1", "Tune 2"]);
    });
    // The suggestions close once the track is in.
    expect(screen.queryByRole("list", { name: tx("playlists:timeline.fill.list") })).not.toBeInTheDocument();
  });

  it("leaves out a suggestion whose only reason has no tempo to word", async () => {
    const unnamed: GapSuggestion = { row: TUNES[3]!, score: 50, reasons: [{ code: "bpmFits", from: 120, via: null, to: 124 }] };
    await openTimeline({ gap: () => [unnamed] });
    await userEvent.click(screen.getByRole("button", { name: tx("playlists:timeline.fill.before") }));
    expect(await screen.findByText(tx("playlists:timeline.fill.none"))).toBeInTheDocument();
  });

  it("offers the start and the end of the playlist, inserting first and last", async () => {
    const asked: (number | null)[] = [];
    const backend = await openTimeline({
      gap: (after) => {
        asked.push(after);
        return [BRIDGE];
      },
    });
    await userEvent.click(screen.getByRole("button", { name: tx("playlists:timeline.fill.before") }));
    await suggestions();
    await userEvent.click(screen.getByRole("button", { name: tx("playlists:timeline.fill.insertFor", { title: "Some Artist - Bridge" }) }));
    await waitFor(() => expect(backend.called("add_tracks_to_playlist")[0]?.args).toMatchObject({ at: 0 }));

    await userEvent.click(await screen.findByRole("button", { name: tx("playlists:timeline.fill.after", { title: "Tune 2" }) }));
    await suggestions();
    await userEvent.click(screen.getByRole("button", { name: tx("playlists:timeline.fill.insertFor", { title: "Some Artist - Bridge" }) }));
    await waitFor(() => expect(backend.called("add_tracks_to_playlist")).toHaveLength(2));
    expect(backend.called("add_tracks_to_playlist")[1]?.args).toMatchObject({ at: 4 });
    expect(asked).toContain(null);
    expect(asked).toContain(3);
  });

  it("says when nothing fits, without a list", async () => {
    await openTimeline({ gap: () => [] });
    await userEvent.click(screen.getByRole("button", { name: tx("playlists:timeline.fill.before") }));
    expect(await screen.findByText(tx("playlists:timeline.fill.none"))).toBeInTheDocument();
    expect(screen.queryByRole("list", { name: tx("playlists:timeline.fill.list") })).not.toBeInTheDocument();
  });
});

describe("the reasons", () => {
  const t = i18n.getFixedT("en", "playlists");
  it("word a reason by which neighbours it has", () => {
    const fits = (from: string | null, to: string | null): BridgeReason => ({ code: "keyFits", from, via: "9A", to });
    expect(reasonText(t, fits("8A", "10A"), "camelot")).toBe(tx("playlists:gap.keyFits", { from: "8A", via: "9A", to: "10A" }));
    expect(reasonText(t, fits("8A", null), "camelot")).toBe(tx("playlists:gap.keyFitsAfter", { from: "8A", via: "9A" }));
    expect(reasonText(t, fits(null, "10A"), "camelot")).toBe(tx("playlists:gap.keyFitsBefore", { via: "9A", to: "10A" }));
    expect(reasonText(t, fits(null, null), "camelot")).toBe(tx("playlists:gap.keyFitsOnly", { via: "9A" }));
  });

  it("leave out a tempo reason that has no tempo to word, instead of saying 0", () => {
    expect(reasonText(t, { code: "bpmFits", from: 120, via: null, to: 124 }, "camelot")).toBeNull();
    expect(reasonText(t, { code: "bpmStep", from: 120, via: 130, to: null }, "camelot")).toBeNull();
    expect(reasonText(t, { code: "bpmFits", from: null, via: 130, to: 124 }, "camelot")).toBe(
      tx("playlists:gap.bpmFitsBefore", { via: 130, to: 124 }),
    );
  });

  it("show keys in the user's notation", () => {
    const bridge: BridgeReason = { code: "keyBridge", from: "8A", via: "9A", to: "10A" };
    expect(reasonText(t, bridge, "musical_standard")).not.toBe(reasonText(t, bridge, "camelot"));
  });
});

describe("the drawing's numbers", () => {
  it("places an entry along the set, one alone in the middle", () => {
    expect([entryAt(0, 3), entryAt(1, 3), entryAt(2, 3), entryAt(0, 1)]).toEqual([0, 0.5, 1, 0.5]);
  });

  it("breaks a line at an unknown value", () => {
    const xs = [0, 10, 20, 30];
    expect(runs([5, null, 7, 8], xs, (v) => v).map((run) => run.length)).toEqual([1, 2]);
    expect(runs([null, null], [0, 10], (v) => v)).toEqual([]);
  });

  it("turns a pointer place into a point and back", () => {
    expect(atOf(xOf(0.37))).toBe(0.37);
    expect(energyOf(yOf(7))).toBe(7);
    expect(energyOf(-50)).toBe(10);
    expect(energyOf(5000)).toBe(1);
  });

  it("keeps points in order, one to a place, and in range", () => {
    expect(addPoint([{ at: 0.5, energy: 3 }], { at: 0.2, energy: 12 })).toEqual([
      { at: 0.2, energy: 10 },
      { at: 0.5, energy: 3 },
    ]);
    expect(addPoint([{ at: 0.5, energy: 3 }], { at: 0.5, energy: 8 })).toEqual([{ at: 0.5, energy: 8 }]);
    expect(movePoint([{ at: 0.5, energy: 3 }], 0, { at: 4, energy: -2 })).toEqual([{ at: 1, energy: 1 }]);
  });

  it("puts the next point in the widest stretch at the line's height", () => {
    expect(nextPoint([])).toEqual({ at: 0.5, energy: 5 });
    expect(nextPoint([{ at: 0.2, energy: 4 }])).toEqual({ at: 0.6, energy: 4 });
    expect(nextPoint([{ at: 0.1, energy: 2 }, { at: 0.5, energy: 6 }])).toEqual({ at: 0.75, energy: 6 });
  });
});
