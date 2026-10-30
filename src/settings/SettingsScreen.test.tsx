import { QueryClientProvider } from "@tanstack/react-query";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { cleanup, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it } from "vitest";
import { createQueryClient } from "../app/queryClient";
import type { WouldChange } from "../bindings";
import i18n from "../i18n";
import { tx } from "../test/tx";
import { SettingsScreen } from "./SettingsScreen";

const READ_AT = "2026-10-01T12:05:00.000Z";

// A date and time as the confirm shows them.
const when = (date: string) =>
  new Intl.DateTimeFormat(i18n.language, { dateStyle: "medium", timeStyle: "short" }).format(
    new Date(date),
  );

/**
 * Stands in for the Rust side: the stored setting, what turning it on would
 * change, and every call made. `failSave` makes the next write fail.
 */
function fakeBackend({
  stored = false,
  wouldChange = { result: "counted", tracks: 12, readAt: READ_AT } as WouldChange,
  failSave = false,
}: { stored?: boolean; wouldChange?: WouldChange; failSave?: boolean } = {}) {
  const backend = { stored, wouldChange, failSave, calls: [] as string[], writes: [] as boolean[] };
  mockIPC((cmd, args) => {
    backend.calls.push(cmd);
    if (cmd === "read_energy_as_stars") return backend.stored;
    if (cmd === "energy_as_stars_would_change") return backend.wouldChange;
    if (cmd === "set_energy_as_stars") {
      if (backend.failSave) throw { kind: "database", params: {} };
      const on = (args as { on: boolean }).on;
      backend.writes.push(on);
      backend.stored = on;
      return null;
    }
    // The send state is asked again when the setting changes; nothing here needs it.
    if (cmd === "send_state") return null;
    throw new Error(`unexpected command ${cmd}`);
  });
  return backend;
}

/** The screen, with the switch once the setting has been read (until then it can't be used). */
async function renderSettings() {
  const box = renderSettingsUnread();
  await waitFor(() => expect(box).toBeEnabled());
  return box;
}

function renderSettingsUnread() {
  render(
    <QueryClientProvider client={createQueryClient()}>
      <SettingsScreen />
    </QueryClientProvider>,
  );
  return screen.getByRole("checkbox", { name: tx("settings:energyStars.label") });
}

afterEach(() => {
  cleanup();
  clearMocks();
});

describe("the Settings screen", () => {
  it("has a heading, one panel for rekordbox, and no filled button until a confirm opens", async () => {
    fakeBackend();
    await renderSettings();
    expect(screen.getByRole("heading", { level: 1, name: tx("settings:title") })).toBeInTheDocument();
    const panel = screen.getByRole("region", { name: tx("settings:rekordbox.title") });
    expect(within(panel).getAllByRole("checkbox")).toHaveLength(1);
    // No Save and no button at all: a change applies at once.
    expect(screen.queryAllByRole("button")).toEqual([]);
  });

  it("shows the energy setting off, with its help describing the switch", async () => {
    const backend = fakeBackend();
    const box = await renderSettings();
    await waitFor(() => expect(backend.calls).toContain("read_energy_as_stars"));
    expect(box).not.toBeChecked();
    expect(box).toHaveAccessibleDescription(tx("settings:energyStars.helpOff"));
  });

  it("shows the energy setting on, with the help for on", async () => {
    const backend = fakeBackend({ stored: true });
    const box = await renderSettings();
    await waitFor(() => expect(box).toBeChecked());
    expect(backend.writes).toEqual([]);
    expect(box).toHaveAccessibleDescription(tx("settings:energyStars.helpOn"));
  });
});

