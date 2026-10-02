import { cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { MusicFolder } from "../bindings";
import "../i18n";
import { MusicFolderWatchSwitch } from "./MusicFolderWatchSwitch";
import { tx } from "../test/tx";

function folder(watch: boolean): MusicFolder {
  return {
    id: 3,
    role: "scan",
    watch,
    path: String.raw`E:\Music`,
    online: true,
    volumeLabel: "",
    addedAt: "2026-09-29T10:00:00.000Z",
    walkedAt: null,
    unreadableFolders: null,
    unreadableFiles: null,
    onlineOnlyFiles: 0,
  };
}

afterEach(cleanup);

describe("the music folder watch switch", () => {
  it("shows whether the folder is watched", () => {
    render(<MusicFolderWatchSwitch folder={folder(true)} onChange={() => {}} />);
    expect(screen.getByRole("checkbox", { name: tx("musicFolderWatch:watch") })).toBeChecked();
  });

  it("hands the new choice to the caller", async () => {
    const onChange = vi.fn();
    render(<MusicFolderWatchSwitch folder={folder(false)} onChange={onChange} />);
    await userEvent.click(screen.getByRole("checkbox", { name: tx("musicFolderWatch:watch") }));
    expect(onChange).toHaveBeenCalledWith(true);
  });
});
