import { QueryClientProvider } from "@tanstack/react-query";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { act, cleanup, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useActivityStore } from "../activity/activityStore";
import { createQueryClient } from "../app/queryClient";
import type { Preflight, SendFailure, SendState, XmlSource } from "../bindings";
import i18n from "../i18n";
import { MAX_ROWS, SendChecklist } from "./SendChecklist";
import { tx } from "../test/tx";

const dialog = vi.hoisted(() => ({ open: vi.fn() }));
vi.mock("@tauri-apps/plugin-dialog", () => dialog);

const EXPORT = "C:\\Users\\dj\\Documents\\rekordbox.xml";
const SEND_FILE = "C:\\Users\\dj\\AppData\\Roaming\\com.tracklistpro.desktop\\tracklist-pro.xml";

function preflight(fields: Partial<Preflight> = {}): Preflight {
  return {
    token: "token-1",
    export: {
      path: EXPORT,
      modifiedMs: Date.UTC(2026, 9, 1, 12, 0),
      readAt: "2026-10-01T12:05:00.000Z",
      notStored: 0,
    },
    newTracks: 3,
    knownTracks: 12,
    leftOut: [],
    fileMissing: [],
    otherFile: [],
    losesEntries: [],
    refusal: null,
    nothingToSend: false,
    canSend: true,
    needsConfirm: false,
    ...fields,
  };
}

function label(id: number, title: string | null, fileName: string | null = `${id}.mp3`) {
  return { libraryTrack: id, title, artist: null, fileName };
}

/**
 * Stands in for the Rust side. Each step ends at once: `onPrepare` and
 * `onWrite` decide what the state records.
 */
function fakeBackend(initial: Partial<SendState> = {}) {
  const backend = {
    state: {
      revision: 0,
      filePath: SEND_FILE,
      exportPath: EXPORT,
      preflight: null,
      step: null,
      failure: null,
      sent: null,
      ...initial,
    } as SendState,
    source: {
      path: EXPORT,
      watch: false,
      lastRead: null,
      lastFailure: null,
      newerExport: null,
      exportFolder: "C:\\Users\\dj\\Documents",
    } as XmlSource,
    prepares: [] as (string | null)[],
    writes: [] as { token: string; confirmed: boolean }[],
    refuse: null as unknown,
    /** A step ends: the state records it and its revision moves. */
    end(change: Partial<SendState>) {
      backend.state = { ...backend.state, ...change, revision: backend.state.revision + 1 };
    },
    onPrepare: (path: string) => {
      backend.end({
        exportPath: path,
        preflight: preflight(),
        step: "prepare",
        failure: null,
        sent: null,
      });
    },
    onWrite: () => {
      const known = backend.state.preflight?.knownTracks ?? 0;
      backend.end({
        preflight: null,
        step: "write",
        failure: null,
        sent: { at: "2026-10-01T12:10:00.000Z", newTracks: 3, knownTracks: known },
      });
    },
  };
  mockIPC((cmd, args) => {
    const a = args as Record<string, unknown>;
    if (cmd === "send_state") return backend.state;
    if (cmd === "rekordbox_xml_source") return backend.source;
    if (cmd === "prepare_send") {
      const path = (a.path as string | null) ?? null;
      backend.prepares.push(path);
      if (backend.refuse) throw backend.refuse;
      backend.onPrepare(path ?? backend.state.exportPath ?? "");
      return 7;
    }
    if (cmd === "write_send") {
      backend.writes.push({ token: a.token as string, confirmed: a.confirmed as boolean });
      backend.onWrite();
      return 8;
    }
    throw new Error(`unexpected command ${cmd}`);
  });
  return backend;
}

function failing(failure: SendFailure, backend: ReturnType<typeof fakeBackend>) {
  return () => backend.end({ step: "write", failure });
}

function renderChecklist(afterSend?: React.ReactNode) {
  const onClose = vi.fn();
  render(
    <QueryClientProvider client={createQueryClient()}>
      <SendChecklist onClose={onClose} afterSend={afterSend} />
    </QueryClientProvider>,
  );
  return { onClose };
}

// A date and time as the checklist shows them.
const when = (date: Date | string | number) =>
  new Intl.DateTimeFormat(i18n.language, { dateStyle: "medium", timeStyle: "short" }).format(
    new Date(date),
  );
const SAVED = tx("send:export.saved", { when: when(Date.UTC(2026, 9, 1, 12, 0)) });
// What the fake backend records when the file is written.
const WRITTEN = tx("send:write.written", { when: when("2026-10-01T12:10:00.000Z") });