describe("turning the energy setting on", () => {
  it("counts first, then asks, with the count and the date of the read, and writes nothing yet", async () => {
    const backend = fakeBackend();
    const box = await renderSettings();
    await userEvent.click(box);
    const dialog = await screen.findByRole("alertdialog", {
      name: tx("settings:energyStars.confirmTitle"),
    });
    expect(backend.calls).toContain("energy_as_stars_would_change");
    expect(dialog).toHaveTextContent(
      tx("settings:energyStars.changes", { count: 12, when: when(READ_AT) }),
    );
    expect(dialog).toHaveTextContent(tx("settings:energyStars.confirmDetail"));
    // Nothing has changed until "Turn on": the box still reads off.
    expect(box).not.toBeChecked();
    expect(backend.writes).toEqual([]);
  });

  it("says so while it counts, and the switch can't be used meanwhile", async () => {
    let release: (answer: WouldChange) => void = () => {};
    const backend = fakeBackend();
    const held = new Promise<WouldChange>((resolve) => {
      release = resolve;
    });
    mockIPC((cmd) => {
      backend.calls.push(cmd);
      if (cmd === "read_energy_as_stars") return false;
      if (cmd === "energy_as_stars_would_change") return held;
      throw new Error(`unexpected command ${cmd}`);
    });
    const box = await renderSettings();
    await userEvent.click(box);
    expect(await screen.findByRole("status")).toHaveTextContent(tx("settings:energyStars.counting"));
    expect(box).toBeDisabled();
    expect(screen.queryByRole("alertdialog")).toBeNull();
    release({ result: "counted", tracks: 1, readAt: READ_AT });
    await screen.findByRole("alertdialog");
    expect(screen.queryByRole("status")).toBeNull();
  });

  it("puts the focus on Go back and makes Turn on the one filled button", async () => {
    fakeBackend();
    await userEvent.click(await renderSettings());
    const dialog = await screen.findByRole("alertdialog");
    const goBack = within(dialog).getByRole("button", { name: tx("settings:energyStars.back") });
    await waitFor(() => expect(goBack).toHaveFocus());
    expect(within(dialog).getAllByRole("button")).toHaveLength(2);
    expect(within(dialog).getByRole("button", { name: tx("settings:energyStars.confirm") })).toBeEnabled();
  });

  it("names one track, several, and none, each with its own line", async () => {
    for (const [tracks, line] of [
      [1, tx("settings:energyStars.changes", { count: 1, when: when(READ_AT) })],
      [1234, tx("settings:energyStars.changes", { count: 1234, when: when(READ_AT) })],
      [0, tx("settings:energyStars.changes", { count: 0, when: when(READ_AT) })],
    ] as const) {
      fakeBackend({ wouldChange: { result: "counted", tracks, readAt: READ_AT } });
      await userEvent.click(await renderSettings());
      expect(await screen.findByRole("alertdialog")).toHaveTextContent(line);
      cleanup();
      clearMocks();
    }
    // The three lines are three different texts.
    expect(
      new Set([1, 1234, 0].map((count) => tx("settings:energyStars.changes", { count, when: "x" }))).size,
    ).toBe(3);
  });

  it("says there is nothing to count yet when rekordbox's export was never read, and still asks", async () => {
    fakeBackend({ wouldChange: { result: "noRead" } });
    await userEvent.click(await renderSettings());
    const dialog = await screen.findByRole("alertdialog");
    expect(dialog).toHaveTextContent(tx("settings:energyStars.noRead"));
    expect(
      within(dialog).getByRole("button", { name: tx("settings:energyStars.confirm") }),
    ).toBeInTheDocument();
  });

  it("leaves it off and gives the focus back to the switch on Go back", async () => {
    const backend = fakeBackend();
    const box = await renderSettings();
    await userEvent.click(box);
    await userEvent.click(
      await screen.findByRole("button", { name: tx("settings:energyStars.back") }),
    );
    expect(screen.queryByRole("alertdialog")).toBeNull();
    expect(box).not.toBeChecked();
    expect(box).toHaveFocus();
    expect(backend.writes).toEqual([]);
  });

  it("leaves it off and gives the focus back to the switch on Esc", async () => {
    const backend = fakeBackend();
    const box = await renderSettings();
    await userEvent.click(box);
    const goBack = await screen.findByRole("button", { name: tx("settings:energyStars.back") });
    await waitFor(() => expect(goBack).toHaveFocus());
    await userEvent.keyboard("{Escape}");
    expect(screen.queryByRole("alertdialog")).toBeNull();
    expect(box).not.toBeChecked();
    expect(box).toHaveFocus();
    expect(backend.writes).toEqual([]);
  });

  it("turns it on when Turn on is chosen, and shows the help for on", async () => {
    const backend = fakeBackend();
    const box = await renderSettings();
    await userEvent.click(box);
    await userEvent.click(
      await screen.findByRole("button", { name: tx("settings:energyStars.confirm") }),
    );
    await waitFor(() => expect(box).toBeChecked());
    expect(backend.writes).toEqual([true]);
    expect(screen.queryByRole("alertdialog")).toBeNull();
    expect(box).toHaveAccessibleDescription(tx("settings:energyStars.helpOn"));
    expect(box).toHaveFocus();
  });

  it("can be done with the keyboard alone", async () => {
    const backend = fakeBackend();
    const box = await renderSettings();
    box.focus();
    await userEvent.keyboard(" ");
    const goBack = await screen.findByRole("button", { name: tx("settings:energyStars.back") });
    await waitFor(() => expect(goBack).toHaveFocus());
    // Shift+Tab reaches Turn on; Enter chooses it.
    await userEvent.tab({ shift: true });
    expect(screen.getByRole("button", { name: tx("settings:energyStars.confirm") })).toHaveFocus();
    await userEvent.keyboard("{Enter}");
    await waitFor(() => expect(box).toBeChecked());
    expect(backend.writes).toEqual([true]);
  });
});

