import { QueryClientProvider } from "@tanstack/react-query";
import { emit } from "@tauri-apps/api/event";
import { clearMocks } from "@tauri-apps/api/mocks";
import { act, cleanup, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { createQueryClient } from "../app/queryClient";
import type { Identified, InboxLookupStatus, JobUpdate } from "../bindings";
import "../i18n";
import { inboxBackend, inboxFolder, inboxItem } from "../test/inboxBackend";
import { tx } from "../test/tx";
import { InboxScreen } from "./InboxScreen";

afterEach(() => {
  cleanup();
  clearMocks();
});

const FOLDER = inboxFolder(1, String.raw`C:\Users\dj\Downloads`);

const status = (fileId: number, fields: Partial<InboxLookupStatus>): InboxLookupStatus => ({
  fileId,
  state: "never",
  reason: null,
  found: 0,
  when: null,
  laterAttempt: null,
  identified: null,
  ...fields,
});

const identified = (fields: Partial<Identified>): Identified => ({
  artist: null,
  title: null,
  mix: null,
  sources: [],
  ...fields,
});

const FILE = { id: 1001, name: "a.mp3" };

function renderScreen() {
  return render(
    <QueryClientProvider client={createQueryClient()}>
      <InboxScreen />
    </QueryClientProvider>,
  );
}

const rowOf = async (name: string) => (await screen.findByText(name)).closest("li") as HTMLElement;

const jobEnded = (): JobUpdate => ({ seq: 1, id: 1, kind: "identify_lookup", status: "done", progress: null, priority: 0 });

describe("the identification line under an Inbox file", () => {
  it("says what the file was identified as, with its version and where that came from", async () => {
    inboxBackend({
      folders: [FOLDER],
      items: [inboxItem(1, { fileName: "a.mp3" })],
      lookUps: [
        status(1001, {
          state: "found",
          found: 3,
          identified: identified({
            artist: "Some Artist",
            title: "Some Title",
            mix: "Extended Mix",
            sources: [{ kind: "online", service: "Service One", via: "Service Two" }],
          }),
        }),
      ],
    });
    renderScreen();
    const row = await rowOf("a.mp3");
    const source = tx("identify:lookUp.source.onlineVia", { service: "Service One", via: "Service Two" });
    const name = `Some Artist${tx("identify:inboxLine.nameJoin")}Some Title${tx("identify:inboxLine.mix", { mix: "Extended Mix" })}`;
    expect(await within(row).findByText(tx("identify:inboxLine.identifiedFrom", { name, source }))).toBeVisible();
  });

  it("leaves out the artist, title or version the look-up did not find", async () => {
    inboxBackend({
      folders: [FOLDER],
      items: [inboxItem(1, { fileName: "a.mp3" })],
      lookUps: [
        status(1001, {
          state: "found",
          found: 1,
          identified: identified({ title: "Only Title", sources: [{ kind: "tags", file: FILE }] }),
        }),
      ],
    });
    renderScreen();
    const row = await rowOf("a.mp3");
    const source = tx("identify:lookUp.source.tags");
    expect(await within(row).findByText(tx("identify:inboxLine.identifiedFrom", { name: "Only Title", source }))).toBeVisible();
  });

  it("says how many details were found when none of them is an artist or a title", async () => {
    inboxBackend({
      folders: [FOLDER],
      items: [inboxItem(1, { fileName: "a.mp3" })],
      lookUps: [status(1001, { state: "found", found: 2, identified: null })],
    });
    renderScreen();
    expect(await within(await rowOf("a.mp3")).findByText(tx("identify:inboxLine.found", { count: 2 }))).toBeVisible();
  });

  it("draws nothing for a file never looked up, or one the backend knows no status for", async () => {
    inboxBackend({
      folders: [FOLDER],
      items: [inboxItem(1, { fileName: "a.mp3" }), inboxItem(2, { fileName: "b.mp3" })],
      lookUps: [status(1001, { state: "never" })],
    });
    renderScreen();
    const first = await rowOf("a.mp3");
    const second = await rowOf("b.mp3");
    // Both rows are drawn; wait until the statuses have been asked for and answered.
    await waitFor(() => expect(first.querySelectorAll("p")).toHaveLength(0));
    expect(second.querySelectorAll("p")).toHaveLength(0);
  });

  describe.each([
    ["nothing was found", "nothing", "no_match", "identify:inboxLine.nothingWhy"],
    ["the look-up failed", "failed", "offline", "identify:inboxLine.failedWhy"],
    ["the look-up was skipped", "skipped", "services_off", "identify:inboxLine.skippedWhy"],
  ] as const)("when %s", (_name, state, code, key) => {
    it("says so with the reason", async () => {
      inboxBackend({
        folders: [FOLDER],
        items: [inboxItem(1, { fileName: "a.mp3" })],
        lookUps: [status(1001, { state, reason: code })],
      });
      renderScreen();
      const reason = tx(`identify:lookUp.reason.${code}`);
      expect(await within(await rowOf("a.mp3")).findByText(tx(key, { reason }))).toBeVisible();
    });
  });

  it("says so while the look-up is waiting or running", async () => {
    inboxBackend({
      folders: [FOLDER],
      items: [inboxItem(1, { fileName: "a.mp3" })],
      lookUps: [status(1001, { state: "running" })],
    });
    renderScreen();
    expect(await within(await rowOf("a.mp3")).findByText(tx("identify:inboxLine.looking"))).toBeVisible();
  });

  it("offers no Use on the row, and no look-up of its own", async () => {
    inboxBackend({
      folders: [FOLDER],
      items: [inboxItem(1, { fileName: "a.mp3" })],
      lookUps: [status(1001, { state: "found", found: 1, identified: identified({ title: "Some Title" }) })],
    });
    renderScreen();
    const row = await rowOf("a.mp3");
    await within(row).findByText(tx("identify:inboxLine.identified", { name: "Some Title" }));
    expect(within(row).queryByRole("button", { name: tx("identify:lookUp.use") })).toBeNull();
    expect(within(row).queryByRole("button", { name: tx("identify:lookUp.useAll") })).toBeNull();
  });

  it("asks once for the statuses of every file on the screen", async () => {
    const backend = inboxBackend({
      folders: [FOLDER],
      items: [inboxItem(1, { fileName: "a.mp3" }), inboxItem(2, { fileName: "b.mp3" }), inboxItem(3, { fileName: "c.mp3" })],
    });
    renderScreen();
    await rowOf("c.mp3");
    await waitFor(() => expect(backend.called("identify_inbox_status")).toHaveLength(1));
    expect(backend.called("identify_inbox_status")[0]?.args).toEqual({ fileIds: [1001, 1002, 1003] });
  });

  it("shows what a look-up found as soon as the background task ends", async () => {
    const backend = inboxBackend({
      folders: [FOLDER],
      items: [inboxItem(1, { fileName: "a.mp3" })],
      lookUps: [status(1001, { state: "running" })],
    });
    renderScreen();
    const row = await rowOf("a.mp3");
    await within(row).findByText(tx("identify:inboxLine.looking"));

    backend.state.lookUps = [
      status(1001, { state: "found", found: 1, identified: identified({ artist: "Some Artist", title: "Some Title" }) }),
    ];
    await act(() => emit("job-updates", [jobEnded()]));
    const name = `Some Artist${tx("identify:inboxLine.nameJoin")}Some Title`;
    expect(await within(row).findByText(tx("identify:inboxLine.identified", { name }))).toBeVisible();
    expect(within(row).queryByText(tx("identify:inboxLine.looking"))).toBeNull();
  });
});
