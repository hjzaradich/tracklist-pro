import { QueryClientProvider } from "@tanstack/react-query";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { cleanup, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { createQueryClient } from "../app/queryClient";
import type { MusicFolder } from "../bindings";
import "../i18n";
import { tx } from "../test/tx";
import { MusicFolderList } from "./MusicFolderList";
import { useOnedriveFolders } from "./useOnedriveFolders";

function folder(id: number, path: string): MusicFolder {
  return {
    id,
    role: "scan",
    watch: false,
    path,
    online: true,
    volumeLabel: "",
    addedAt: "2026-10-01T10:00:00.000Z",
    walkedAt: "2026-10-01T10:00:00.000Z",
    unreadableFolders: null,
    unreadableFiles: null,
    onlineOnlyFiles: 0,
  };
}

const FOLDERS = [folder(1, String.raw`C:\Users\dj\OneDrive\Music`), folder(2, String.raw`D:\Music`)];

function List() {
  return <MusicFolderList folders={FOLDERS} onedrive={useOnedriveFolders(FOLDERS)} />;
}

let asked = 0;

function renderList(inOnedrive: number[]) {
  asked = 0;
  mockIPC((cmd) => {
    if (cmd === "onedrive_music_folders") {
      asked += 1;
      return inOnedrive;
    }
    throw new Error(`unexpected command ${cmd}`);
  });
  return render(
    <QueryClientProvider client={createQueryClient()}>
      <List />
    </QueryClientProvider>,
  );
}

afterEach(() => {
  cleanup();
  clearMocks();
});

describe("the OneDrive note", () => {
  it("shows beside a folder in OneDrive, with what to do, and not beside another", async () => {
    const { container } = renderList([1]);
    await waitFor(() => expect(container.querySelectorAll('[data-note="onedrive"]')).toHaveLength(1));
    const note = container.querySelector('[data-note="onedrive"]');
    expect(note).toHaveTextContent(tx("musicFolderStatus:onedrive"));
    expect(note).toHaveTextContent(tx("musicFolderStatus:onedriveHelp"));
    expect(note?.closest("li")).toHaveTextContent(String.raw`OneDrive\Music`);
    expect(screen.getByText(String.raw`D:\Music`).closest("li")?.querySelector("[data-note=onedrive]")).toBeNull();
  });

  it("is hidden when no folder is in OneDrive", async () => {
    const { container } = renderList([]);
    await screen.findByText(String.raw`D:\Music`);
    await waitFor(() => expect(asked).toBeGreaterThan(0));
    expect(container.querySelector('[data-note="onedrive"]')).toBeNull();
  });
});