const step = (title: string) =>
  within(screen.getByRole("heading", { level: 3, name: title }).closest("li") as HTMLElement);

beforeEach(() => {
  dialog.open.mockReset();
  useActivityStore.getState().reset();
});
afterEach(() => {
  cleanup();
  clearMocks();
  vi.unstubAllGlobals();
});

describe("the send checklist", () => {
  it("covers rekordbox's side in order: export, review, write, import, after", async () => {
    fakeBackend();
    renderChecklist();
    await screen.findByText(EXPORT);
    const titles = screen.getAllByRole("heading", { level: 3 }).map((h) => h.textContent);
    expect(titles).toEqual([
      tx("send:export.title"),
      tx("send:review.title"),
      tx("send:write.title"),
      tx("send:import.title"),
      tx("send:after.title"),
    ]);
    expect(
      screen.getByText(tx("send:warning")),
    ).toBeInTheDocument();
    const steps = step(tx("send:import.title"));
    expect(steps.getByText(tx("send:import.point"))).toBeInTheDocument();
    expect(steps.getByText(tx("send:import.refresh"))).toBeInTheDocument();
    expect(steps.getByText(tx("send:import.tracks"))).toBeInTheDocument();
  });

  it("can't write the file before the export is read and reviewed", async () => {
    const backend = fakeBackend();
    renderChecklist();
    await screen.findByText(EXPORT);
    expect(screen.getByText(tx("send:review.notYet"))).toBeInTheDocument();
    expect(screen.getByRole("button", { name: tx("send:write.go") })).toBeDisabled();
    expect(backend.writes).toEqual([]);
  });

  it("reads the export on request and shows when rekordbox saved it and what will be sent", async () => {
    const backend = fakeBackend();
    renderChecklist();
    await userEvent.click(await screen.findByRole("button", { name: tx("send:export.read") }));

    expect(await screen.findByText(SAVED)).toBeInTheDocument();
    expect(backend.prepares).toEqual([null]);
    const review = step(tx("send:review.title"));
    expect(review.getByText(tx("send:review.new", { count: 3 }))).toBeInTheDocument();
    expect(review.getByText(tx("send:review.known", { count: 12 }))).toBeInTheDocument();
    // How many dialogs to expect is stated before the import.
    expect(
      screen.getByText(
        tx("send:import.dialogs", { count: 12 }),
      ),
    ).toBeInTheDocument();
    expect(screen.getByRole("button", { name: tx("send:write.go") })).toBeEnabled();
  });

  it("reads a newly chosen export, which becomes the chosen one", async () => {
    const backend = fakeBackend({ exportPath: null });
    const picked = "D:\\exports\\collection.xml";
    dialog.open.mockResolvedValue(picked);
    renderChecklist();
    expect(await screen.findByText(tx("send:export.noneChosen"))).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: tx("send:export.read") })).not.toBeInTheDocument();

    await userEvent.click(screen.getByRole("button", { name: tx("send:export.choose") }));
    expect(await screen.findByText(picked)).toBeInTheDocument();
    expect(backend.prepares).toEqual([picked]);
  });

  it("writes the file only on the go, for the preflight shown, then counts the dialogs sent", async () => {
    const backend = fakeBackend({ preflight: preflight({ knownTracks: 1 }) });
    renderChecklist();
    const go = await screen.findByRole("button", { name: tx("send:write.go") });
    await waitFor(() => expect(go).toBeEnabled());
    expect(backend.writes).toEqual([]);

    await userEvent.click(go);
    expect(await screen.findByText(WRITTEN)).toBeInTheDocument();
    expect(backend.writes).toEqual([{ token: "token-1", confirmed: false }]);
    expect(
      screen.getByText(tx("send:import.dialogs", { count: 1 })),
    ).toBeInTheDocument();
    // One preflight, one send: the next needs a new read.
    expect(screen.getByRole("button", { name: tx("send:write.go") })).toBeDisabled();
  });

  it("says when no dialogs are to be expected", async () => {
    fakeBackend({ preflight: preflight({ knownTracks: 0 }) });
    renderChecklist();
    expect(await screen.findByText(tx("send:import.dialogsNone"))).toBeInTheDocument();
    expect(screen.getByText(tx("send:review.known", { count: 0 }))).toBeInTheDocument();
  });

  it("needs Send anyway ticked when a crate arrives with tracks left out", async () => {
    const backend = fakeBackend({
      preflight: preflight({
        needsConfirm: true,
        leftOut: [{ track: label(5, "Gone"), reason: "fileMissing", attribute: null }],
        losesEntries: [{ kind: "crate", path: ["Friday", "Peak"], lost: 2, entries: 9 }],
      }),
    });
    renderChecklist();
    const review = step(tx("send:review.title"));
    expect(await review.findByText(tx("send:review.losesEntriesRow", { name: "Friday / Peak", count: 2, entries: 9 }))).toBeInTheDocument();
    expect(review.getByText(tx("send:review.leftOut", { count: 1 }))).toBeInTheDocument();
    expect(review.getByText(tx("send:review.leftOutReason.fileMissing"))).toBeInTheDocument();
    const go = screen.getByRole("button", { name: tx("send:write.go") });
    expect(go).toBeDisabled();

    await userEvent.click(review.getByRole("checkbox", { name: tx("send:review.confirm") }));
    expect(go).toBeEnabled();
    await userEvent.click(go);
    await screen.findByText(WRITTEN);
    expect(backend.writes).toEqual([{ token: "token-1", confirmed: true }]);
  });

  it("needs Send anyway ticked when the export held tracks that couldn't be read", async () => {
    fakeBackend({
      preflight: preflight({
        needsConfirm: true,
        export: { ...preflight().export, notStored: 2 },
      }),
    });
    renderChecklist();
    expect(
      await screen.findByText(tx("send:review.notStored", { count: 2 })),
    ).toBeInTheDocument();
    expect(screen.getByRole("button", { name: tx("send:write.go") })).toBeDisabled();
    await userEvent.click(screen.getByRole("checkbox", { name: tx("send:review.confirm") }));
    expect(screen.getByRole("button", { name: tx("send:write.go") })).toBeEnabled();
  });

  it("asks for no confirm when nothing needs one", async () => {
    fakeBackend({ preflight: preflight() });
    renderChecklist();
    await screen.findByText(tx("send:review.new", { count: 3 }));
    expect(screen.queryByRole("checkbox", { name: tx("send:review.confirm") })).not.toBeInTheDocument();
  });

  it("a new preflight starts with Send anyway unticked", async () => {
    const backend = fakeBackend({ preflight: preflight({ needsConfirm: true, token: "old" }) });
    backend.onPrepare = () =>
      backend.end({ preflight: preflight({ needsConfirm: true, token: "new" }) });
    renderChecklist();
    await userEvent.click(await screen.findByRole("checkbox", { name: tx("send:review.confirm") }));
    expect(screen.getByRole("checkbox", { name: tx("send:review.confirm") })).toBeChecked();

    await userEvent.click(screen.getByRole("button", { name: tx("send:export.read") }));
    await waitFor(() => expect(backend.state.revision).toBe(1));
    await waitFor(() =>
      expect(screen.getByRole("checkbox", { name: tx("send:review.confirm") })).not.toBeChecked(),
    );
    expect(screen.getByRole("button", { name: tx("send:write.go") })).toBeDisabled();
  });

  it("lists tracks sent with their file missing, and asks for no confirm", async () => {
    fakeBackend({ preflight: preflight({ fileMissing: [label(4, "Gone"), label(6, "Lost")] }) });
    renderChecklist();
    const review = step(tx("send:review.title"));
    expect(
      await review.findByText(
        tx("send:review.fileMissing", { count: 2 }),
      ),
    ).toBeInTheDocument();
    expect(review.getByText("Gone")).toBeInTheDocument();
    expect(review.getByText("Lost")).toBeInTheDocument();
    expect(screen.queryByRole("checkbox", { name: tx("send:review.confirm") })).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: tx("send:write.go") })).toBeEnabled();
  });

  it("lists tracks rekordbox already has as another file, and lets them go", async () => {
    fakeBackend({
      preflight: preflight({
        otherFile: [
          {
            track: { ...label(9, "Twice"), artist: "Kit" },
            libraryFile: "E:\\Music\\twice.flac",
            rekordboxFile: "E:\\Old\\twice.mp3",
          },
        ],
      }),
    });
    renderChecklist();
    const review = step(tx("send:review.title"));
    expect(
      await review.findByText(
        tx("send:review.otherFile", { count: 1 }),
      ),
    ).toBeInTheDocument();
    expect(review.getByText(tx("send:trackWithArtist", { title: "Twice", artist: "Kit" }))).toBeInTheDocument();
    expect(
      review.getByText(tx("send:review.libraryFile", { path: "E:\\Music\\twice.flac" })),
    ).toBeInTheDocument();
    expect(
      review.getByText(tx("send:review.rekordboxFile", { path: "E:\\Old\\twice.mp3" })),
    ).toBeInTheDocument();
    expect(screen.getByRole("button", { name: tx("send:write.go") })).toBeEnabled();
  });

  it("names a left-out track by its file when it has no title, and says which field is at fault", async () => {
    fakeBackend({
      preflight: preflight({
        leftOut: [
          { track: label(1, null, "untitled.mp3"), reason: "unsendableCharacter", attribute: "Comments" },
        ],
      }),
    });
    renderChecklist();
    const review = step(tx("send:review.title"));
    expect(await review.findByText("untitled.mp3")).toBeInTheDocument();
    expect(
      review.getByText(tx("send:review.leftOutReason.unsendableCharacter", { attribute: "Comments" })),
    ).toBeInTheDocument();
  });

  it("shows the first rows of a long list and counts the rest", async () => {
    const many = Array.from({ length: MAX_ROWS + 3 }, (_, i) => ({
      track: label(i + 1, `Track ${i + 1}`),
      reason: "fileMissing" as const,
      attribute: null,
    }));
    fakeBackend({ preflight: preflight({ leftOut: many }) });
    renderChecklist();
    const review = step(tx("send:review.title"));
    expect(await review.findByText(`Track ${MAX_ROWS}`)).toBeInTheDocument();
    expect(review.queryByText(`Track ${MAX_ROWS + 1}`)).not.toBeInTheDocument();
    expect(review.getByText(tx("send:more", { count: 3 }))).toBeInTheDocument();
  });

  it("explains a refused send and won't start it", async () => {
    fakeBackend({
      preflight: preflight({
        canSend: false,
        needsConfirm: true,
        refusal: { reason: "sameName", path: ["Crates", "warm up"] },
      }),
    });
    renderChecklist();
    expect(
      await screen.findByText(
        tx("send:review.refused.sameName", { path: "Crates / warm up" }),
      ),
    ).toBeInTheDocument();
    expect(screen.getByRole("button", { name: tx("send:write.go") })).toBeDisabled();
    // No confirm is offered for a send that can't go.
    expect(screen.queryByRole("checkbox", { name: tx("send:review.confirm") })).not.toBeInTheDocument();
  });

  it("explains a send refused for an incomplete export", async () => {
    fakeBackend({
      preflight: preflight({
        canSend: false,
        refusal: { reason: "incompleteExport", path: [] },
      }),
    });
    renderChecklist();
    expect(
      await screen.findByText(
        tx("send:review.refused.incompleteExport"),
      ),
    ).toBeInTheDocument();
    expect(screen.getByRole("button", { name: tx("send:write.go") })).toBeDisabled();
  });

  it("says No tracks or crates to send and won't start a send", async () => {
    fakeBackend({ preflight: preflight({ canSend: false, nothingToSend: true, newTracks: 0, knownTracks: 0 }) });
    renderChecklist();
    expect(await screen.findByText(tx("send:review.nothing"))).toBeInTheDocument();
    expect(screen.getByRole("button", { name: tx("send:write.go") })).toBeDisabled();
  });

  it("shows why a go was refused, under the write step", async () => {
    const backend = fakeBackend({ preflight: preflight() });
    backend.onWrite = failing("exportChanged", backend);
    renderChecklist();
    const go = await screen.findByRole("button", { name: tx("send:write.go") });
    await waitFor(() => expect(go).toBeEnabled());
    await userEvent.click(go);
    const write = step(tx("send:write.title"));
    expect(
      await write.findByText(tx("send:failure.exportChanged")),
    ).toBeInTheDocument();
    expect(screen.queryByText(WRITTEN)).not.toBeInTheDocument();
  });

  it("shows why the export couldn't be read, in the rekordbox panel's words", async () => {
    const backend = fakeBackend();
    backend.onPrepare = () => {
      backend.source = {
        ...backend.source,
        lastFailure: { path: EXPORT, reason: "damaged", at: "2026-10-01T12:00:00.000Z" },
      };
      backend.end({ preflight: null, step: "prepare", failure: "readFailed" });
    };
    renderChecklist();
    await userEvent.click(await screen.findByRole("button", { name: tx("send:export.read") }));
    const read = step(tx("send:export.title"));
    expect(
      await read.findByText(tx("rekordbox:failed.damaged", { path: EXPORT })),
    ).toBeInTheDocument();
    expect(screen.getByRole("button", { name: tx("send:write.go") })).toBeDisabled();
  });

  it("shows a refused start in words, never as raw error text", async () => {
    const backend = fakeBackend({ exportPath: EXPORT });
    backend.refuse = { kind: "noRekordboxXml", params: {} };
    renderChecklist();
    await userEvent.click(await screen.findByRole("button", { name: tx("send:export.read") }));
    expect(await screen.findByText(tx("rekordbox:error.noneChosen"))).toBeInTheDocument();
  });

  it("shows the file's path and copies it", async () => {
    fakeBackend();
    const writeText = vi.fn().mockResolvedValue(undefined);
    renderChecklist();
    const path = await screen.findByRole("textbox", { name: tx("send:write.path") });
    expect(path).toHaveValue(SEND_FILE);
    expect(path).toHaveAttribute("readonly");
    // After the render: userEvent installs its own clipboard on setup.
    vi.stubGlobal("navigator", { ...navigator, clipboard: { writeText } });
    await userEvent.click(screen.getByRole("button", { name: tx("send:write.copy") }));
    expect(await screen.findByText(tx("send:write.copied"))).toBeInTheDocument();
    expect(writeText).toHaveBeenCalledWith(SEND_FILE);
  });

  it("says so when the path can't be copied", async () => {
    fakeBackend();
    renderChecklist();
    await screen.findByRole("textbox", { name: tx("send:write.path") });
    vi.stubGlobal("navigator", {
      ...navigator,
      clipboard: { writeText: vi.fn().mockRejectedValue(new Error("denied")) },
    });
    await userEvent.click(screen.getByRole("button", { name: tx("send:write.copy") }));
    expect(
      await screen.findByText(tx("send:write.copyFailed")),
    ).toBeInTheDocument();
  });

  it("shows a failed read under the export step when the checklist is opened again", async () => {
    fakeBackend({ step: "prepare", failure: "cancelled" });
    renderChecklist();
    const read = step(tx("send:export.title"));
    expect(await read.findByText(tx("send:failure.cancelled"))).toBeInTheDocument();
    expect(step(tx("send:write.title")).queryByText(tx("send:failure.cancelled"))).not.toBeInTheDocument();
  });

  it("is busy while a step runs, and shows its result once the app records the step's end", async () => {
    const backend = fakeBackend();
    // The read is under way: nothing is recorded yet.
    backend.onPrepare = () => {};
    renderChecklist();
    await userEvent.click(await screen.findByRole("button", { name: tx("send:export.read") }));

    expect(await screen.findByText(tx("send:export.reading"))).toBeInTheDocument();
    expect(screen.getByRole("button", { name: tx("send:export.read") })).toBeDisabled();
    expect(screen.getByRole("button", { name: tx("send:export.chooseAnother") })).toBeDisabled();
    expect(screen.getByRole("button", { name: tx("send:write.go") })).toBeDisabled();
    expect(screen.getByText(tx("send:review.notYet"))).toBeInTheDocument();

    // The job ends; the checklist finds out by asking again.
    backend.end({ preflight: preflight(), step: "prepare" });
    expect(await screen.findByText(tx("send:review.new", { count: 3 }))).toBeInTheDocument();
    expect(screen.queryByText(tx("send:export.reading"))).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: tx("send:write.go") })).toBeEnabled();
  });

  it("stops waiting when Activity says the step's job ended without a result", async () => {
    const backend = fakeBackend();
    backend.onPrepare = () => {};
    renderChecklist();
    await userEvent.click(await screen.findByRole("button", { name: tx("send:export.read") }));
    expect(await screen.findByText(tx("send:export.reading"))).toBeInTheDocument();

    // Cancelled while it was still queued: no step ever ends.
    act(() => {
      const activity = useActivityStore.getState();
      activity.applySnapshot({ seq: 0, jobs: [] });
      activity.applyUpdates([
        { seq: 1, id: 7, kind: "export", status: "cancelled", progress: null, priority: 10 },
      ]);
    });
    await waitFor(() =>
      expect(screen.queryByText(tx("send:export.reading"))).not.toBeInTheDocument(),
    );
    expect(screen.getByRole("button", { name: tx("send:export.read") })).toBeEnabled();
  });

  it("mounts the after-send lists in its last step", async () => {
    fakeBackend();
    renderChecklist(<p>after-send lists</p>);
    const last = step(tx("send:after.title"));
    expect(await last.findByText("after-send lists")).toBeInTheDocument();
  });

  it("closes on request", async () => {
    fakeBackend();
    const { onClose } = renderChecklist();
    await userEvent.click(await screen.findByRole("button", { name: tx("send:close") }));
    expect(onClose).toHaveBeenCalledOnce();
  });
});
