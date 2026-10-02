import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { STAGES } from "../shell/stages";
import { renderApp } from "./testApp";
import { tx } from "../test/tx";

// Each stage's name and what its screen says with an empty Library.
const SCREENS = [
  { path: "/overview", name: tx("shell:stages.overview"), empty: tx("firstRun:title") },
  { path: "/review", name: tx("shell:stages.review"), empty: tx("review:empty") },
  { path: "/all-music", name: tx("shell:stages.allMusic"), empty: tx("allMusic:empty") },
  { path: "/library", name: tx("shell:stages.library"), empty: tx("library:empty") },
  { path: "/crates", name: tx("shell:stages.crates"), empty: tx("crates:empty") },
];

describe("stage routes", () => {
  // The screens ask the backend for their tracks: there are none.
  beforeEach(() => {
    mockIPC((cmd) => {
      if (cmd === "library_tracks") return [];
      if (cmd === "list_crates") return [];
      if (cmd === "all_music_tracks") return { total: 0, tracks: [] };
      if (cmd === "music_folders") return [];
      if (cmd === "rekordbox_xml_source") {
        return {
          path: null,
          watch: false,
          lastRead: null,
          lastFailure: null,
          newerExport: null,
          exportFolder: null,
        };
      }
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
    expect(await screen.findByRole("heading", { level: 1, name: tx("shell:stages.overview") })).toBeInTheDocument();
    await waitFor(() => expect(router.state.location.pathname).toBe("/overview"));
  });
});
