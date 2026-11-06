import { QueryClientProvider } from "@tanstack/react-query";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { cleanup, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { createQueryClient } from "../app/queryClient";
import type { HistoryOutcome, Play, SessionSummary, WishlistOutcome } from "../bindings";
import "../i18n";
import { quietAnswer } from "../test/quietCommands";
import { tx } from "../test/tx";
import { HistoryScreen } from "./HistoryScreen";

const dialog = vi.hoisted(() => ({ open: vi.fn() }));
vi.mock("@tauri-apps/plugin-dialog", () => dialog);

afterEach(() => {
  cleanup();
  clearMocks();
  dialog.open.mockReset();
});

const SESSION: SessionSummary = {
  id: 4,
  source: "m3u8",
  name: "Set Name",
  playedOn: "2026-05-03",
  importedAt: "2026-05-04T10:00:00.000Z",
  plays: 2,
  matched: 1,
};

const PLAYS: Play[] = [
  { id: 1, position: 0, recordingId: 9, artist: "Artist A", title: "Title A", path: "E:/Music/a.mp3" },
  { id: 2, position: 1, recordingId: null, artist: "Artist B", title: "Title B", path: "E:/x/b.mp3" },
];

/** A fake Rust side for History; `importAnswer` is what an import comes to. */
function backend(options: { importAnswer?: HistoryOutcome; wishlistOffer?: boolean } = {}) {
  const calls: { cmd: string; args: unknown }[] = [];
  let sessions: SessionSummary[] = [];
  mockIPC((cmd, args) => {
    calls.push({ cmd, args });
    switch (cmd) {
      case "history_sessions":
        return sessions;
      case "history_plays":
        return { plays: PLAYS, wishlistOffer: options.wishlistOffer ?? false };
      case "history_import_m3u8": {
        const answer = options.importAnswer ?? {
          outcome: "done",
          operationId: 1,
          sessionId: SESSION.id,
          plays: 2,
          matched: 1,
          skippedLines: 1,
        };
        if (answer.outcome === "done") sessions = [SESSION];
        return answer;
      }
      case "history_set_date":
        return { outcome: "refused", reason: "badDate", sessionId: null, skippedLines: 0 };
      case "wishlists_list":
        return { outcome: "lists", lists: [{ id: 7, name: "Wants", source: "paste", total: 0, owned: 0, ownedOtherCut: 0, rework: 0, missing: 0, bought: 0 }] };
      case "history_add_to_wishlist": {
        const added: WishlistOutcome = {
          outcome: "done",
          operationId: 2,
          wishlistId: 7,
          added: 1,
          skipped: [],
          skippedTotal: 0,
        };
        return added;
      }
      default:
        return quietAnswer(cmd);
    }
  });
  return { called: (cmd: string) => calls.filter((c) => c.cmd === cmd) };
}

function renderScreen() {
  return render(
    <QueryClientProvider client={createQueryClient()}>
      <HistoryScreen />
    </QueryClientProvider>,
  );
}

describe("the History screen", () => {
  it("imports a picked m3u8 file as a set and shows its plays, marking the ones you don't have", async () => {
    const fake = backend();
    dialog.open.mockResolvedValue(String.raw`C:\dj\2026-05-03.m3u8`);
    renderScreen();
    const user = userEvent.setup();
    expect(await screen.findByText(tx("history:sessions.none"))).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: tx("history:import.choose") }));

    expect(await screen.findByText(tx("history:import.done", { count: 2, matched: 1 }))).toBeInTheDocument();
    expect(screen.getByText(tx("history:import.skippedLines", { count: 1 }))).toBeInTheDocument();
    expect(fake.called("history_import_m3u8")[0].args).toEqual({ path: String.raw`C:\dj\2026-05-03.m3u8` });

    const plays = await screen.findByRole("region", { name: tx("history:plays.label") });
    const rows = await within(plays).findAllByRole("listitem");
    expect(rows).toHaveLength(2);
    expect(rows[0]).toHaveTextContent(tx("history:plays.track", { artist: "Artist A", title: "Title A" }));
    expect(within(rows[0]).queryByText(tx("history:plays.notOwned"))).toBeNull();
    expect(within(rows[1]).getByText(tx("history:plays.notOwned"))).toBeInTheDocument();
    // Wishlists are off: no offer.
    expect(screen.queryByRole("button", { name: tx("history:wishlist.add") })).toBeNull();
  });

  it("says a set is already imported, and opens that set", async () => {
    backend({
      importAnswer: { outcome: "refused", reason: "alreadyImported", sessionId: SESSION.id, skippedLines: 0 },
    });
    dialog.open.mockResolvedValue(String.raw`C:\dj\again.m3u8`);
    renderScreen();
    const user = userEvent.setup();
    await user.click(await screen.findByRole("button", { name: tx("history:import.choose") }));
    expect(await screen.findByText(tx("history:refusal.alreadyImported"))).toBeInTheDocument();
  });

  it("offers the plays you don't have to a wishlist while wishlists are on", async () => {
    const fake = backend({ wishlistOffer: true });
    dialog.open.mockResolvedValue(String.raw`C:\dj\set.m3u8`);
    renderScreen();
    const user = userEvent.setup();
    await user.click(await screen.findByRole("button", { name: tx("history:import.choose") }));

    await user.click(await screen.findByRole("button", { name: tx("history:wishlist.add") }));

    expect(await screen.findByText(tx("history:wishlist.added", { count: 1 }))).toBeInTheDocument();
    expect(fake.called("history_add_to_wishlist")[0].args).toEqual({ sessionId: SESSION.id, wishlistId: 7 });
  });

  it("corrects a set's date and says when the date is refused", async () => {
    const fake = backend();
    dialog.open.mockResolvedValue(String.raw`C:\dj\set.m3u8`);
    renderScreen();
    const user = userEvent.setup();
    await user.click(await screen.findByRole("button", { name: tx("history:import.choose") }));

    const date = await screen.findByLabelText(tx("history:plays.date"));
    await user.clear(date);
    await user.type(date, "2026-05-04");
    await user.click(screen.getByRole("button", { name: tx("history:plays.saveDate") }));

    expect(await screen.findByText(tx("history:refusal.badDate"))).toBeInTheDocument();
    expect(fake.called("history_set_date")[0].args).toEqual({ sessionId: SESSION.id, playedOn: "2026-05-04" });
  });
});
