import { cleanup, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";
import "../i18n";
import { tx } from "../test/tx";
import { ColumnEditor } from "./ColumnEditor";
import { DEFAULT_COLUMNS, type ColumnId, type TableScreen } from "./columns";

afterEach(cleanup);

const name = (id: string) => tx(`trackTable:columns.${id}`);

/** The editor with state of its own, as the screen holds it. */
function Editor({
  screen = "library",
  start = DEFAULT_COLUMNS.library,
  onReset = () => {},
}: {
  screen?: TableScreen;
  start?: readonly ColumnId[];
  onReset?: () => void;
}) {
  const [columns, setColumns] = useState(start);
  return (
    <>
      <output data-testid="columns">{columns.join(",")}</output>
      <ColumnEditor
        screen={screen}
        columns={columns}
        onChange={setColumns}
        onReset={() => {
          onReset();
          setColumns(DEFAULT_COLUMNS[screen]);
        }}
      />
    </>
  );
}

const shownNow = () => screen.getByTestId("columns").textContent;
const open = () => userEvent.click(screen.getByRole("button", { name: tx("trackTable:editor.open") }));
const panel = () => screen.getByRole("dialog", { name: tx("trackTable:editor.title") });
const button = (key: string, column: string) =>
  within(panel()).getByRole("button", { name: tx(`trackTable:editor.${key}`, { column: name(column) }) });

describe("the column editor", () => {
  it("is closed until the Columns button opens it, and says it has a panel", async () => {
    render(<Editor />);
    const opener = screen.getByRole("button", { name: tx("trackTable:editor.open") });
    expect(opener).toHaveAttribute("aria-expanded", "false");
    expect(screen.queryByRole("dialog")).toBeNull();
    await userEvent.click(opener);
    expect(opener).toHaveAttribute("aria-expanded", "true");
    expect(panel()).toHaveFocus();
  });

  it("lists the shown columns in order and the hidden ones apart", async () => {
    render(<Editor />);
    await open();
    const shown = within(within(panel()).getByRole("list", { name: tx("trackTable:editor.shown") })).getAllByRole(
      "listitem",
    );
    expect(shown.map((li) => li.getAttribute("data-column"))).toEqual(DEFAULT_COLUMNS.library);
    const hidden = within(within(panel()).getByRole("list", { name: tx("trackTable:editor.hidden") })).getAllByRole(
      "listitem",
    );
    expect(hidden.map((li) => li.getAttribute("data-column"))).toEqual(["file", "folder"]);
  });

  it("moves a column up and down with buttons, and keeps the focus on the button used", async () => {
    render(<Editor />);
    await open();
    await userEvent.click(button("moveUp", "bpm"));
    expect(shownNow()).toBe("title,bpm,artist,key,time,genre,rating,dateAdded");
    expect(button("moveUp", "bpm")).toHaveFocus();
    await userEvent.click(button("moveDown", "bpm"));
    await userEvent.click(button("moveDown", "bpm"));
    expect(shownNow()).toBe("title,artist,key,bpm,time,genre,rating,dateAdded");
    expect(button("moveDown", "bpm")).toHaveFocus();
  });

  it("moves a column by keyboard alone", async () => {
    render(<Editor />);
    await open();
    button("moveDown", "title").focus();
    await userEvent.keyboard("{Enter}");
    expect(shownNow()).toBe("artist,title,bpm,key,time,genre,rating,dateAdded");
    await userEvent.keyboard(" ");
    expect(shownNow()).toBe("artist,bpm,title,key,time,genre,rating,dateAdded");
  });

  it("greys out Move up on the first column and Move down on the last, and moves the focus to the other arrow at an end", async () => {
    render(<Editor start={["title", "artist"]} />);
    await open();
    expect(button("moveUp", "title")).toBeDisabled();
    expect(button("moveDown", "artist")).toBeDisabled();
    await userEvent.click(button("moveUp", "artist"));
    expect(shownNow()).toBe("artist,title");
    // Artist is first now, so Move up is greyed out and the focus went to Move down.
    expect(button("moveUp", "artist")).toBeDisabled();
    expect(button("moveDown", "artist")).toHaveFocus();
  });

  it("hides a column and shows it again at the end, with the focus following it", async () => {
    render(<Editor />);
    await open();
    await userEvent.click(button("hide", "genre"));
    expect(shownNow()).toBe("title,artist,bpm,key,time,rating,dateAdded");
    expect(button("show", "genre")).toHaveFocus();
    await userEvent.click(button("show", "genre"));
    expect(shownNow()).toBe("title,artist,bpm,key,time,rating,dateAdded,genre");
    expect(button("hide", "genre")).toHaveFocus();
  });

  it("shows a column that was never shown", async () => {
    render(<Editor />);
    await open();
    await userEvent.click(button("show", "file"));
    expect(shownNow()).toBe(`${DEFAULT_COLUMNS.library.join(",")},file`);
  });

  it("cannot hide the title, and says it is always shown", async () => {
    render(<Editor />);
    await open();
    expect(button("hide", "title")).toBeDisabled();
    expect(within(panel()).getByText(tx("trackTable:editor.alwaysShown"))).toBeInTheDocument();
    expect(button("hide", "artist")).toBeEnabled();
  });

  it("says when no column is hidden", async () => {
    render(
      <Editor start={["title", "artist", "bpm", "key", "time", "genre", "rating", "dateAdded", "file", "folder"]} />,
    );
    await open();
    expect(within(panel()).getByText(tx("trackTable:editor.noneHidden"))).toBeInTheDocument();
  });

  it("resets to the default, which is greyed out while the columns already are the default", async () => {
    const onReset = vi.fn();
    render(<Editor start={["title", "file"]} screen="allMusic" onReset={onReset} />);
    await open();
    const reset = within(panel()).getByRole("button", { name: tx("trackTable:editor.reset") });
    expect(reset).toBeEnabled();
    await userEvent.click(reset);
    expect(onReset).toHaveBeenCalledTimes(1);
    expect(shownNow()).toBe(DEFAULT_COLUMNS.allMusic.join(","));
    expect(within(panel()).getByRole("button", { name: tx("trackTable:editor.reset") })).toBeDisabled();
  });

  it("closes with Done, giving the focus back to the Columns button", async () => {
    render(<Editor />);
    await open();
    await userEvent.click(within(panel()).getByRole("button", { name: tx("trackTable:editor.done") }));
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(screen.getByRole("button", { name: tx("trackTable:editor.open") })).toHaveFocus();
  });

  it("closes with Escape, and keeps the columns chosen so far", async () => {
    render(<Editor />);
    await open();
    await userEvent.click(button("hide", "rating"));
    await userEvent.keyboard("{Escape}");
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(screen.getByRole("button", { name: tx("trackTable:editor.open") })).toHaveFocus();
    expect(shownNow()).not.toContain("rating");
  });

  it("closes when the user clicks somewhere else", async () => {
    render(
      <>
        <Editor />
        <p>Elsewhere</p>
      </>,
    );
    await open();
    await userEvent.click(screen.getByText("Elsewhere"));
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("tells a crate that its choice is shared by every crate", async () => {
    render(<Editor screen="crate" />);
    await open();
    expect(within(panel()).getByText(tx("trackTable:editor.helpCrate"))).toBeInTheDocument();
    expect(within(panel()).queryByText(tx("trackTable:editor.help"))).toBeNull();
  });
});
