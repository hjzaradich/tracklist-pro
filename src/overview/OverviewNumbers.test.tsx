import { clearMocks } from "@tauri-apps/api/mocks";
import { cleanup, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it } from "vitest";
import { renderApp } from "../app/testApp";
import type { GenreCount, Overview } from "../bindings";
import { fakeBackend } from "../test/overviewBackend";
import { tx } from "../test/tx";
import { energyBars, genreBars, TOP_GENRES } from "./Charts";

// The Overview's four numbers, the energy and genre bars and the strip for
// what's outside the Library (1cB-12). The numbers come from the Overview
// command, as the fake backend returns them.

afterEach(() => {
  cleanup();
  clearMocks();
});

const MINUTE_MS = 60_000;
const ENERGY_LEVELS = 10;

/** The rows of a chart, each as `[label, count]`, read from what is written beside the bars. */
function rowsOf(region: HTMLElement): [string, string][] {
  return within(region)
    .getAllByRole("listitem")
    .map((row) => [row.children[0]?.textContent ?? "", row.children[2]?.textContent ?? ""]);
}

const sum = (rows: [string, string][]) => rows.reduce((total, [, n]) => total + Number(n), 0);

function genres(counts: number[]): GenreCount[] {
  return counts.map((tracks, i) => ({ name: `Genre ${i + 1}`, tracks }));
}

describe("the four numbers", () => {
  it("show the tracks and their length, the crates, the analyzed and when it was last sent, as the command counted them", async () => {
    fakeBackend({
      library: 5,
      overview: {
        totalLengthMs: 90 * MINUTE_MS,
        inCrates: 3,
        analyzedByRekordbox: 4,
        lastSentMs: Date.UTC(2026, 9, 20, 12, 0),
      },
    });
    renderApp("/overview");
    const list = (await screen.findByText(tx("overview:numbers.inCrates"))).closest("ul");
    expect(list).not.toBeNull();
    const tiles = within(list as HTMLElement)
      .getAllByRole("listitem")
      .map((tile) => tile.textContent);
    expect(tiles).toEqual([
      "5" +
        tx("overview:numbers.tracks", { count: 5 }) +
        tx("overview:numbers.length", { hours: 1, minutes: 30 }),
      "3" + tx("overview:numbers.inCrates"),
      "4" + tx("overview:numbers.analyzed"),
      expect.stringContaining(tx("overview:numbers.lastSent")),
    ]);
  });

  it("show a length under an hour in minutes", async () => {
    fakeBackend({ overview: { totalLengthMs: 45 * MINUTE_MS } });
    renderApp("/overview");
    expect(await screen.findByText(tx("overview:numbers.lengthMinutes", { minutes: 45 }))).toBeInTheDocument();
  });

  it("don't say how long the Library is when no length is known", async () => {
    fakeBackend({ library: 3, overview: { totalLengthMs: 0 } });
    renderApp("/overview");
    const tracks = (await screen.findByText(tx("overview:numbers.tracks", { count: 3 }))).closest("li");
    expect(tracks?.textContent).toBe("3" + tx("overview:numbers.tracks", { count: 3 }));
  });

  it("say Not sent yet when the Library was never sent", async () => {
    fakeBackend({ overview: { lastSentMs: null } });
    renderApp("/overview");
    expect(await screen.findByText(tx("overview:numbers.neverSent"))).toBeInTheDocument();
    expect(screen.queryByText(tx("overview:numbers.lastSent"))).toBeNull();
  });

  it("don't count tracks as analyzed before rekordbox has been read", async () => {
    fakeBackend({ overview: { rekordboxRead: false, analyzedByRekordbox: 0 } });
    renderApp("/overview");
    expect(await screen.findByText(tx("overview:numbers.analyzedUnread"))).toBeInTheDocument();
    expect(screen.queryByText(tx("overview:numbers.analyzed"))).toBeNull();
  });

  it("say a number is still counting while background work can change it", async () => {
    fakeBackend({ overview: { counting: { files: false, rekordbox: true, quality: false } } });
    renderApp("/overview");
    const analyzed = (await screen.findByText(tx("overview:numbers.analyzed"))).closest("li");
    expect(analyzed).toHaveTextContent(tx("overview:todo.lineCounting"));
    const crates = screen.getByText(tx("overview:numbers.inCrates")).closest("li");
    expect(crates).not.toHaveTextContent(tx("overview:todo.lineCounting"));
  });
});