describe("before the setting is known", () => {
  it("is disabled and shows neither state while the setting is being read, and a click starts nothing", async () => {
    let answer: (on: boolean) => void = () => {};
    const stored = new Promise<boolean>((resolve) => {
      answer = resolve;
    });
    const calls: string[] = [];
    mockIPC((cmd) => {
      calls.push(cmd);
      if (cmd === "read_energy_as_stars") return stored;
      throw new Error(`unexpected command ${cmd}`);
    });
    const box = renderSettingsUnread();
    expect(box).toBeDisabled();
    expect(box).toBePartiallyChecked();
    expect(screen.queryByText(tx("settings:energyStars.helpOff"))).toBeNull();
    await userEvent.click(box);
    expect(calls).not.toContain("energy_as_stars_would_change");
    expect(screen.queryByRole("alertdialog")).toBeNull();
    // Once it is read as on, it shows on and offers no turn-on.
    answer(true);
    await waitFor(() => expect(box).toBeEnabled());
    expect(box).toBeChecked();
    expect(box).not.toBePartiallyChecked();
    expect(box).toHaveAccessibleDescription(tx("settings:energyStars.helpOn"));
  });

  it("stays disabled and shows an alert when the setting can't be read, and a click starts nothing", async () => {
    const calls: string[] = [];
    mockIPC((cmd) => {
      calls.push(cmd);
      if (cmd === "read_energy_as_stars") throw { kind: "database", params: {} };
      throw new Error(`unexpected command ${cmd}`);
    });
    const box = renderSettingsUnread();
    expect(await screen.findByRole("alert")).toHaveTextContent(tx("errors:database"));
    expect(box).toBeDisabled();
    await userEvent.click(box);
    expect(calls).not.toContain("energy_as_stars_would_change");
    expect(calls).not.toContain("set_energy_as_stars");
    expect(screen.queryByRole("alertdialog")).toBeNull();
  });
});

describe("when the tracks can't be counted", () => {
  it("shows its own alert, not the one about saving, and leaves the switch off and usable", async () => {
    const backend = fakeBackend();
    const box = await renderSettings();
    mockIPC((cmd) => {
      backend.calls.push(cmd);
      if (cmd === "read_energy_as_stars") return false;
      if (cmd === "energy_as_stars_would_change") throw { kind: "database", params: {} };
      throw new Error(`unexpected command ${cmd}`);
    });
    await userEvent.click(box);
    expect(await screen.findByRole("alert")).toHaveTextContent(tx("settings:energyStars.countError"));
    expect(screen.queryByText(tx("settings:energyStars.error"))).toBeNull();
    expect(box).not.toBeChecked();
    expect(box).toBeEnabled();
    expect(screen.queryByRole("alertdialog")).toBeNull();
  });
});

