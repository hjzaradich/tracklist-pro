import type { CrateSummary, SendState } from "../bindings";

/**
 * The commands the app shell, the sidebar and the screens ask for on their
 * own, whatever a test is about, with the answer of an app that has nothing
 * to show: no tracks, nothing to review, nothing sent.
 *
 * A test's own fake Rust side answers the commands it is about, then asks
 * this for the rest, instead of throwing "unexpected command": the error was
 * logged by the app for every such command and every test that did not care
 * (hundreds of lines in the CI log). A command that is not here still
 * throws, so a test that needs one says so.
 *
 * ```ts
 * mockIPC((cmd, args) => {
 *   if (cmd === "list_crates") return list;
 *   return quietAnswer(cmd);
 * });
 * ```
 */

const EMPTY_SUMMARY: CrateSummary = {
  notes: null,
  tracks: 0,
  bpm: null,
  energy: null,
  keys: [],
  genres: [],
};

const IDLE_SEND: SendState = {
  revision: 0,
  filePath: String.raw`C:\data\send.xml`,
  exportPath: null,
  preflight: null,
  step: null,
  workingOn: null,
  failure: null,
  sent: null,
};

const NEXT_SETTINGS = { weights: { key: 4, bpm: 4, energy: 2, played: 2, genre: 0, tags: 1, similar: 1 }, tolerance: "tight" };

const ANSWERS: Record<string, unknown> = {
  list_crates: [],
  list_playlists: [],
  rekordbox_playlists_to_copy: [],
  list_tags: [],
  tags_of_track: [],
  crate_summary: EMPTY_SUMMARY,
  library_summary: { tracks: 0, changeStamp: 0 },
  next_undo_operation: { operation: null, refusal: null },
  inbox_items: [],
  inbox_done_for_you: [],
  inbox_count: {
    new: 0,
    alreadyThere: 0,
    byLabel: { checking: 0, newToYou: 0, alreadyHave: 0, betterFile: 0, otherCut: 0, rework: 0 },
    folders: [],
  },
  review_counts: { needsCloserLook: 0, clearBestFile: 0, qualityProblems: 0, conflicts: 0, total: 0 },
  send_state: IDLE_SEND,
  rekordbox_xml_source: {
    path: null,
    watch: false,
    lastRead: null,
    lastFailure: null,
    newerExport: null,
    exportFolder: null,
  },
  rekordbox_offer: { toAdd: 0, alreadyInLibrary: 0, waitingInMissing: 0, waitingForConfirmation: 0 },
  music_folders: [],
  read_online_only_files: false,
  read_energy_as_stars: false,
  feature_enabled: false,
  update_status: { asked: true, enabled: false, current: "0.1.0", available: null, checkedAt: null },
  check_for_updates: { asked: true, enabled: false, current: "0.1.0", available: null, checkedAt: null },
  key_notation: "camelot",
  table_columns: null,
  track_table_formats: [],
  track_table_page: { total: 0, offset: 0, rows: [], behind: false },
  missing_tracks: { total: 0, groups: [] },
  review_conflict_fields: [],
  waveform_peaks: { state: "notReady" },
  next_for: { results: [], considered: 0, ignored: [], settings: NEXT_SETTINGS },
  next_settings: NEXT_SETTINGS,
  online_services_status: {
    musicBrainz: false,
    acoustId: { on: false, keyMissing: true },
    discogs: { on: false, tokenSaved: false, storeAvailable: true },
    coverArt: false,
  },
  identify_suggestions: { fields: [], catalogueNumber: { field: "CatalogueNumber", current: null, suggestions: [] } },
  identify_lookup_status: [],
  identify_cover_releases: [],
  audio_model_status: { enabled: false, fingerprinted: 0, total: 0, failed: 0, running: false },
};

/** Whether `cmd` has a quiet answer. */
export const isQuiet = (cmd: string): boolean => cmd in ANSWERS;

/** The quiet answer to `cmd`; throws, as an unknown command does, if there is none. */
export function quietAnswer(cmd: string): unknown {
  if (!isQuiet(cmd)) throw new Error(`unexpected command ${cmd}`);
  return ANSWERS[cmd];
}