describe("the energy bars", () => {
  it("count the tracks at each energy, 1 to 10, and the tracks with no energy", async () => {
    const energy = [0, 2, 0, 0, 5, 0, 0, 1, 0, 3];
    fakeBackend({ overview: { energy, noEnergy: 4 } });
    renderApp("/overview");
    const region = await screen.findByRole("region", { name: tx("overview:charts.energy.title") });
    expect(rowsOf(region)).toEqual([
      ...energy.map((tracks, i): [string, string] => [String(i + 1), String(tracks)]),
      [tx("overview:charts.energy.none"), "4"],
    ]);
  });

  it("leave out the no energy row when every track has an energy", async () => {
    fakeBackend({ overview: { energy: Array(ENERGY_LEVELS).fill(1), noEnergy: 0 } });
    renderApp("/overview");
    const rows = rowsOf(await screen.findByRole("region", { name: tx("overview:charts.energy.title") }));
    expect(rows).toHaveLength(ENERGY_LEVELS);
    expect(rows.map(([label]) => label)).not.toContain(tx("overview:charts.energy.none"));
  });

  it("say no track has an energy, rather than draw empty bars", async () => {
    fakeBackend({ overview: { energy: Array(ENERGY_LEVELS).fill(0), noEnergy: 3 } });
    renderApp("/overview");
    const region = await screen.findByRole("region", { name: tx("overview:charts.energy.title") });
    expect(within(region).getByText(tx("overview:charts.energy.empty"))).toBeInTheDocument();
    expect(within(region).queryByRole("list")).toBeNull();
  });

  it("draw each bar as long as its count is against the biggest", async () => {
    fakeBackend({ overview: { energy: [4, 2, 0, 1, 0, 0, 0, 0, 0, 0], noEnergy: 0 } });
    renderApp("/overview");
    const region = await screen.findByRole("region", { name: tx("overview:charts.energy.title") });
    const widths = within(region)
      .getAllByRole("listitem")
      .slice(0, 4)
      .map((row) => (row.children[1]?.firstElementChild as HTMLElement).style.width);
    expect(widths).toEqual(["100%", "50%", "0%", "25%"]);
  });

  it("list the bars from energy 1 up, then No energy", () => {
    const bars = energyBars({ energy: [4, 2, 0, 0, 0, 0, 0, 0, 0, 0], noEnergy: 5 }, { none: "x" });
    expect(bars.map((bar) => bar.label)).toEqual([
      ...Array.from({ length: ENERGY_LEVELS }, (_, i) => String(i + 1)),
      "x",
    ]);
  });
});

describe("the genre bars", () => {
  it("show every genre, biggest first, and No genre, when there are few", async () => {
    fakeBackend({
      overview: {
        genres: [
          { name: "House", tracks: 5 },
          { name: "Techno", tracks: 2 },
        ],
        noGenre: 1,
      },
    });
    renderApp("/overview");
    const region = await screen.findByRole("region", { name: tx("overview:charts.genres.title") });
    expect(rowsOf(region)).toEqual([
      ["House", "5"],
      ["Techno", "2"],
      [tx("overview:charts.genres.none"), "1"],
    ]);
  });

  it("group the genres past the biggest ones as Other, and keep the track total", async () => {
    const many = genres([20, 19, 18, 17, 16, 15, 14, 13, 3, 2]);
    fakeBackend({ overview: { genres: many, noGenre: 4 } });
    renderApp("/overview");
    const rows = rowsOf(await screen.findByRole("region", { name: tx("overview:charts.genres.title") }));
    expect(rows).toEqual([
      ...many.slice(0, TOP_GENRES).map((g): [string, string] => [g.name, String(g.tracks)]),
      [tx("overview:charts.genres.other"), "5"],
      [tx("overview:charts.genres.none"), "4"],
    ]);
    expect(sum(rows)).toBe(many.reduce((total, g) => total + g.tracks, 0) + 4);
  });

  it("leave out No genre when every track has a genre", async () => {
    fakeBackend({ overview: { genres: [{ name: "House", tracks: 3 }], noGenre: 0 } });
    renderApp("/overview");
    const rows = rowsOf(await screen.findByRole("region", { name: tx("overview:charts.genres.title") }));
    expect(rows).toEqual([["House", "3"]]);
  });

  it("say no track has a genre, rather than draw only No genre", async () => {
    fakeBackend({ overview: { genres: [], noGenre: 3 } });
    renderApp("/overview");
    const region = await screen.findByRole("region", { name: tx("overview:charts.genres.title") });
    expect(within(region).getByText(tx("overview:charts.genres.empty"))).toBeInTheDocument();
  });

  it("keep a single genre too many as a bar of its own instead of an Other of one", () => {
    const labels = { other: "Other", none: "None" };
    const nine = genreBars({ genres: genres(Array(TOP_GENRES + 1).fill(3)), noGenre: 0 }, labels);
    expect(nine.map((bar) => bar.label)).not.toContain("Other");
    expect(nine).toHaveLength(TOP_GENRES + 1);
    const ten = genreBars({ genres: genres(Array(TOP_GENRES + 2).fill(3)), noGenre: 0 }, labels);
    expect(ten.at(-1)).toMatchObject({ label: "Other", tracks: 6 });
  });
});

