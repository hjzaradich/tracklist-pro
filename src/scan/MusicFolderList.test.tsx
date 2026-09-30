import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import type { MusicFolder } from "../bindings";
import "../i18n";
import { MusicFolderList } from "./MusicFolderList";

function folder(id: number, fields: Partial<MusicFolder> = {}): MusicFolder {
  return {
    id,
    role: "scan",
    watch: false,
    path: String.raw`E:\Music ` + id,
    online: true,
    volumeLabel: "",
    addedAt: "2026-09-29T10:00:00.000Z",
    walkedAt: "2026-09-29T10:05:00.000Z",
    unreadableFolders: 0,
    unreadableFiles: 0,
    onlineOnlyFiles: 0,
    ...fields,
  };
}

afterEach(cleanup);

describe("the music folder list", () => {
  it("greys out a folder whose drive isn't connected and says so", () => {
    const gig = folder(2, { online: false, volumeLabel: "GIG USB", path: String.raw`F:\Gig` });
    render(<MusicFolderList folders={[folder(1), gig]} />);
    const rows = screen.getAllByRole("listitem");
    expect(rows[0]).toHaveAttribute("data-online", "true");
    expect(rows[1]).toHaveAttribute("data-online", "false");
    expect(rows[1]).toHaveTextContent(String.raw`F:\Gig`);
    expect(rows[1]).toHaveAccessibleDescription("Drive not connected (GIG USB)");
    // Nothing to say about a connected folder the scan read in full.
    expect(rows[0]).not.toHaveAttribute("aria-describedby");
  });

  it("shows what the last scan couldn't read next to the folder", () => {
    render(<MusicFolderList folders={[folder(1, { unreadableFolders: 2, onlineOnlyFiles: 5 })]} />);
    const row = screen.getByRole("listitem");
    expect(row).toHaveTextContent("2 folders couldn't be read");
    expect(row).toHaveTextContent("5 files online only (not downloaded)");
  });

  it("names what's absent when there are no music folders", () => {
    render(<MusicFolderList folders={[]} />);
    expect(screen.getByText("No music folders")).toBeInTheDocument();
    expect(screen.queryByRole("list")).toBeNull();
  });
});
