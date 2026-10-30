import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { cleanup, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it } from "vitest";
import { renderApp } from "../app/testApp";
import type { SendState } from "../bindings";
import { NOTHING_WAITING, NO_REVIEW, preflight } from "../test/overviewBackend";
import { tx } from "../test/tx";

afterEach(() => {
  cleanup();
  clearMocks();
});

/**
 * The Overview, the guided send and Settings, with a setting and a send
 * state that behave like the real ones: Read export builds a review from the
 * setting as it is then, and the review stays held until the next one.
 */
function fakeApp(starsOn: boolean) {
  const backend = {
    starsOn,
    prepares: 0,
    revision: 0,
    held: null as SendState["preflight"],
  };
  mockIPC(
    (cmd, args) => {
      switch (cmd) {
        case "library_summary":
          return { tracks: 3, changeStamp: 3 };
        case "overview":
          return NOTHING_WAITING;
        case "review_counts":
          return NO_REVIEW;
        case "send_state":
          return {
            revision: backend.revision,
            filePath: String.raw`C:\data\send.xml`,
            exportPath: String.raw`C:\export.xml`,
            preflight: backend.held,
            step: null,
            failure: null,
            sent: null,
          } satisfies SendState;
        case "prepare_send":
          backend.prepares += 1;
          backend.revision += 1;
          backend.held = preflight({
            token: `t${backend.prepares}`,
            newTracks: 3,
            starsChanged: backend.starsOn ? 4 : null,
            changedAfterExport: [],
          });
          return 7;
        case "read_energy_as_stars":
          return backend.starsOn;
        case "set_energy_as_stars":
          backend.starsOn = (args as { on: boolean }).on;
          return null;
        case "rekordbox_offer":
          return { toAdd: 0, alreadyInLibrary: 0, waitingInMissing: 0, waitingForConfirmation: 0 };
        case "after_send_lists":
          return { playlistsChecked: true, stalePlaylists: [], manualRemovals: [] };
        case "rekordbox_xml_source":
          return {
            path: String.raw`C:\export.xml`,
            watch: false,
            lastRead: null,
            lastFailure: null,
            newerExport: null,
            exportFolder: null,
          };
        case "music_folders":
          return [];
        case "list_crates":
          return [];
        default:
          throw { kind: "internal", params: {} };
      }
    },
    { shouldMockEvents: true },
  );
  return backend;
}

describe("changing the setting from the send review", () => {
  it("loses the open review, and the next Send reads again and shows the new state", async () => {
    const backend = fakeApp(true);
    const { router } = renderApp("/overview");

    // A review made while energy is sent as stars: the echo and the count.
    await userEvent.click(await screen.findByRole("button", { name: tx("send:open") }));
    await userEvent.click(await screen.findByRole("button", { name: tx("send:export.read") }));
    const review = () =>
      within(screen.getByRole("heading", { level: 3, name: tx("send:review.title") }).closest("li") as HTMLElement);
    expect(await review().findByText(tx("send:review.starsChange", { count: 4 }))).toBeInTheDocument();
    expect(backend.prepares).toBe(1);

    // The link leaves the review: Settings, with the setting on.
    await userEvent.click(review().getByRole("link", { name: tx("send:review.starsLink") }));
    const box = await screen.findByRole("checkbox", { name: tx("settings:energyStars.label") });
    await waitFor(() => expect(box).toBeChecked());
    expect(router.state.location.pathname).toBe("/settings");

    // Turning it off asks nothing and applies at once.
    await userEvent.click(box);
    await waitFor(() => expect(box).not.toBeChecked());
    expect(backend.starsOn).toBe(false);

    // Back on the Overview the send is closed, as after any navigation.
    await userEvent.click(screen.getByRole("link", { name: tx("shell:stages.overview") }));
    await userEvent.click(await screen.findByRole("button", { name: tx("send:open") }));
    await userEvent.click(await screen.findByRole("button", { name: tx("send:export.read") }));

    // The new review is a new prepare, made under the new setting: no echo, no count.
    await waitFor(() => expect(backend.prepares).toBe(2));
    await waitFor(() =>
      expect(review().queryByRole("link", { name: tx("send:review.starsLink") })).toBeNull(),
    );
    expect(await review().findByText(tx("send:review.new", { count: 3 }))).toBeInTheDocument();
    for (const count of [0, 1, 4]) {
      expect(review().queryByText(tx("send:review.starsChange", { count }))).toBeNull();
    }
  });
});
