import { QueryClientProvider } from "@tanstack/react-query";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { cleanup, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createQueryClient } from "../app/queryClient";
import type { BundleSummary, ErrorKind, KeptLibrary } from "../bindings";
import { tx } from "../test/tx";
import { BundlePanel } from "./BundlePanel";

const dialog = vi.hoisted(() => ({ open: vi.fn(), save: vi.fn() }));
vi.mock("@tauri-apps/plugin-dialog", () => dialog);
vi.mock("@tauri-apps/api/path", () => ({
  desktopDir: vi.fn(async () => String.raw`C:\Synthetic\Desktop`),
  join: vi.fn(async (...parts: string[]) => parts.join("\\")),
}));

const MADE_PATH = String.raw`C:\Synthetic\data\bundles\tracklist-pro-20261107-120000000.zip`;
const MADE_NAME = "tracklist-pro-20261107-120000000.zip";
const PICKED_SAVE = String.raw`D:\Synthetic\Moving\laptop.zip`;
const PICKED_OPEN = String.raw`D:\Synthetic\Moving\from-the-studio.zip`;

const SUMMARY: BundleSummary = {
  createdMs: 1_794_000_000_000,
  appVersion: "9.8.7",
  schemaVersion: 40,
  counts: { tracks: 1234, crates: 5, playlists: 6, tags: 7, musicFolders: 2 },
};
const BACKUP: KeptLibrary = {
  id: "1793000000000",
  createdMs: 1_793_000_000_000,
  counts: { tracks: 321, crates: 4, playlists: 3, tags: 2, musicFolders: 1 },
};

interface Options {
  /** Makes this command fail with this error kind. */
  fail?: Record<string, ErrorKind>;
  backups?: KeptLibrary[];
}

/** Stands in for the Rust side, and remembers every command with its arguments. */
function fakeBackend({ fail = {}, backups = [] }: Options = {}) {
  const calls: { cmd: string; args: unknown }[] = [];
  mockIPC((cmd, args) => {
    calls.push({ cmd, args });
    if (fail[cmd]) throw { kind: fail[cmd], params: {} };
    switch (cmd) {
      case "create_bundle":
        return { path: MADE_PATH, manifest: {} };
      case "save_bundle_as":
        return null;
      case "stage_import":
      case "stage_restore":
        return SUMMARY;
      case "confirm_import":
        return null;
      case "list_backups":
        return backups;
      default:
        throw new Error(`unexpected command ${cmd}`);
    }
  });
  return calls;
}

const commandsSent = (calls: { cmd: string }[]) => calls.map((c) => c.cmd);

function renderPanel() {
  render(
    <QueryClientProvider client={createQueryClient()}>
      <BundlePanel />
    </QueryClientProvider>,
  );
}

beforeEach(() => {
  dialog.open.mockReset();
  dialog.save.mockReset();
});

afterEach(() => {
  cleanup();
  clearMocks();
});

describe("create a bundle", () => {
  it("makes it, asks where to save, then says what it saved", async () => {
    const calls = fakeBackend();
    dialog.save.mockResolvedValue(PICKED_SAVE);
    renderPanel();
    await userEvent.click(screen.getByRole("button", { name: tx("settings:bundle.create") }));
    expect(await screen.findByText(tx("settings:bundle.saved", { name: "laptop.zip" }))).toBeInTheDocument();
    const saveCall = calls.find((c) => c.cmd === "save_bundle_as");
    expect(saveCall?.args).toEqual({ name: MADE_NAME, destPath: PICKED_SAVE });
    // The dialog started at the Desktop, filtered to .zip.
    const options = dialog.save.mock.calls[0][0] as { defaultPath: string; filters: { extensions: string[] }[] };
    expect(options.defaultPath.startsWith(String.raw`C:\Synthetic\Desktop`)).toBe(true);
    expect(options.defaultPath.endsWith(".zip")).toBe(true);
    expect(options.filters[0].extensions).toEqual(["zip"]);
  });

  it("shows a wait state while the bundle is made", async () => {
    let finish: (value: unknown) => void = () => {};
    mockIPC((cmd) => {
      if (cmd === "list_backups") return [];
      if (cmd === "create_bundle") return new Promise((resolve) => (finish = resolve));
      throw new Error(`unexpected command ${cmd}`);
    });
    dialog.save.mockResolvedValue(null);
    renderPanel();
    await userEvent.click(screen.getByRole("button", { name: tx("settings:bundle.create") }));
    expect(await screen.findByText(tx("settings:bundle.creating"))).toBeInTheDocument();
    expect(screen.getByRole("button", { name: tx("settings:bundle.create") })).toBeDisabled();
    finish({ path: MADE_PATH, manifest: {} });
    expect(await screen.findByText(tx("settings:bundle.notSaved"))).toBeInTheDocument();
  });

  it("keeps nothing but the app's own copy when the dialog is cancelled", async () => {
    const calls = fakeBackend();
    dialog.save.mockResolvedValue(null);
    renderPanel();
    await userEvent.click(screen.getByRole("button", { name: tx("settings:bundle.create") }));
    expect(await screen.findByText(tx("settings:bundle.notSaved"))).toBeInTheDocument();
    expect(commandsSent(calls)).not.toContain("save_bundle_as");
  });

  it("says why when the place is refused", async () => {
    fakeBackend({ fail: { save_bundle_as: "saveExists" } });
    dialog.save.mockResolvedValue(PICKED_SAVE);
    renderPanel();
    await userEvent.click(screen.getByRole("button", { name: tx("settings:bundle.create") }));
    expect(await screen.findByRole("alert")).toHaveTextContent(tx("settings:bundle.save.exists"));
    expect(screen.queryByText(tx("settings:bundle.saved", { name: "laptop.zip" }))).toBeNull();
  });
});

