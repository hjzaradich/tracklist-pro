import { QueryClientProvider } from "@tanstack/react-query";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { cleanup, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { createQueryClient } from "../app/queryClient";
import type { OnlineService, OnlineServicesStatus, SaveTokenOutcome } from "../bindings";
import "../i18n";
import { tx } from "../test/tx";
import rawMessages from "../locales/en/onlineServices.json?raw";
import { OnlineServicesRows } from "./OnlineServicesRows";

const OFF: OnlineServicesStatus = {
  musicBrainz: false,
  acoustId: { on: false, keyMissing: false },
  discogs: { on: false, tokenSaved: false, storeAvailable: true },
  coverArt: false,
};

const TOKEN = "tok-3f9a-secret-value";
const ADDRESS = "https://www.discogs.com/settings/developers";

/** Stands in for the Rust side: the switches, the saved-token flag and every call made. */
function fakeBackend(
  initial: OnlineServicesStatus,
  { outcome = "saved", failSave = false }: { outcome?: SaveTokenOutcome; failSave?: boolean } = {},
) {
  const backend = {
    status: initial,
    outcome,
    sets: [] as { service: OnlineService; on: boolean }[],
    saved: [] as string[],
    removes: 0,
    commands: [] as string[],
  };
  mockIPC((cmd, args) => {
    backend.commands.push(cmd);
    switch (cmd) {
      case "online_services_status":
        return structuredClone(backend.status);
      case "set_online_service": {
        const { service, on } = args as { service: OnlineService; on: boolean };
        if (failSave) throw { kind: "database", params: {} };
        backend.sets.push({ service, on });
        const s = backend.status;
        backend.status =
          service === "musicBrainz"
            ? { ...s, musicBrainz: on }
            : service === "coverArt"
              ? { ...s, coverArt: on }
              : service === "acoustId"
                ? { ...s, acoustId: { ...s.acoustId, on } }
                : { ...s, discogs: { ...s.discogs, on } };
        return null;
      }
      case "save_discogs_token": {
        if (failSave) throw { kind: "database", params: {} };
        backend.saved.push((args as { token: string }).token);
        if (backend.outcome === "saved") {
          backend.status = { ...backend.status, discogs: { ...backend.status.discogs, tokenSaved: true } };
        }
        return backend.outcome;
      }
      case "remove_discogs_token": {
        backend.removes += 1;
        backend.status = { ...backend.status, discogs: { ...backend.status.discogs, tokenSaved: false } };
        return { saved: false, storeAvailable: true };
      }
      default:
        throw new Error(`unexpected command ${cmd}`);
    }
  });
  return backend;
}

function renderRows() {
  render(
    <QueryClientProvider client={createQueryClient()}>
      <OnlineServicesRows />
    </QueryClientProvider>,
  );
}

const switchNamed = (key: string) => screen.getByRole("checkbox", { name: tx(key) });

async function renderLoaded() {
  renderRows();
  await waitFor(() => expect(switchNamed("onlineServices:musicBrainz.label")).toBeEnabled());
}

const DISCOGS_ON: OnlineServicesStatus = { ...OFF, discogs: { on: true, tokenSaved: false, storeAvailable: true } };

describe("OnlineServicesRows", () => {
  afterEach(() => {
    cleanup();
    clearMocks();
    vi.unstubAllGlobals();
  });

  it("starts with every switch off and says what each service is sent", async () => {
    fakeBackend(OFF);
    await renderLoaded();
    for (const name of ["musicBrainz", "acoustId", "discogs", "coverArt"]) {
      expect(switchNamed(`onlineServices:${name}.label`)).not.toBeChecked();
      expect(screen.getByText(tx(`onlineServices:${name}.help`))).toBeVisible();
    }
    expect(screen.queryByLabelText(tx("onlineServices:discogs.tokenLabel"))).toBeNull();
  });

  it.each([
    ["musicBrainz", "musicBrainz"],
    ["acoustId", "acoustId"],
    ["discogs", "discogs"],
    ["coverArt", "coverArt"],
  ] as const)("the %s switch calls set with that service and shows the new state", async (name, service) => {
    const backend = fakeBackend(OFF);
    await renderLoaded();
    const box = switchNamed(`onlineServices:${name}.label`);
    await userEvent.click(box);
    await waitFor(() => expect(box).toBeChecked());
    await waitFor(() => expect(box).toBeEnabled());
    expect(backend.sets).toEqual([{ service, on: true }]);
    await userEvent.click(box);
    await waitFor(() => expect(box).not.toBeChecked());
    expect(backend.sets).toEqual([
      { service, on: true },
      { service, on: false },
    ]);
  });

  it("the other switches stay off when one is turned on", async () => {
    fakeBackend(OFF);
    await renderLoaded();
    const box = switchNamed("onlineServices:coverArt.label");
    await userEvent.click(box);
    await waitFor(() => expect(box).toBeChecked());
    for (const name of ["musicBrainz", "acoustId", "discogs"]) {
      expect(switchNamed(`onlineServices:${name}.label`)).not.toBeChecked();
    }
  });

  it("shows a plain error and keeps the old state when a switch can't be saved", async () => {
    fakeBackend(OFF, { failSave: true });
    await renderLoaded();
    const box = switchNamed("onlineServices:musicBrainz.label");
    await userEvent.click(box);
    expect(await screen.findByRole("alert")).toHaveTextContent(tx("onlineServices:error"));
    await waitFor(() => expect(box).not.toBeChecked());
  });

  it("AcoustID is disabled with its reason while the app key is missing, and the others still work", async () => {
    const backend = fakeBackend({ ...OFF, acoustId: { on: false, keyMissing: true } });
    await renderLoaded();
    const box = switchNamed("onlineServices:acoustId.label");
    expect(box).toBeDisabled();
    expect(box).toHaveAccessibleDescription(expect.stringContaining(tx("onlineServices:acoustId.unavailable")));
    expect(screen.getByText(tx("onlineServices:acoustId.unavailable"))).toBeVisible();
    await userEvent.click(box);
    expect(backend.sets).toEqual([]);
    expect(switchNamed("onlineServices:musicBrainz.label")).toBeEnabled();
  });

  it("AcoustID can be switched on, with no reason shown, once the key is there", async () => {
    fakeBackend(OFF);
    await renderLoaded();
    expect(switchNamed("onlineServices:acoustId.label")).toBeEnabled();
    expect(screen.queryByText(tx("onlineServices:acoustId.unavailable"))).toBeNull();
  });

  it("the Discogs token field is password-typed, empty, labelled, and shows the saved state", async () => {
    fakeBackend(DISCOGS_ON);
    await renderLoaded();
    const field = await screen.findByLabelText(tx("onlineServices:discogs.tokenLabel"));
    expect(field).toHaveAttribute("type", "password");
    expect(field).toHaveValue("");
    expect(screen.getByText(tx("onlineServices:discogs.notSaved"))).toBeVisible();
  });

  it("a saved token shows only that one is saved, never the token", async () => {
    fakeBackend({ ...OFF, discogs: { on: true, tokenSaved: true, storeAvailable: true } });
    await renderLoaded();
    const field = await screen.findByLabelText(tx("onlineServices:discogs.tokenLabel"));
    expect(field).toHaveValue("");
    expect(screen.getByText(tx("onlineServices:discogs.saved"))).toBeVisible();
    expect(screen.queryByText(tx("onlineServices:discogs.notSaved"))).toBeNull();
  });

  it("the token field is cleared after Save and the token text never appears in the page afterwards", async () => {
    const backend = fakeBackend(DISCOGS_ON);
    await renderLoaded();
    const field = await screen.findByLabelText(tx("onlineServices:discogs.tokenLabel"));
    await userEvent.type(field, TOKEN);
    await userEvent.click(screen.getByRole("button", { name: tx("onlineServices:discogs.save") }));
    await screen.findByText(tx("onlineServices:discogs.outcome.saved"));
    expect(backend.saved).toEqual([TOKEN]);
    const after = screen.getByLabelText(tx("onlineServices:discogs.tokenLabel")) as HTMLInputElement;
    expect(after.value).toBe("");
    expect(document.body.innerHTML).not.toContain(TOKEN);
    expect(document.body.textContent).not.toContain(TOKEN);
    await waitFor(() => expect(screen.getByText(tx("onlineServices:discogs.saved"))).toBeVisible());
    expect(document.body.innerHTML).not.toContain(TOKEN);
  });

  it("the token is cleared even when Discogs turns it down", async () => {
    fakeBackend(DISCOGS_ON, { outcome: "rejected" });
    await renderLoaded();
    const field = await screen.findByLabelText(tx("onlineServices:discogs.tokenLabel"));
    await userEvent.type(field, TOKEN);
    await userEvent.click(screen.getByRole("button", { name: tx("onlineServices:discogs.save") }));
    await screen.findByText(tx("onlineServices:discogs.outcome.rejected"));
    expect(field).toHaveValue("");
    expect(document.body.innerHTML).not.toContain(TOKEN);
  });

  it("Enter in the token field saves it", async () => {
    const backend = fakeBackend(DISCOGS_ON);
    await renderLoaded();
    const field = await screen.findByLabelText(tx("onlineServices:discogs.tokenLabel"));
    await userEvent.type(field, `${TOKEN}{Enter}`);
    await screen.findByText(tx("onlineServices:discogs.outcome.saved"));
    expect(backend.saved).toEqual([TOKEN]);
  });

  it.each([
    "saved",
    "switchOff",
    "empty",
    "storeUnavailable",
    "rejected",
    "couldNotCheck",
    "storeFailed",
  ] as const)("the %s outcome of Save shows its own line", async (outcome) => {
    fakeBackend(DISCOGS_ON, { outcome });
    await renderLoaded();
    const field = await screen.findByLabelText(tx("onlineServices:discogs.tokenLabel"));
    await userEvent.type(field, TOKEN);
    await userEvent.click(screen.getByRole("button", { name: tx("onlineServices:discogs.save") }));
    const line = await screen.findByText(tx(`onlineServices:discogs.outcome.${outcome}`));
    expect(line).toBeVisible();
    // Only the outcome the backend gave is shown.
    for (const other of ["saved", "rejected", "couldNotCheck", "storeFailed", "switchOff", "empty"] as const) {
      if (other !== outcome) expect(screen.queryByText(tx(`onlineServices:discogs.outcome.${other}`))).toBeNull();
    }
  });

  it("says so, in plain words, when the app itself fails to save the token", async () => {
    fakeBackend(DISCOGS_ON, { failSave: true });
    await renderLoaded();
    const field = await screen.findByLabelText(tx("onlineServices:discogs.tokenLabel"));
    await userEvent.type(field, TOKEN);
    await userEvent.click(screen.getByRole("button", { name: tx("onlineServices:discogs.save") }));
    expect(await screen.findByText(tx("onlineServices:discogs.outcome.failed"))).toBeVisible();
    expect(document.body.innerHTML).not.toContain(TOKEN);
  });

  it("Save can't be used and the reason shows when the computer has no password store", async () => {
    fakeBackend({ ...OFF, discogs: { on: true, tokenSaved: false, storeAvailable: false } });
    await renderLoaded();
    await screen.findByLabelText(tx("onlineServices:discogs.tokenLabel"));
    expect(screen.getByRole("button", { name: tx("onlineServices:discogs.save") })).toBeDisabled();
    expect(screen.getByText(tx("onlineServices:discogs.outcome.storeUnavailable"))).toBeVisible();
  });

  it("Remove calls remove and the status goes back to no token saved", async () => {
    const backend = fakeBackend({ ...OFF, discogs: { on: true, tokenSaved: true, storeAvailable: true } });
    await renderLoaded();
    await userEvent.click(await screen.findByRole("button", { name: tx("onlineServices:discogs.remove") }));
    expect(await screen.findByText(tx("onlineServices:discogs.removed"))).toBeVisible();
    expect(backend.removes).toBe(1);
    expect(await screen.findByText(tx("onlineServices:discogs.notSaved"))).toBeVisible();
  });

  it("Remove can't be used while no token is saved", async () => {
    fakeBackend(DISCOGS_ON);
    await renderLoaded();
    expect(await screen.findByRole("button", { name: tx("onlineServices:discogs.remove") })).toBeDisabled();
  });

  it("the token controls go away when Discogs is switched off", async () => {
    fakeBackend(DISCOGS_ON);
    await renderLoaded();
    await screen.findByLabelText(tx("onlineServices:discogs.tokenLabel"));
    await userEvent.click(switchNamed("onlineServices:discogs.label"));
    await waitFor(() => expect(screen.queryByLabelText(tx("onlineServices:discogs.tokenLabel"))).toBeNull());
  });

  it("Copy copies the token address and nothing opens a link", async () => {
    const backend = fakeBackend(DISCOGS_ON);
    const writeText = vi.fn().mockResolvedValue(undefined);
    const open = vi.fn();
    vi.stubGlobal("open", open);
    Object.defineProperty(navigator, "clipboard", { value: { writeText }, configurable: true });
    await renderLoaded();
    // The address is the owner's decision, so it is pinned in the locale file itself (the page shows a marker in the marker mode).
    const file = JSON.parse(rawMessages) as {
      discogs: { tokenUrl: string };
    };
    expect(file.discogs.tokenUrl).toBe(ADDRESS);
    const shown = tx("onlineServices:discogs.tokenUrl");
    expect(await screen.findByText(shown)).toBeVisible();
    await userEvent.click(screen.getByRole("button", { name: tx("onlineServices:discogs.copy") }));
    expect(writeText).toHaveBeenCalledWith(shown);
    expect(await screen.findByText(tx("onlineServices:discogs.copied"))).toBeVisible();
    // Only text: no link to follow, no window opened, no plugin asked to open anything.
    expect(screen.queryByRole("link")).toBeNull();
    expect(open).not.toHaveBeenCalled();
    expect(backend.commands.filter((cmd) => cmd.startsWith("plugin:"))).toEqual([]);
  });

  it("says copying failed when the clipboard refuses", async () => {
    fakeBackend(DISCOGS_ON);
    const writeText = vi.fn().mockRejectedValue(new Error("no"));
    Object.defineProperty(navigator, "clipboard", { value: { writeText }, configurable: true });
    await renderLoaded();
    await userEvent.click(await screen.findByRole("button", { name: tx("onlineServices:discogs.copy") }));
    expect(await screen.findByText(tx("onlineServices:discogs.copyFailed"))).toBeVisible();
  });

  it("each switch is tied to its own label so a screen reader names it", async () => {
    fakeBackend(OFF);
    await renderLoaded();
    const rows = screen.getAllByRole("checkbox");
    expect(rows).toHaveLength(4);
    const box = switchNamed("onlineServices:coverArt.label");
    expect(box).toHaveAccessibleDescription(tx("onlineServices:coverArt.help"));
    expect(within(box.closest("label") as HTMLElement).getByRole("checkbox")).toBe(box);
  });
});
