import { QueryClientProvider } from "@tanstack/react-query";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { act, cleanup, render, screen, } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { createQueryClient } from "../app/queryClient";
import "../i18n";
import { quietAnswer } from "../test/quietCommands";
import { tableRow } from "../test/trackTableIpc";
import { tx } from "../test/tx";
import { DetailsPanel } from "./DetailsPanel";
import { useSelectedTrack } from "./selectedTrack";

/**
 * The real Details panel, as the app mounts it: the behavior a user sees when the
 * selection moves. (The panel is keyed per track twice, in DetailsPanel and in
 * TrackDetails, so this checks the outcome, not either key alone.)
 */
beforeEach(() => {
  mockIPC((cmd) => {
    switch (cmd) {
      case "track_tag_sources":
        return { inRekordbox: false, fields: [], confirmedEntry: null };
      case "audio_model_status":
        return { enabled: true, fingerprinted: 3, total: 40, failed: 0, running: false };
      case "audio_model_guesses":
        return { state: "ready", genres: [{ key: "houseMusic", percent: 61 }], mood: null };
      case "audio_model_your_genres":
        return { genres: [], why: "nothingStrongEnough" };
      case "sounds_like":
        return { state: "ready", tracks: [] };
      case "set_genre":
        return 7;
      default:
        return quietAnswer(cmd);
    }
  });
});

afterEach(() => {
  cleanup();
  clearMocks();
  useSelectedTrack.setState({ row: null, tables: 0 });
});

describe("the genre line in the Details panel", () => {
  it("says 'Genre set to' for the track Use was pressed on, and not for the next track selected", async () => {
    const genre = tx("audioModel:label.houseMusic");
    const use = () => screen.findByRole("button", { name: tx("audioModel:useGenre", { genre }) });
    useSelectedTrack.setState({ row: tableRow(2) });
    render(
      <QueryClientProvider client={createQueryClient()}>
        <DetailsPanel />
      </QueryClientProvider>,
    );
    // Track 2's guesses are loaded and kept, so showing it again waits for nothing.
    await use();
    act(() => useSelectedTrack.setState({ row: tableRow(1) }));
    await userEvent.click(await use());
    expect(await screen.findByText(tx("audioModel:used", { genre }))).toBeVisible();

    act(() => useSelectedTrack.setState({ row: tableRow(2) }));

    await use();
    expect(screen.queryByText(tx("audioModel:used", { genre }))).toBeNull();
  });
});