describe("import a bundle", () => {
  it("checks the file, then asks, saying plainly what happens, before replacing", async () => {
    const calls = fakeBackend();
    dialog.open.mockResolvedValue(PICKED_OPEN);
    renderPanel();
    await userEvent.click(screen.getByRole("button", { name: tx("settings:bundle.import") }));
    const confirm = await screen.findByRole("alertdialog");
    expect(calls.find((c) => c.cmd === "stage_import")?.args).toEqual({ path: PICKED_OPEN });
    expect(within(confirm).getByText(tx("settings:bundle.confirm.importTitle"))).toBeInTheDocument();
    expect(within(confirm).getByText(/9\.8\.7/)).toBeInTheDocument();
    expect(within(confirm).getByText(tx("settings:bundle.confirm.tracks", { value: 1234 }))).toBeInTheDocument();
    expect(within(confirm).getByText(tx("settings:bundle.confirm.folders", { value: 2 }))).toBeInTheDocument();
    expect(within(confirm).getByText(tx("settings:bundle.confirm.detail"))).toBeInTheDocument();
    expect(within(confirm).getByText(tx("settings:bundle.confirm.drives"))).toBeInTheDocument();
    // Nothing is replaced until the user says so.
    expect(commandsSent(calls)).not.toContain("confirm_import");
    await userEvent.click(within(confirm).getByRole("button", { name: tx("settings:bundle.confirm.replace") }));
    expect(await screen.findByText(tx("settings:bundle.restarting"))).toBeInTheDocument();
    expect(commandsSent(calls)).toContain("confirm_import");
  });

  it("changes nothing when the user cancels the confirm or the dialog", async () => {
    const calls = fakeBackend();
    dialog.open.mockResolvedValueOnce(null).mockResolvedValueOnce(PICKED_OPEN);
    renderPanel();
    await userEvent.click(screen.getByRole("button", { name: tx("settings:bundle.import") }));
    await waitFor(() => expect(dialog.open).toHaveBeenCalledTimes(1));
    expect(screen.queryByRole("alertdialog")).toBeNull();
    await userEvent.click(screen.getByRole("button", { name: tx("settings:bundle.import") }));
    await userEvent.click(await screen.findByRole("button", { name: tx("settings:bundle.confirm.cancel") }));
    expect(screen.queryByRole("alertdialog")).toBeNull();
    expect(commandsSent(calls)).not.toContain("confirm_import");
  });

  it.each([
    ["bundleNewer", "settings:bundle.error.newer"],
    ["bundleNotOurs", "settings:bundle.error.notOurs"],
    ["bundleDamaged", "settings:bundle.error.damaged"],
  ] as const)("refuses a file that is %s, in plain words, with no confirm", async (kind, key) => {
    const calls = fakeBackend({ fail: { stage_import: kind } });
    dialog.open.mockResolvedValue(PICKED_OPEN);
    renderPanel();
    await userEvent.click(screen.getByRole("button", { name: tx("settings:bundle.import") }));
    expect(await screen.findByRole("alert")).toHaveTextContent(tx(key));
    expect(screen.queryByRole("alertdialog")).toBeNull();
    expect(commandsSent(calls)).not.toContain("confirm_import");
  });
});

describe("backups", () => {
  it("says so when there are none", async () => {
    fakeBackend();
    renderPanel();
    expect(await screen.findByText(tx("settings:bundle.backups.empty"))).toBeInTheDocument();
  });

  it("lists each backup with its counts and restores one after a confirm", async () => {
    const calls = fakeBackend({ backups: [BACKUP] });
    renderPanel();
    const restore = await screen.findByRole("button", { name: tx("settings:bundle.backups.restore") });
    expect(screen.getByText(/321/)).toBeInTheDocument();
    await userEvent.click(restore);
    const confirm = await screen.findByRole("alertdialog");
    expect(calls.find((c) => c.cmd === "stage_restore")?.args).toEqual({ id: BACKUP.id });
    expect(within(confirm).getByText(tx("settings:bundle.confirm.restoreTitle"))).toBeInTheDocument();
    await userEvent.click(within(confirm).getByRole("button", { name: tx("settings:bundle.confirm.replace") }));
    expect(await screen.findByText(tx("settings:bundle.restarting"))).toBeInTheDocument();
    expect(commandsSent(calls)).toContain("confirm_import");
  });

  it("says when the backup is gone", async () => {
    fakeBackend({ backups: [BACKUP], fail: { stage_restore: "bundleBackupNotFound" } });
    renderPanel();
    await userEvent.click(await screen.findByRole("button", { name: tx("settings:bundle.backups.restore") }));
    expect(await screen.findByRole("alert")).toHaveTextContent(tx("settings:bundle.error.backupNotFound"));
  });
});
