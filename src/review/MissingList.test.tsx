import { cleanup, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { MissingGroup, MissingList as Missing } from "../bindings";
import "../i18n";
import { MissingList } from "./MissingList";
import { tx } from "../test/tx";

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

/** The line with a folder's name, its count and its button (the summary line also counts). */
function folderRow(folder: string): HTMLElement {
  return screen.getByText(folder).parentElement as HTMLElement;
}

describe("the Missing list", () => {
  it("names what's absent when no track is missing", () => {
    render(<MissingList list={list([])} />);
    expect(screen.getByText(tx("review:missing.empty"))).toBeInTheDocument();
  });

  it("says what's still being worked out instead of claiming no track is missing", () => {
    render(<MissingList list={list([])} waiting={tx("activity:task.relink")} />);
    expect(screen.getByText(tx("activity:task.relink"))).toBeInTheDocument();
    expect(screen.queryByText(tx("review:missing.empty"))).toBeNull();
  });

  it("shows each folder with its track count and the tracks' last known paths", () => {
    render(<MissingList list={list([group(String.raw`D:\Old`)])} />);
    expect(screen.getByText(String.raw`D:\Old`)).toBeInTheDocument();
    expect(within(folderRow(String.raw`D:\Old`)).getByText(tx("review:missing.tracksCount", { count: 1 }))).toBeInTheDocument();
    expect(screen.getByText(tx("review:missing.trackName", { artist: "Artist", title: "Song" }))).toBeInTheDocument();
    expect(screen.getByText(String.raw`D:\Old\song.mp3`)).toBeInTheDocument();
  });

  it("calls a group with no known folder the unknown folder", () => {
    render(<MissingList list={list([group(null)])} />);
    expect(screen.getByText(tx("review:missing.unknownFolder"))).toBeInTheDocument();
  });

  it("offers to add only a folder the list says can be added", async () => {
    const onAddFolder = vi.fn();
    render(
      <MissingList
        list={list([group(String.raw`D:\Here`, { canAdd: true }), group(String.raw`D:\Gone`)])}
        onAddFolder={onAddFolder}
      />,
    );
    const buttons = screen.getAllByRole("button", { name: tx("review:missing.addFolder") });
    expect(buttons).toHaveLength(1);
    await userEvent.click(buttons[0]);
    expect(onAddFolder).toHaveBeenCalledWith(String.raw`D:\Here`);
  });

  it("counts several tracks in a folder in the plural", () => {
    const many = group(String.raw`D:\Old`, {
      tracks: [1, 2, 3].map((id) => ({
        id,
        title: `Song ${id}`,
        artist: "",
        lastKnownPath: null,
      })),
    });
    render(<MissingList list={list([many])} />);
    expect(within(folderRow(String.raw`D:\Old`)).getByText(tx("review:missing.tracksCount", { count: 3 }))).toBeInTheDocument();
  });

  it("shows only the heading while the list loads", () => {
    render(<MissingList list={undefined} />);
    expect(screen.getByRole("heading", { name: tx("review:missing.title") })).toBeInTheDocument();
    expect(screen.queryByText(tx("review:missing.empty"))).toBeNull();
  });

  it("says so when the list or an add failed, instead of looking empty", () => {
    render(<MissingList list={undefined} problem={tx("errors:internal")} />);
    expect(screen.getByRole("alert")).toHaveTextContent(tx("errors:internal"));
  });

  it("holds the add buttons while an add runs", () => {
    render(
      <MissingList
        list={list([group(String.raw`D:\Here`, { canAdd: true })])}
        adding
        onAddFolder={vi.fn()}
      />,
    );
    expect(screen.getByRole("button", { name: tx("review:missing.addFolder") })).toBeDisabled();
  });

  it("is one line, the heading and how many tracks, until it is opened", () => {
    const { container } = render(<MissingList list={list([group(String.raw`D:\Old`), group(String.raw`D:\Older`)])} />);
    const details = container.querySelector("details") as HTMLDetailsElement;
    expect(details.open).toBe(false);
    const summary = details.querySelector("summary") as HTMLElement;
    expect(within(summary).getByRole("heading", { name: tx("review:missing.title") })).toBeInTheDocument();
    expect(within(summary).getByText(tx("review:missing.tracksCount", { count: 2 }))).toBeInTheDocument();
  });

  it("is open when nothing is missing, and when something went wrong", () => {
    const { container, rerender } = render(<MissingList list={list([])} />);
    expect((container.querySelector("details") as HTMLDetailsElement).open).toBe(true);
    rerender(<MissingList list={list([group(String.raw`D:\Old`)])} problem="It failed" />);
    expect((container.querySelector("details") as HTMLDetailsElement).open).toBe(true);
    expect(screen.getByRole("alert")).toHaveTextContent("It failed");
  });
});
