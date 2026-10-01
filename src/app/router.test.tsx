import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { STAGES } from "../shell/stages";
import { renderApp } from "./testApp";

// Each stage's name and what its screen says with an empty Library, in English.
const SCREENS = [
  { path: "/overview", name: "Overview", empty: "Start your Library" },
  { path: "/review", name: "Review", empty: "Every decision the app can't make on its own." },
  { path: "/all-music", name: "All music", empty: "No tracks in All music" },
  { path: "/library", name: "Library", empty: "No Library tracks" },
  { path: "/crates", name: "Crates", empty: "Groups of Library tracks, collected for a purpose." },
];

describe("stage routes", () => {
  // The screens ask the backend for their tracks: there are none.
  beforeEach(() => {
    mockIPC((cmd) => {
      if (cmd === "library_tracks") return [];
      if (cmd === "all_music_tracks") return { total: 0, tracks: [] };
      if (cmd === "music_folders") return [];
      if (cmd === "rekordbox_offer") {
        return { toAdd: 0, alreadyInLibrary: 0, waitingInMissing: 0, waitingForConfirmation: 0 };
      }
      throw new Error(`unexpected command ${cmd}`);
    });
  });
  afterEach(() => clearMocks());

  it("cover exactly the stages the sidebar lists", () => {
    expect(SCREENS.map((s) => s.path)).toEqual(STAGES.map((s) => s.path));
  });

  it.each(SCREENS)("$path renders the $name screen", async ({ path, name, empty }) => {
    renderApp(path);
    expect(await screen.findByRole("heading", { level: 1, name })).toBeInTheDocument();
    await waitFor(() => expect(screen.getByRole("main")).toHaveTextContent(empty));
  });

  it("opens on Overview at /", async () => {
    const { router } = renderApp("/");
    expect(await screen.findByRole("heading", { level: 1, name: "Overview" })).toBeInTheDocument();
    await waitFor(() => expect(router.state.location.pathname).toBe("/overview"));
  });
});
