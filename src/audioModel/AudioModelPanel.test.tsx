import { QueryClientProvider } from "@tanstack/react-query";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { cleanup, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { createQueryClient } from "../app/queryClient";
import type { AudioModelStatus, Guesses, SoundsLike } from "../bindings";
import "../i18n";
import { useSelectedTrack } from "../shell/selectedTrack";
import { tableRow } from "../test/trackTableIpc";
import { tx } from "../test/tx";
import { AudioModelPanel } from "./AudioModelPanel";

let track = tableRow(1);

const STATUS: AudioModelStatus = { enabled: true, fingerprinted: 3, total: 40, failed: 0, running: true };
const READY_GUESSES: Guesses = {
  state: "ready",
  genres: [
    { key: "techno", percent: 84 },
    { key: "houseMusic", percent: 61 },
  ],
  mood: { key: "excitingMusic", percent: 52 },
};

let status: AudioModelStatus;
let guesses: Guesses;
let similar: SoundsLike;
let asked: string[];
/** What set_genre was asked, in order. */
let setGenre: unknown[];

function renderPanel() {
  const client = createQueryClient();
  render(
    <QueryClientProvider client={client}>
      <AudioModelPanel row={track} />
    </QueryClientProvider>,
  );
  return client;
}

beforeEach(() => {
  status = STATUS;
  guesses = READY_GUESSES;
  similar = { state: "ready", tracks: [] };
  asked = [];
  setGenre = [];
  track = tableRow(1);
  useSelectedTrack.setState({ row: null, ids: [] });
  mockIPC((cmd, args) => {
    asked.push(cmd);
    switch (cmd) {
      case "set_genre":
        setGenre.push(args);
        return 7;
      case "audio_model_status":
        return status;
      case "audio_model_guesses":
        return guesses;
      case "sounds_like":
        return similar;
      default:
        throw new Error(`unexpected command ${cmd}`);
    }
  });
});

afterEach(() => {
  cleanup();
  clearMocks();
});

/** Matches the accessible name of any genre guess's Use button. */
function useName(name: string): boolean {
  return [tx("audioModel:label.techno"), tx("audioModel:label.houseMusic")].some(
    (genre) => name === tx("audioModel:useGenre", { genre }),
  );
}

describe("AudioModelPanel", () => {
  it("is hidden while the switch is off, and asks for nothing but the switch", async () => {
    status = { ...STATUS, enabled: false };
    const client = renderPanel();
    // Wait for the switch's answer to arrive, so a panel wrongly shown would be on screen by now.
    await waitFor(() => expect(client.getQueryData(["audioModel", "status"])).toBeDefined());
    expect(screen.queryByRole("heading", { name: tx("audioModel:title") })).toBeNull();
    expect(asked).toEqual(["audio_model_status"]);
  });

  it("shows the model's genre and mood guesses with their percent", async () => {
    renderPanel();
    const panel = await screen.findByRole("region", { name: tx("audioModel:title") });
    const guess = (key: string, percent: number) =>
      tx("audioModel:guess", { label: tx(`audioModel:label.${key}`), percent });
    expect(await within(panel).findByText(guess("techno", 84))).toBeVisible();
    expect(within(panel).getByText(guess("houseMusic", 61))).toBeVisible();
    expect(within(panel).getByText(guess("excitingMusic", 52))).toBeVisible();
  });

  it("gives each genre guess of a Library track a Use button, and the mood none", async () => {
    renderPanel();
    const panel = await screen.findByRole("region", { name: tx("audioModel:title") });
    const uses = await within(panel).findAllByRole("button", { name: useName });
    expect(uses.map((button) => button.getAttribute("aria-label"))).toEqual([
      tx("audioModel:useGenre", { genre: tx("audioModel:label.techno") }),
      tx("audioModel:useGenre", { genre: tx("audioModel:label.houseMusic") }),
    ]);
    expect(uses[0]).toHaveTextContent(tx("audioModel:use"));
  });

  it("Use calls set_genre with the Library track and the guess's name as shown, once, and says what it did", async () => {
    renderPanel();
    const name = tx("audioModel:label.houseMusic");
    await userEvent.click(await screen.findByRole("button", { name: tx("audioModel:useGenre", { genre: name }) }));
    await waitFor(() => expect(setGenre).toEqual([{ libraryTrack: track.libraryTrackId, name }]));
    expect(await screen.findByText(tx("audioModel:used", { genre: name }))).toBeVisible();
  });

  it("never sets a genre by itself: nothing is asked of set_genre until Use is pressed", async () => {
    similar = { state: "ready", tracks: [{ row: tableRow(2), percent: 90 }] };
    renderPanel();
    const panel = await screen.findByRole("region", { name: tx("audioModel:title") });
    await within(panel).findByRole("button", { name: /Synthetic Tune 2/ });
    expect(asked).not.toContain("set_genre");
  });

  it("shows no Use for a track outside the Library", async () => {
    track = tableRow(1, { libraryTrackId: null, inLibrary: false });
    renderPanel();
    const panel = await screen.findByRole("region", { name: tx("audioModel:title") });
    await within(panel).findByText(tx("audioModel:guess", { label: tx("audioModel:label.techno"), percent: 84 }));
    expect(within(panel).queryAllByRole("button", { name: useName })).toHaveLength(0);
  });

  it("shows no Use for the guess that is already the track's genre", async () => {
    track = tableRow(1, { genre: tx("audioModel:label.techno") });
    renderPanel();
    const panel = await screen.findByRole("region", { name: tx("audioModel:title") });
    const uses = await within(panel).findAllByRole("button", { name: useName });
    expect(uses.map((button) => button.getAttribute("aria-label"))).toEqual([
      tx("audioModel:useGenre", { genre: tx("audioModel:label.houseMusic") }),
    ]);
  });

  it("says a track not listened to yet is waiting, with how far the listening has got", async () => {
    guesses = { state: "notListenedYet", genres: [], mood: null };
    renderPanel();
    const panel = await screen.findByRole("region", { name: tx("audioModel:title") });
    expect(await within(panel).findByText(tx("audioModel:state.notListened", { done: 3, count: 40 }))).toBeVisible();
    expect(asked).not.toContain("sounds_like");
  });

  it("says a track that could not be read could not be read", async () => {
    guesses = { state: "couldntBeRead", genres: [], mood: null };
    renderPanel();
    expect(await screen.findByText(tx("audioModel:state.couldntRead"))).toBeVisible();
    expect(asked).not.toContain("sounds_like");
  });

  it("says a track outside the Library is not listened to", async () => {
    guesses = { state: "notInLibrary", genres: [], mood: null };
    renderPanel();
    expect(await screen.findByText(tx("audioModel:state.notInLibrary"))).toBeVisible();
  });

  it("names the genre and the mood that the model has no guess for", async () => {
    guesses = { state: "ready", genres: [], mood: null };
    renderPanel();
    expect(await screen.findByText(tx("audioModel:noGenre"))).toBeVisible();
    expect(screen.getByText(tx("audioModel:noMood"))).toBeVisible();
  });

  it("says there are no similar tracks yet when the list is empty", async () => {
    renderPanel();
    expect(await screen.findByText(tx("audioModel:similar.empty"))).toBeVisible();
  });

  it("lists the tracks that sound alike, best first, each with its percent", async () => {
    similar = {
      state: "ready",
      tracks: [
        { row: tableRow(2), percent: 91 },
        { row: tableRow(3, { artist: null }), percent: 74 },
      ],
    };
    renderPanel();
    const list = await screen.findByRole("list", { name: tx("audioModel:similar.title") });
    const items = within(list).getAllByRole("listitem");
    expect(items).toHaveLength(2);
    expect(items[0]).toHaveTextContent(tx("audioModel:similar.track", { artist: "Made Up Artist 2", title: "Synthetic Tune 2" }));
    expect(items[0]).toHaveTextContent(tx("audioModel:similar.percent", { percent: 91 }));
    expect(items[1]).toHaveTextContent("Synthetic Tune 3");
  });

  it("selects a track when its row is clicked", async () => {
    const other = tableRow(2);
    similar = { state: "ready", tracks: [{ row: other, percent: 91 }] };
    renderPanel();
    await userEvent.click(await screen.findByRole("button", { name: /Synthetic Tune 2/ }));
    expect(useSelectedTrack.getState().row).toEqual(other);
  });
});