describe("while Turn on is saving", () => {
  it("ignores Esc: the panel stays until the result lands", async () => {
    let finish: () => void = () => {};
    const saving = new Promise<null>((resolve) => {
      finish = () => resolve(null);
    });
    mockIPC((cmd) => {
      if (cmd === "read_energy_as_stars") return false;
      if (cmd === "energy_as_stars_would_change") return { result: "noRead" };
      if (cmd === "set_energy_as_stars") return saving;
      if (cmd === "send_state") return null;
      throw new Error(`unexpected command ${cmd}`);
    });
    const box = await renderSettings();
    await userEvent.click(box);
    const turnOn = await screen.findByRole("button", { name: tx("settings:energyStars.confirm") });
    await userEvent.click(turnOn);
    await waitFor(() => expect(turnOn).toBeDisabled());
    const back = screen.getByRole("button", { name: tx("settings:energyStars.back") });
    back.focus();
    await userEvent.keyboard("{Escape}");
    expect(screen.getByRole("alertdialog")).toBeInTheDocument();
    finish();
    await waitFor(() => expect(screen.queryByRole("alertdialog")).toBeNull());
    expect(box).toBeChecked();
  });
});

describe("turning the energy setting off", () => {
  it("doesn't ask, writes at once, and says what is left behind", async () => {
    const backend = fakeBackend({ stored: true });
    const box = await renderSettings();
    await waitFor(() => expect(box).toBeChecked());
    await userEvent.click(box);
    await waitFor(() => expect(box).not.toBeChecked());
    expect(backend.writes).toEqual([false]);
    expect(screen.queryByRole("alertdialog")).toBeNull();
    // Counting is for turning on only.
    expect(backend.calls).not.toContain("energy_as_stars_would_change");
    expect(box).toHaveAccessibleDescription(
      `${tx("settings:energyStars.helpOff")} ${tx("settings:energyStars.offNote")}`,
    );
  });
});

describe("when the setting can't be saved", () => {
  it("keeps it off after Turn on fails, and shows an alert", async () => {
    const backend = fakeBackend({ failSave: true });
    const box = await renderSettings();
    await userEvent.click(box);
    await userEvent.click(
      await screen.findByRole("button", { name: tx("settings:energyStars.confirm") }),
    );
    expect(await screen.findByRole("alert")).toHaveTextContent(tx("settings:energyStars.error"));
    expect(box).not.toBeChecked();
    expect(box).toBeEnabled();
    expect(backend.stored).toBe(false);
    expect(screen.queryByRole("alertdialog")).toBeNull();
  });

  it("keeps it on after turning off fails, and shows an alert", async () => {
    const backend = fakeBackend({ stored: true });
    const box = await renderSettings();
    await waitFor(() => expect(box).toBeChecked());
    backend.failSave = true;
    await userEvent.click(box);
    expect(await screen.findByRole("alert")).toHaveTextContent(tx("settings:energyStars.error"));
    expect(box).toBeChecked();
    expect(box).toBeEnabled();
    expect(box).toHaveAccessibleDescription(tx("settings:energyStars.helpOn"));
  });

  it("disables the row while the change is saved", async () => {
    let finish: () => void = () => {};
    const saving = new Promise<null>((resolve) => {
      finish = () => resolve(null);
    });
    mockIPC((cmd) => {
      if (cmd === "read_energy_as_stars") return true;
      if (cmd === "set_energy_as_stars") return saving;
      if (cmd === "send_state") return null;
      throw new Error(`unexpected command ${cmd}`);
    });
    const box = await renderSettings();
    await waitFor(() => expect(box).toBeChecked());
    await userEvent.click(box);
    await waitFor(() => expect(box).toBeDisabled());
    // The box shows where it is going while it saves.
    expect(box).not.toBeChecked();
    finish();
    await waitFor(() => expect(box).toBeEnabled());
    expect(box).not.toBeChecked();
  });
});
