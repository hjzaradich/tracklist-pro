import { cleanup, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { MusicFolder } from "../bindings";
import "../i18n";
import { MusicFolderList } from "./MusicFolderList";
import { tx } from "../test/tx";

function folder(id: number, fields: Partial<MusicFolder> = {}): MusicFolder {
  return {
    id,
    role: "scan",
    watch: false,
    path: String.raw`E:\Music ` + id,
    online: true,
    folderFound: true,
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
    const gig = folder(2, { online: false, folderFound: false, volumeLabel: "GIG USB", path: String.raw`F:\Gig` });
    render(<MusicFolderList folders={[folder(1), gig]} />);
    const rows = screen.getAllByRole("listitem");
    expect(rows[0]).toHaveAttribute("data-online", "true");
    expect(rows[1]).toHaveAttribute("data-online", "false");
    expect(rows[1]).toHaveTextContent(String.raw`F:\Gig`);
    expect(rows[1]).toHaveAccessibleDescription(
      tx("musicFolderStatus:offlineNamed", { label: "GIG USB" }),
    );
    // Nothing to say about a connected folder the scan read in full.
    expect(rows[0]).not.toHaveAttribute("aria-describedby");
  });

  it("offers Find it for a folder missing from a connected drive, and not for one that is there", async () => {
    const gone = folder(2, { folderFound: false, path: String.raw`E:\Moved away` });
    const onFindIt = vi.fn();
    render(<MusicFolderList folders={[folder(1), gone]} onFindIt={onFindIt} onScanAgain={vi.fn()} />);
    const [present, missing] = screen.getAllByRole("listitem");
    expect(within(present!).queryByRole("button", { name: tx("musicFolderStatus:findIt") })).toBeNull();
    const findIt = within(missing!).getByRole("button", { name: tx("musicFolderStatus:findIt") });
    // The drive is connected, so this is not the "drive not connected" note.
    expect(missing).toHaveTextContent(tx("musicFolderStatus:folderNotFound"));
    expect(missing).not.toHaveTextContent(tx("musicFolderStatus:offline"));
    // Scanning a folder that isn't there has nothing to do.
    expect(within(missing!).getByRole("button", { name: tx("musicFolderStatus:scanAgain") })).toBeDisabled();
    expect(within(present!).getByRole("button", { name: tx("musicFolderStatus:scanAgain") })).toBeEnabled();
    await userEvent.click(findIt);
    expect(onFindIt).toHaveBeenCalledExactlyOnceWith(2);
  });

  it("still offers Find it for a folder whose drive isn't connected", () => {
    const away = folder(3, { online: false, folderFound: false });
    render(<MusicFolderList folders={[away]} onFindIt={vi.fn()} />);
    expect(screen.getByRole("button", { name: tx("musicFolderStatus:findIt") })).toBeVisible();
  });

  it("shows what the last scan couldn't read next to the folder", () => {
    render(<MusicFolderList folders={[folder(1, { unreadableFolders: 2, onlineOnlyFiles: 5 })]} />);
    const row = screen.getByRole("listitem");
    expect(row).toHaveTextContent(tx("musicFolderStatus:unreadableFolders", { count: 2 }));
    expect(row).toHaveTextContent(tx("musicFolderStatus:onlineOnly", { count: 5 }));
  });

  it("names what's absent when there are no music folders", () => {
    render(<MusicFolderList folders={[]} />);
    expect(screen.getByText(tx("musicFolderStatus:empty"))).toBeInTheDocument();
    expect(screen.queryByRole("list")).toBeNull();
  });
});