describe("outside your Library", () => {
  const strip = () => screen.getByRole("region", { name: tx("overview:outside.title") });

  it("counts the tracks in All music that aren't in the Library, and the items that need a decision", async () => {
    fakeBackend({
      overview: { outsideLibrary: 1204 },
      review: { needsCloserLook: 2, conflicts: 1, total: 3 },
    });
    renderApp("/overview");
    await screen.findByRole("region", { name: tx("overview:outside.title") });
    expect(await within(strip()).findByText(tx("overview:outside.tracks", { count: 1204 }))).toBeInTheDocument();
    expect(await within(strip()).findByText(tx("overview:outside.decisions", { count: 3 }))).toBeInTheDocument();
  });

  it("says plainly when nothing is outside and nothing needs a decision", async () => {
    fakeBackend({ overview: { outsideLibrary: 0 } });
    renderApp("/overview");
    await screen.findByRole("region", { name: tx("overview:outside.title") });
    expect(within(strip()).getByText(tx("overview:outside.tracks", { count: 0 }))).toBeInTheDocument();
    expect(await within(strip()).findByText(tx("overview:outside.decisions", { count: 0 }))).toBeInTheDocument();
  });

  it.each([
    ["the Review queue", "shell:stages.review", "/review"],
    ["the All music list", "shell:stages.allMusic", "/all-music"],
  ])("opens %s", async (_name, key, path) => {
    fakeBackend({ overview: { outsideLibrary: 3 } });
    const { router } = renderApp("/overview");
    await screen.findByRole("region", { name: tx("overview:outside.title") });
    await userEvent.click(within(strip()).getByRole("link", { name: tx(key) }));
    await waitFor(() => expect(router.state.location.pathname).toBe(path));
  });

  it("takes the user to the music folders panel", async () => {
    fakeBackend();
    renderApp("/overview");
    await screen.findByRole("region", { name: tx("overview:outside.title") });
    const folders = await screen.findByRole("region", { name: tx("firstRun:folders.title") });
    await userEvent.click(within(strip()).getByRole("button", { name: tx("overview:outside.manageFolders") }));
    expect(folders.parentElement).toHaveFocus();
  });

  it("isn't drawn while the Library is empty", async () => {
    fakeBackend({ library: 0 });
    renderApp("/overview");
    await screen.findByRole("heading", { name: tx("firstRun:title") });
    expect(screen.queryByRole("region", { name: tx("overview:outside.title") })).toBeNull();
  });
});

describe("the Overview command's numbers reach the screen unchanged", () => {
  it("shows the counts it was given for energy, genres and outside the Library", async () => {
    const given: Partial<Overview> = {
      energy: [1, 1, 1, 1, 1, 1, 1, 1, 1, 1],
      noEnergy: 2,
      genres: [{ name: "House", tracks: 7 }],
      noGenre: 3,
      outsideLibrary: 9,
    };
    fakeBackend({ overview: given });
    renderApp("/overview");
    const energy = rowsOf(await screen.findByRole("region", { name: tx("overview:charts.energy.title") }));
    expect(sum(energy)).toBe(12);
    const genreRows = rowsOf(screen.getByRole("region", { name: tx("overview:charts.genres.title") }));
    expect(sum(genreRows)).toBe(10);
    expect(
      within(screen.getByRole("region", { name: tx("overview:outside.title") })).getByText(
        tx("overview:outside.tracks", { count: 9 }),
      ),
    ).toBeInTheDocument();
  });
});
