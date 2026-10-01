import { QueryClientProvider } from "@tanstack/react-query";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { cleanup, render, screen, within } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { createQueryClient } from "../app/queryClient";
import type { LibraryTrack } from "../bindings";
import "../i18n";
import { LibraryScreen } from "./LibraryScreen";

function track(id: number, fields: Partial<LibraryTrack> = {}): LibraryTrack {
  return {
    id,
    recordingId: id + 100,
    kind: "linked",
    title: `Synthetic Tune ${id}`,
    artist: `Made Up Artist ${id}`,
    file: { path: String.raw`E:\Music\tune ` + id + ".mp3", name: `tune ${id}.mp3`, present: true },
    fragile: null,
    addedAt: "2026-09-30T10:00:00.000Z",
    ...fields,
  };
}

/** Stands in for the Rust side: the Library holds `tracks`, or can't be read. */
function library(tracks: LibraryTrack[] | "broken") {
  mockIPC((cmd) => {
    if (cmd !== "library_tracks") throw new Error(`unexpected command ${cmd}`);
    if (tracks === "broken") throw { kind: "database", params: {} };
    return tracks;
  });
}

function renderScreen() {
  return render(
    <QueryClientProvider client={createQueryClient()}>
      <LibraryScreen />
    </QueryClientProvider>,
  );
}

/** The list's body rows, once loaded. */
async function rows() {
  const table = await screen.findByRole("table");
  return within(table).getAllByRole("row").slice(1);
}

afterEach(() => {
  cleanup();
  clearMocks();
});

describe("the Library screen", () => {
  it("names what's absent when there are no Library tracks", async () => {
    library([]);
    renderScreen();
    expect(await screen.findByText("No Library tracks")).toBeInTheDocument();
    expect(screen.queryByRole("table")).toBeNull();
  });

  it("lists each Library track's title, artist and file, in the order the backend gave", async () => {
    library([track(2), track(1)]);
    renderScreen();
    const [first, second] = await rows();
    expect(
      screen.getAllByRole("columnheader").map((header) => header.textContent),
    ).toEqual(["Title", "Artist", "File"]);
    expect(within(first).getAllByRole("cell").map((cell) => cell.textContent)).toEqual([
      "Synthetic Tune 2",
      "Made Up Artist 2",
      String.raw`E:\Music\tune 2.mp3`,
    ]);
    expect(second).toHaveTextContent("Synthetic Tune 1");
    expect(screen.queryByText("No Library tracks")).toBeNull();
    // Nothing to say about a track whose file is there.
    expect(first).not.toHaveAttribute("aria-describedby");
  });

  it("shows the file's name as the title of a track that has none", async () => {
    library([track(1, { title: null, artist: null })]);
    renderScreen();
    const [row] = await rows();
    expect(within(row).getAllByRole("cell").map((cell) => cell.textContent)).toEqual([
      "tune 1.mp3",
      "",
      String.raw`E:\Music\tune 1.mp3`,
    ]);
  });

  it("says so next to a track whose file is missing", async () => {
    const gone = track(2, {
      file: { path: String.raw`E:\Music\gone.mp3`, name: "gone.mp3", present: false },
    });
    library([track(1), gone]);
    renderScreen();
    const [here, missing] = await rows();
    expect(missing).toHaveAccessibleDescription("File missing");
    expect(missing).toHaveTextContent(String.raw`E:\Music\gone.mp3`);
    expect(missing).toHaveAttribute("data-file-present", "false");
    expect(here).not.toHaveTextContent("File missing");
    expect(here).toHaveAttribute("data-file-present", "true");
  });

  it.each([
    ["downloads", "In Downloads: will be lost if Downloads is cleared"],
    ["temp", "In a temp folder: Windows may delete it"],
    ["external", "On an external drive: will be lost when the drive is unplugged"],
    ["network", "On a network drive: will be lost when the drive is offline"],
  ] as const)("says why a file in a %s location is fragile", async (reason, text) => {
    library([track(1, { fragile: reason })]);
    renderScreen();
    const [row] = await rows();
    expect(row).toHaveAccessibleDescription(text);
  });

  it("shows no note for a track whose file isn't in a fragile location", async () => {
    library([track(1)]);
    renderScreen();
    const [row] = await rows();
    expect(row).not.toHaveAttribute("aria-describedby");
  });

  it("shows both notes when the file is missing and was somewhere fragile", async () => {
    library([
      track(1, {
        fragile: "external",
        file: { path: String.raw`E:\Music\gone.mp3`, name: "gone.mp3", present: false },
      }),
    ]);
    renderScreen();
    const [row] = await rows();
    expect(row).toHaveTextContent("File missing");
    expect(row).toHaveTextContent("On an external drive: will be lost when the drive is unplugged");
  });

  it("shows an error message, not an empty Library, when the list can't be loaded", async () => {
    library("broken");
    renderScreen();
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Couldn't read or save the Library. Try again.",
    );
    expect(screen.queryByText("No Library tracks")).toBeNull();
  });
});
