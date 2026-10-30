import { mockIPC } from "@tauri-apps/api/mocks";
import type { Overview, Preflight, ReviewCounts, SendState } from "../bindings";

// A stand-in for the Rust side of the Overview screen, shared by its tests:
// the Library, the Overview command's numbers, Review's counts, the send's
// state and the commands the panels below the numbers ask for.

export const NOTHING_WAITING: Overview = {
  tracks: 3,
  totalLengthMs: 600_000,
  inCrates: 3,
  inNoCrate: 0,
  analyzedByRekordbox: 3,
  notAnalyzed: 0,
  rekordboxRead: true,
  lowQuality: 0,
  lastSentMs: Date.UTC(2026, 9, 20, 12, 0),
  allMusic: 3,
  addedThisWeek: 0,
  decisionsThisWeek: 0,
  genres: [],
  noGenre: 0,
  energy: [0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
  noEnergy: 3,
  outsideLibrary: 0,
  missingTags: 0,
  missingArt: 0,
  counting: { files: false, rekordbox: false, quality: false },
};

export const NO_REVIEW: ReviewCounts = {
  needsCloserLook: 0,
  clearBestFile: 0,
  qualityProblems: 0,
  conflicts: 0,
  total: 0,
};


export function preflight(over: Partial<Preflight>): Preflight {
  return {
    token: "t1",
    export: { path: String.raw`C:\export.xml`, modifiedMs: 0, readAt: "2026-10-27T10:00:00.000Z", notStored: 0 },
    newTracks: 0,
    knownTracks: 0,
    leftOut: [],
    fileMissing: [],
    noFileAtLocation: [],
    otherFile: [],
    noKind: [],
    losesEntries: [],
    conflictsWaiting: 0,
    refusal: null,
    nothingToSend: false,
    canSend: true,
    exportOlderThanLastSend: false,
    needsConfirm: false,
    ...over,
  } as Preflight;
}

/** What the fake Rust side answers; a test may change it between answers. */
export interface World {
  library?: number;
  /**
   * The Library's change stamp. By default it is the track count, so adding a
   * track moves it; set it to retitle a track and leave the count alone.
   */
  libraryStamp?: number;
  /** The Library can't be read. */
  libraryBroken?: boolean;
  overview?: Partial<Overview>;
  /** The Overview command fails. */
  overviewBroken?: boolean;
  /** The Overview command doesn't answer until `release()`. */
  overviewHeld?: boolean;
  review?: Partial<ReviewCounts>;
  preflight?: Preflight | null;
  /** How many rekordbox tracks the offer could add to the Library. */
  rekordboxToAdd?: number;
  /** A newer rekordbox export is offered. */
  newerExport?: boolean;
}

/** Stands in for the Rust side, and keeps the names of the commands asked. */
export function fakeBackend(world: World = {}) {
  const calls: string[] = [];
  let release = () => {};
  const gate = new Promise<void>((resolve) => {
    release = resolve;
  });
  mockIPC(
    async (cmd) => {
      calls.push(cmd);
      switch (cmd) {
        case "library_summary":
          if (world.libraryBroken) throw { kind: "database", params: {} };
          return { tracks: world.library ?? 3, changeStamp: world.libraryStamp ?? world.library ?? 3 };
        case "overview":
          if (world.overviewHeld) await gate;
          if (world.overviewBroken) throw { kind: "database", params: {} };
          return { ...NOTHING_WAITING, ...world.overview };
        case "review_counts":
          return { ...NO_REVIEW, ...world.review };
        case "send_state": {
          const state: SendState = {
            revision: 0,
            filePath: String.raw`C:\data\send.xml`,
            exportPath: null,
            preflight: world.preflight ?? null,
            step: null,
            failure: null,
            sent: null,
          };
          return state;
        }
        case "rekordbox_offer":
          return {
            toAdd: world.rekordboxToAdd ?? 0,
            alreadyInLibrary: 0,
            waitingInMissing: 0,
            waitingForConfirmation: 0,
          };
        case "after_send_lists":
          return { playlistsChecked: true, stalePlaylists: [], manualRemovals: [] };
        case "rekordbox_xml_source":
          return {
            path: "C:/export.xml",
            watch: true,
            lastRead: null,
            lastFailure: null,
            newerExport: world.newerExport
              ? { path: "C:/export 2.xml", name: "export 2.xml", modifiedMs: 0 }
              : null,
            exportFolder: null,
          };
        case "music_folders":
          return [];
        case "read_online_only_files":
          return false;
        case "key_notation":
          return "camelot";
        case "table_columns":
          return null;
        case "track_table_page":
          return { total: 0, offset: 0, rows: [], behind: false };
        case "list_crates":
          return [];
        case "missing_tracks":
          return { total: 0, groups: [] };
        case "review_conflict_fields":
          return [];
        case "review_items":
          return { total: 0, items: [] };
        default:
          // The shell's own commands (Activity) aren't this test's business.
          throw { kind: "internal", params: {} };
      }
    },
    { shouldMockEvents: true },
  );
  return { calls, release };
}
