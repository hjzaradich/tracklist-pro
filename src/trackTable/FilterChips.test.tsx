import { QueryClientProvider } from "@tanstack/react-query";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createQueryClient } from "../app/queryClient";
import type { TableFilter, TagGroup } from "../bindings";
import "../i18n";
import { tx } from "../test/tx";
import { FilterChips } from "./FilterChips";

// The tag filter's chip (3C-F). Every name here is made up.

const MOOD: TagGroup = {
  id: 1,
  name: "Mood",
  colour: null,
  tags: [
    { id: 10, groupId: 1, name: "Sunny", colour: null, rekordboxName: null, goneFromRekordbox: false, trackCount: 2 },
  ],
};

/** The commands the fake Rust side was asked, in order. */
let asked: string[];

beforeEach(() => {
  asked = [];
  mockIPC((cmd) => {
    asked.push(cmd);
    if (cmd === "list_tags") return [MOOD];
    if (cmd === "track_table_formats") return [];
    throw new Error(`unexpected ${cmd}`);
  });
});

afterEach(() => {
  cleanup();
  clearMocks();
});

function show(filters: TableFilter[], onChange = vi.fn()) {
  render(
    <QueryClientProvider client={createQueryClient()}>
      <FilterChips
        screen="library"
        source={{ kind: "library" }}
        keyPrefix={["test"]}
        filters={filters}
        onChange={onChange}
      />
    </QueryClientProvider>,
  );
  return onChange;
}

describe("FilterChips tag chip", () => {
  it("names the tag it filters by and takes the filter off with its ×", async () => {
    const user = userEvent.setup();
    const onChange = show([{ kind: "tag", id: 10 }, { kind: "missingArt" }]);
    expect(await screen.findByText(tx("trackTable:filters.tagNamed", { tag: "Sunny" }))).toBeTruthy();
    await user.click(
      screen.getByRole("button", { name: tx("trackTable:filters.remove", { filter: tx("trackTable:filters.tagNamed", { tag: "Sunny" }) }) }),
    );
    expect(onChange).toHaveBeenCalledWith([{ kind: "missingArt" }]);
  });

  it("says the tag is removed when the tag no longer exists", async () => {
    show([{ kind: "tag", id: 99 }]);
    expect(await screen.findByText(tx("trackTable:filters.tagGone"))).toBeTruthy();
  });

  it("reads the tags only when a tag filter is on", async () => {
    show([{ kind: "missingArt" }]);
    await screen.findByText(tx("trackTable:filters.missingArt"));
    expect(asked).not.toContain("list_tags");
  });
});
