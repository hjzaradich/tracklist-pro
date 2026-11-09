import { QueryClientProvider } from "@tanstack/react-query";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { cleanup, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { createQueryClient } from "../app/queryClient";
import type {
  CoverAnswer,
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
import { DISCOGS_SERVICE, MUSICBRAINZ_SERVICE } from "./useIdentify";

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
/** What identify_look_up_tracks and identify_look_up_recordings answer: the job, or none. */
let job: number | null;
let lookedUpRecordings: unknown[];
/** The release ids the track's kept look-up has, and what the Cover Art Archive says for the first. */
let releases: string[];
let cover: CoverAnswer;
let coverAsks: unknown[];

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
  lookedUpRecordings = [];
  releases = [];
  cover = { kind: "noCover" };
  coverAsks = [];
  track = tableRow(1);
  useSelectedTrack.setState({ row: null, ids: [] });
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
      case "identify_look_up_recordings":
        lookedUpRecordings.push(args);
        return job;
      case "identify_cover_releases":
        return releases;
      case "identify_cover":
        coverAsks.push(args);
        return cover;
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
    it("Look up N tracks sends the ids of every selected row, rows the table never loaded included, once", async () => {
      // Ctrl+A over 5,000 rows: the screen holds a few of them, the request carries them all.
      const everyRow = Array.from({ length: 5000 }, (_, index) => 101 + index);
      useSelectedTrack.setState({ ids: everyRow });
      renderPanel();
      const region = await panel();
      await userEvent.click(await within(region).findByRole("button", { name: tx("identify:lookUp.selected", { count: 5000 }) }));
      await waitFor(() => expect(lookedUpRecordings).toEqual([{ recordings: everyRow }]));
      expect(lookedUp).toEqual([]);
      expect(await within(region).findByText(tx("identify:lookUp.asked.queued", { count: 5000 }))).toBeVisible();
    });

    it("says so when the backend finds no selected track with a file to look up", async () => {
      job = null;
      useSelectedTrack.setState({ ids: [101, 102] });
      renderPanel();
      const region = await panel();
      await userEvent.click(await within(region).findByRole("button", { name: tx("identify:lookUp.selected", { count: 2 }) }));
      expect(await within(region).findByText(tx("identify:lookUp.asked.none"))).toBeVisible();
      expect(lookedUpRecordings).toEqual([{ recordings: [101, 102] }]);
    });

    it("disables Look up N tracks with the same reason when every online service is off", async () => {
      services = ALL_OFF;
      useSelectedTrack.setState({ ids: [101, 102] });
      renderPanel();
      const region = await panel();
      const button = await within(region).findByRole("button", { name: tx("identify:lookUp.selected", { count: 2 }) });
      await waitFor(() => expect(button).toBeDisabled());
      expect(button).toHaveAccessibleDescription(tx("identify:lookUp.off"));
    });
  });

  describe("the cover", () => {
    const COVER_ONLY: OnlineServicesStatus = { ...ONE_ON, coverArt: true };
    const releaseId = "11111111-1111-1111-1111-111111111111";
    const withAlbum = () => {
      suggestions = { ...suggestions, fields: [field("Album", null, [{ value: "Some Album", sources: [ONLINE] }])] };
    };

    it("draws the picture from the data the command returns, not from a web address", async () => {
      services = COVER_ONLY;
      releases = [releaseId, "22222222-2222-2222-2222-222222222222"];
      cover = { kind: "image", mime: "image/jpeg", data: "QUJD" };
      withAlbum();
      renderPanel();
      const region = await panel();
      const picture = await within(region).findByRole("img", { name: tx("identify:lookUp.cover.altAlbum", { album: "Some Album" }) });
      expect(picture.getAttribute("src")).toBe("data:image/jpeg;base64,QUJD");
      expect(picture).toHaveAttribute("width", "96");
      expect(coverAsks).toEqual([{ releaseMbid: releaseId }]);
    });

    it("names the cover generally when the album is not known", async () => {
      services = COVER_ONLY;
      releases = [releaseId];
      cover = { kind: "image", mime: "image/png", data: "QUJD" };
      renderPanel();
      expect(await within(await panel()).findByRole("img", { name: tx("identify:lookUp.cover.alt") })).toBeVisible();
    });

    it("does not draw a picture whose type is not a picture type", async () => {
      services = COVER_ONLY;
      releases = [releaseId];
      cover = { kind: "image", mime: "text/html", data: "QUJD" };
      renderPanel();
      const region = await panel();
      await waitFor(() => expect(coverAsks).toHaveLength(1));
      expect(within(region).queryByRole("img")).toBeNull();
    });

    it("asks for no cover, and not for the releases, while Cover Art is off", async () => {
      services = ONE_ON;
      releases = [releaseId];
      renderPanel();
      const region = await panel();
      await within(region).findByText("Fixed Title");
      expect(within(region).queryByRole("img")).toBeNull();
      expect(asked).not.toContain("identify_cover");
      expect(asked).not.toContain("identify_cover_releases");
    });

    it("asks for no cover when the kept look-up has no release", async () => {
      services = COVER_ONLY;
      releases = [];
      renderPanel();
      const region = await panel();
      await within(region).findByText("Fixed Title");
      await waitFor(() => expect(asked).toContain("identify_cover_releases"));
      expect(asked).not.toContain("identify_cover");
      expect(within(region).queryByText(tx("identify:lookUp.cover.none"))).toBeNull();
    });

    it("stops saying it is looking once the cover has arrived", async () => {
      services = COVER_ONLY;
      releases = [releaseId];
      cover = { kind: "image", mime: "image/jpeg", data: "QUJD" };
      renderPanel();
      const region = await panel();
      expect(await within(region).findByRole("img")).toBeVisible();
      expect(within(region).queryByText(tx("identify:lookUp.cover.loading"))).toBeNull();
    });

    it("draws nothing when the archive has no cover", async () => {
      services = COVER_ONLY;
      releases = [releaseId];
      cover = { kind: "noCover" };
      renderPanel();
      const region = await panel();
      await waitFor(() => expect(coverAsks).toHaveLength(1));
      await waitFor(() => expect(within(region).queryByText(tx("identify:lookUp.cover.loading"))).toBeNull());
      expect(within(region).queryByRole("img")).toBeNull();
      expect(within(region).queryByText(tx("identify:lookUp.cover.none"))).toBeNull();
    });

    it("says only 'No cover found' when fetching the cover failed", async () => {
      services = COVER_ONLY;
      releases = [releaseId];
      cover = { kind: "failed", reason: "offline" };
      renderPanel();
      const region = await panel();
      expect(await within(region).findByText(tx("identify:lookUp.cover.none"))).toBeVisible();
      expect(within(region).queryByRole("alert")).toBeNull();
    });
  });

  describe("the Discogs credit", () => {
    const DISCOGS: IdentifySource = { kind: "online", service: DISCOGS_SERVICE, via: null };
    const credit = () => tx("identify:lookUp.discogsCredit");

    it("shows 'Data provided by Discogs' when a shown suggestion came from Discogs", async () => {
      suggestions = { ...suggestions, fields: [field("Label", null, [{ value: "Some Label", sources: [TAGS, DISCOGS] }])] };
      renderPanel();
      expect(await within(await panel()).findByText(credit())).toBeVisible();
    });

    it("shows no credit when no shown suggestion came from Discogs", async () => {
      suggestions = { ...suggestions, fields: [field("Label", null, [{ value: "Some Label", sources: [ONLINE, TAGS] }])] };
      renderPanel();
      const region = await panel();
      await within(region).findByText("Some Label");
      expect(within(region).queryByText(credit())).toBeNull();
    });

    it("shows the credit as plain text, with no link", async () => {
      suggestions = { ...suggestions, fields: [field("Label", null, [{ value: "Some Label", sources: [DISCOGS] }])] };
      renderPanel();
      const region = await panel();
      await within(region).findByText(credit());
      expect(within(region).queryByRole("link")).toBeNull();
    });
  });

  it("labels MusicBrainz with no service leading to it as a name search, and Discogs as Discogs", async () => {
    const musicBrainz = MUSICBRAINZ_SERVICE;
    const acoustId = "Service Two";
    suggestions = {
      ...suggestions,
      fields: [
        field("Label", null, [{ value: "By Name", sources: [{ kind: "online", service: musicBrainz, via: null }] }]),
        field("Year", null, [{ value: "2019", sources: [{ kind: "online", service: DISCOGS_SERVICE, via: null }] }]),
        field("Album", null, [{ value: "By Print", sources: [{ kind: "online", service: musicBrainz, via: acoustId }] }]),
      ],
    };
    renderPanel();
    const region = await panel();
    const row = (value: string) => within(region).getByText(value).closest("li") as HTMLElement;
    await within(region).findByText("By Name");
    expect(within(row("By Name")).getByText(tx("identify:lookUp.source.nameSearch", { service: musicBrainz }))).toBeVisible();
    expect(within(row("2019")).getByText(tx("identify:lookUp.source.online", { service: DISCOGS_SERVICE }))).toBeVisible();
    expect(within(row("By Print")).getByText(tx("identify:lookUp.source.onlineVia", { service: musicBrainz, via: acoustId }))).toBeVisible();
  });
});
