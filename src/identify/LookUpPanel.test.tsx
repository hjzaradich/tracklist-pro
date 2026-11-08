import { QueryClientProvider } from "@tanstack/react-query";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { cleanup, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { createQueryClient } from "../app/queryClient";
import type {
  IdentifyField,
  IdentifyFieldSuggestions,
  IdentifyLookupStatus,
  IdentifySource,
  IdentifySuggestions,
  OnlineServicesStatus,
} from "../bindings";
import "../i18n";
import { useSelectedTrack } from "../shell/selectedTrack";
import { tableRow } from "../test/trackTableIpc";
import { tx } from "../test/tx";
import { LookUpPanel } from "./LookUpPanel";

let track = tableRow(1);

const FILE = { id: 11, name: "a.mp3" };
const TAGS: IdentifySource = { kind: "tags", file: FILE };
const NAME: IdentifySource = { kind: "fileName", file: FILE };
const SIBLING: IdentifySource = { kind: "sibling", file: { id: 12, name: "b.mp3" } };
const ONLINE: IdentifySource = { kind: "online", service: "Service One", via: "Service Two" };

const field = (
  name: IdentifyField,
  current: string | null,
  suggestions: IdentifyFieldSuggestions["suggestions"],
): IdentifyFieldSuggestions => ({ field: name, current, suggestions });
const NO_CATALOGUE = field("CatalogueNumber", null, []);

const ALL_OFF: OnlineServicesStatus = {
  musicBrainz: false,
  acoustId: { on: false, keyMissing: false },
  discogs: { on: false, tokenSaved: false, storeAvailable: true },
  coverArt: false,
};
const ONE_ON: OnlineServicesStatus = { ...ALL_OFF, musicBrainz: true };

const STATUS: IdentifyLookupStatus = {
  libraryTrackId: 1,
  state: "never",
  reason: null,
  found: 0,
  when: null,
  laterAttempt: null,
};

let suggestions: IdentifySuggestions;
let status: IdentifyLookupStatus;
let services: OnlineServicesStatus;
/** When set, identify_use refuses with this error. */
let refusal: unknown;
let asked: string[];
let used: unknown[];
let lookedUp: unknown[];
/** What identify_look_up_tracks answers: the job, or none. */
let job: number | null;

function renderPanel() {
  const client = createQueryClient();
  render(
    <QueryClientProvider client={client}>
      <LookUpPanel row={track} />
    </QueryClientProvider>,
  );
  return client;
}

const panel = () => screen.findByRole("region", { name: tx("identify:lookUp.title") });

beforeEach(() => {
  suggestions = {
    fields: [
      field("Name", "Old Title", [
        { value: "Fixed Title", sources: [TAGS] },
        { value: "Other Title", sources: [NAME] },
      ]),
      field("Artist", null, [{ value: "Some Artist", sources: [TAGS, SIBLING] }]),
      field("Label", null, []),
    ],
    catalogueNumber: NO_CATALOGUE,
  };
  status = STATUS;
  services = ONE_ON;
  refusal = null;
  asked = [];
  used = [];
  lookedUp = [];
  job = 5;
  track = tableRow(1);
  useSelectedTrack.setState({ row: null, ids: [], libraryIdsOf: null });
  mockIPC((cmd, args) => {
    asked.push(cmd);
    switch (cmd) {
      case "identify_suggestions":
        return suggestions;
      case "identify_lookup_status":
        return [status];
      case "online_services_status":
        return services;
      case "identify_use":
        if (refusal !== null) throw refusal;
        used.push(args);
        return 9;
      case "identify_look_up_tracks":
        lookedUp.push(args);
        return job;
      default:
        // Events and anything else the page asks for: quiet.
        throw new Error(`unexpected command ${cmd}`);
    }
  });
});

afterEach(() => {
  cleanup();
  clearMocks();
});

describe("LookUpPanel", () => {
  describe("the status line", () => {
    const WHEN = "2026-11-08T10:00:00Z";
    const when = new Date(WHEN).toLocaleString();
    const cases: [string, Partial<IdentifyLookupStatus>, () => string][] = [
      ["never looked up", {}, () => tx("identify:lookUp.status.never")],
      ["looking up while queued", { state: "queued" }, () => tx("identify:lookUp.status.queued")],
      ["looking up while running", { state: "running" }, () => tx("identify:lookUp.status.running")],
      [
        "found, with its source and when",
        { state: "found", found: 2, when: WHEN },
        () => tx("identify:lookUp.status.foundFrom", { count: 2, source: tx("identify:lookUp.source.onlineVia", { service: "Service One", via: "Service Two" }), when }),
      ],
      [
        "nothing found, with its reason",
        { state: "nothing", reason: "no_match", when: WHEN },
        () => tx("identify:lookUp.status.nothingWhy", { when, reason: tx("identify:lookUp.reason.no_match") }),
      ],
      [
        "failed, with its reason",
        { state: "failed", reason: "offline", when: WHEN },
        () => tx("identify:lookUp.status.failedWhy", { when, reason: tx("identify:lookUp.reason.offline") }),
      ],
      [
        "skipped for a service that is not on, naming where to turn it on",
        { state: "skipped", reason: "acoustid_off" },
        () => tx("identify:lookUp.status.skippedWhy", { reason: tx("identify:lookUp.reason.acoustid_off") }),
      ],
      [
        "skipped because the app's key for a service is missing",
        { state: "skipped", reason: "key_missing" },
        () => tx("identify:lookUp.status.skippedWhy", { reason: tx("identify:lookUp.reason.key_missing") }),
      ],
      [
        "failed for an unknown reason, showing the code it came with",
        { state: "failed", reason: "from_the_future", when: WHEN },
        () => tx("identify:lookUp.status.failedWhy", { when, reason: tx("identify:lookUp.reason.other", { code: "from_the_future" }) }),
      ],
    ];
    it.each(cases)("says %s", async (_name, change, line) => {
      status = { ...STATUS, ...change };
      if (change.state === "found") {
        suggestions = { ...suggestions, fields: [field("Name", null, [{ value: "Online Title", sources: [ONLINE] }])] };
      }
      renderPanel();
      const region = await panel();
      expect(await within(region).findByText(line())).toBeVisible();
    });

    it("offers Look up again once the track has been looked up", async () => {
      status = { ...STATUS, state: "nothing", reason: "no_match", when: WHEN };
      renderPanel();
      const region = await panel();
      expect(await within(region).findByRole("button", { name: tx("identify:lookUp.again") })).toBeEnabled();
    });
  });

  it("asks the backend to look up this track when Look up is pressed, and says it started", async () => {
    renderPanel();
    const region = await panel();
    await userEvent.click(await within(region).findByRole("button", { name: tx("identify:lookUp.action") }));
    await waitFor(() => expect(lookedUp).toEqual([{ libraryTrackIds: [track.libraryTrackId] }]));
    expect(await within(region).findByText(tx("identify:lookUp.asked.queued", { count: 1 }))).toBeVisible();
  });

  it("disables Look up with the reason when every online service is off, and still shows the offline suggestions", async () => {
    services = ALL_OFF;
    renderPanel();
    const region = await panel();
    const button = await within(region).findByRole("button", { name: tx("identify:lookUp.action") });
    await waitFor(() => expect(button).toBeDisabled());
    expect(within(region).getByText(tx("identify:lookUp.off"))).toBeVisible();
    expect(button).toHaveAccessibleDescription(tx("identify:lookUp.off"));
    expect(await within(region).findByText("Fixed Title")).toBeVisible();
    await userEvent.click(button);
    expect(lookedUp).toEqual([]);
  });

  it("draws one row per field that has suggestions, and none for a field without", async () => {
    renderPanel();
    const region = await panel();
    expect(await within(region).findByText(tx("identify:lookUp.field.Name"))).toBeVisible();
    expect(within(region).getByText(tx("identify:lookUp.field.Artist"))).toBeVisible();
    expect(within(region).queryByText(tx("identify:lookUp.field.Label"))).toBeNull();
    expect(within(region).queryByText(tx("identify:lookUp.field.Album"))).toBeNull();
  });

  it("shows the current value beside its suggestions, or says there is none", async () => {
    renderPanel();
    const region = await panel();
    expect(await within(region).findByText(tx("identify:lookUp.current", { value: "Old Title" }))).toBeVisible();
    expect(within(region).getByText(tx("identify:lookUp.currentNone"))).toBeVisible();
  });

  it("says there is nothing to suggest, in one line, when no field has a suggestion", async () => {
    suggestions = { fields: [field("Name", null, [])], catalogueNumber: NO_CATALOGUE };
    renderPanel();
    const region = await panel();
    expect(await within(region).findByText(tx("identify:lookUp.nothing"))).toBeVisible();
    expect(within(region).queryByRole("button", { name: tx("identify:lookUp.useAll") })).toBeNull();
    expect(within(region).queryByRole("list")).toBeNull();
  });

  it("names every source in plain words, each once, and renders an unknown one from its data", async () => {
    suggestions = {
      fields: [
        field("Name", null, [{ value: "From Tags", sources: [TAGS, { kind: "tags", file: { id: 99, name: "z.mp3" } }] }]),
        field("Artist", null, [{ value: "From Name", sources: [NAME] }]),
        field("Album", null, [{ value: "From Sibling", sources: [SIBLING] }]),
        field("Label", null, [{ value: "From Service", sources: [ONLINE, { kind: "online", service: "Service Three", via: null }] }]),
        field("Year", null, [{ value: "2020", sources: [{ kind: "carrierPigeon" } as unknown as IdentifySource] }]),
      ],
      catalogueNumber: NO_CATALOGUE,
    };
    renderPanel();
    const region = await panel();
    const row = (value: string) => within(region).getByText(value).closest("li") as HTMLElement;
    await within(region).findByText("From Tags");
    expect(within(row("From Tags")).getByText(tx("identify:lookUp.source.tags"))).toBeVisible();
    expect(within(row("From Name")).getByText(tx("identify:lookUp.source.fileName"))).toBeVisible();
    expect(within(row("From Sibling")).getByText(tx("identify:lookUp.source.sibling"))).toBeVisible();
    expect(
      within(row("From Service")).getByText(
        [tx("identify:lookUp.source.onlineVia", { service: "Service One", via: "Service Two" }), tx("identify:lookUp.source.online", { service: "Service Three" })].join(
          tx("identify:lookUp.sourceSeparator"),
        ),
      ),
    ).toBeVisible();
    expect(within(row("2020")).getByText(tx("identify:lookUp.source.other", { kind: "carrierPigeon" }))).toBeVisible();
  });

  it("Use sends exactly that one field and value, in one call, and says what it did", async () => {
    renderPanel();
    const region = await panel();
    await userEvent.click(
      await within(region).findByRole("button", {
        name: tx("identify:lookUp.useValue", { value: "Other Title", field: tx("identify:lookUp.field.Name") }),
      }),
    );
    await waitFor(() => expect(used).toEqual([{ libraryTrackId: track.libraryTrackId, choices: [{ field: "Name", value: "Other Title" }] }]));
    expect(await within(region).findByText(tx("identify:lookUp.used", { count: 1 }))).toBeVisible();
  });

  it("asks for the suggestions again after Use, so a value now in use is no longer offered", async () => {
    renderPanel();
    const region = await panel();
    await within(region).findByText("Some Artist");
    suggestions = { ...suggestions, fields: [field("Name", "Fixed Title", [{ value: "Other Title", sources: [NAME] }])] };
    await userEvent.click(
      within(region).getByRole("button", {
        name: tx("identify:lookUp.useValue", { value: "Fixed Title", field: tx("identify:lookUp.field.Name") }),
      }),
    );
    await waitFor(() => expect(within(region).queryByText("Some Artist")).toBeNull());
    expect(within(region).getByText(tx("identify:lookUp.current", { value: "Fixed Title" }))).toBeVisible();
  });

  it("Use all sends the first suggestion of every shown field in one call", async () => {
    suggestions = { ...suggestions, catalogueNumber: field("CatalogueNumber", null, [{ value: "CAT001", sources: [TAGS] }]) };
    renderPanel();
    const region = await panel();
    await userEvent.click(await within(region).findByRole("button", { name: tx("identify:lookUp.useAll") }));
    await waitFor(() =>
      expect(used).toEqual([
        {
          libraryTrackId: track.libraryTrackId,
          choices: [
            { field: "Name", value: "Fixed Title" },
            { field: "Artist", value: "Some Artist" },
            { field: "CatalogueNumber", value: "CAT001" },
          ],
        },
      ]),
    );
    expect(await within(region).findByText(tx("identify:lookUp.used", { count: 3 }))).toBeVisible();
  });

  it("shows the catalogue number with Use and a note that rekordbox doesn't get it", async () => {
    suggestions = { fields: [], catalogueNumber: field("CatalogueNumber", null, [{ value: "CAT001", sources: [NAME] }]) };
    renderPanel();
    const region = await panel();
    await userEvent.click(
      await within(region).findByRole("button", {
        name: tx("identify:lookUp.useValue", { value: "CAT001", field: tx("identify:lookUp.field.CatalogueNumber") }),
      }),
    );
    expect(within(region).getByText(tx("identify:lookUp.catalogueNote"))).toBeVisible();
    await waitFor(() => expect(used).toEqual([{ libraryTrackId: track.libraryTrackId, choices: [{ field: "CatalogueNumber", value: "CAT001" }] }]));
  });

  it("shows a refusal from Use in plain words", async () => {
    refusal = { kind: "identifyValueNotSendable", params: {} };
    renderPanel();
    const region = await panel();
    await userEvent.click(await within(region).findByRole("button", { name: tx("identify:lookUp.useAll") }));
    expect(await within(region).findByRole("alert")).toHaveTextContent(tx("identify:error.notSendable"));
  });

  it("draws nothing for a track outside the Library", async () => {
    track = tableRow(1, { libraryTrackId: null, inLibrary: false });
    renderPanel();
    expect(screen.queryByRole("region", { name: tx("identify:lookUp.title") })).toBeNull();
    expect(asked).toEqual([]);
  });

  describe("with several tracks selected", () => {
    it("Look up N tracks asks for the selected Library tracks, once", async () => {
      useSelectedTrack.setState({
        ids: [101, 102, 103],
        libraryIdsOf: (ids) => ids.map((id) => id - 100),
      });
      renderPanel();
      const region = await panel();
      await userEvent.click(await within(region).findByRole("button", { name: tx("identify:lookUp.selected", { count: 3 }) }));
      await waitFor(() => expect(lookedUp).toEqual([{ libraryTrackIds: [1, 2, 3] }]));
      expect(await within(region).findByText(tx("identify:lookUp.asked.queued", { count: 3 }))).toBeVisible();
    });

    it("says how many were left out when some selected tracks aren't Library tracks the table holds", async () => {
      useSelectedTrack.setState({ ids: [101, 102, 103], libraryIdsOf: () => [1] });
      renderPanel();
      const region = await panel();
      await userEvent.click(await within(region).findByRole("button", { name: tx("identify:lookUp.selected", { count: 3 }) }));
      expect(await within(region).findByText(tx("identify:lookUp.asked.someLeftOut", { count: 1, left: 2 }))).toBeVisible();
      expect(lookedUp).toEqual([{ libraryTrackIds: [1] }]);
    });

    it("sends nothing and says so when none of the selected tracks is a Library track", async () => {
      useSelectedTrack.setState({ ids: [101, 102], libraryIdsOf: () => [] });
      renderPanel();
      const region = await panel();
      await userEvent.click(await within(region).findByRole("button", { name: tx("identify:lookUp.selected", { count: 2 }) }));
      expect(await within(region).findByText(tx("identify:lookUp.asked.none"))).toBeVisible();
      expect(lookedUp).toEqual([]);
    });

    it("disables Look up N tracks with the same reason when every online service is off", async () => {
      services = ALL_OFF;
      useSelectedTrack.setState({ ids: [101, 102], libraryIdsOf: (ids) => [...ids] });
      renderPanel();
      const region = await panel();
      const button = await within(region).findByRole("button", { name: tx("identify:lookUp.selected", { count: 2 }) });
      await waitFor(() => expect(button).toBeDisabled());
      expect(button).toHaveAccessibleDescription(tx("identify:lookUp.off"));
    });
  });
});
