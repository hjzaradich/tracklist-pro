import { cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { MissingGroup, MissingList as Missing } from "../bindings";
import "../i18n";
import { MissingList } from "./MissingList";

function group(folder: string | null, fields: Partial<MissingGroup> = {}): MissingGroup {
  return {
    folder,
    canAdd: false,
    tracks: [
      {
        id: 1,
        title: "Song",
        artist: "Artist",
        lastKnownPath: folder === null ? null : String.raw`${folder}\song.mp3`,
      },
    ],
    ...fields,
  };
}

function list(groups: MissingGroup[]): Missing {
  return { total: groups.reduce((n, g) => n + g.tracks.length, 0), groups };
}

afterEach(cleanup);

describe("the Missing list", () => {
  it("names what's absent when no track is missing", () => {
    render(<MissingList list={list([])} />);
    expect(screen.getByText("No missing tracks")).toBeInTheDocument();
  });

  it("shows each folder with its track count and the tracks' last known paths", () => {
    render(<MissingList list={list([group(String.raw`D:\Old`)])} />);
    expect(screen.getByText(String.raw`D:\Old`)).toBeInTheDocument();
    expect(screen.getByText("(1 track)")).toBeInTheDocument();
    expect(screen.getByText("Artist - Song")).toBeInTheDocument();
    expect(screen.getByText(String.raw`D:\Old\song.mp3`)).toBeInTheDocument();
  });

  it("calls a group with no known folder the unknown folder", () => {
    render(<MissingList list={list([group(null)])} />);
    expect(screen.getByText("Unknown folder")).toBeInTheDocument();
  });

  it("offers to add only a folder the list says can be added", async () => {
    const onAddFolder = vi.fn();
    render(
      <MissingList
        list={list([group(String.raw`D:\Here`, { canAdd: true }), group(String.raw`D:\Gone`)])}
        onAddFolder={onAddFolder}
      />,
    );
    const buttons = screen.getAllByRole("button", { name: "Add folder" });
    expect(buttons).toHaveLength(1);
    await userEvent.click(buttons[0]);
    expect(onAddFolder).toHaveBeenCalledWith(String.raw`D:\Here`);
  });
});
