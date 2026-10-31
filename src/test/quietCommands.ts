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
  failure: null,
  sent: null,
};

const ANSWERS: Record<string, unknown> = {
  list_crates: [],
  crate_summary: EMPTY_SUMMARY,
  library_summary: { tracks: 0, changeStamp: 0 },
  next_undo_operation: { operation: null, refusal: null },
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
  key_notation: "camelot",
  table_columns: null,
  track_table_formats: [],
  track_table_page: { total: 0, offset: 0, rows: [], behind: false },
  missing_tracks: { total: 0, groups: [] },
  review_conflict_fields: [],
  waveform_peaks: { state: "notReady" },
};

/** Whether `cmd` has a quiet answer. */
export const isQuiet = (cmd: string): boolean => cmd in ANSWERS;

/** The quiet answer to `cmd`; throws, as an unknown command does, if there is none. */
export function quietAnswer(cmd: string): unknown {
  if (!isQuiet(cmd)) throw new Error(`unexpected command ${cmd}`);
  return ANSWERS[cmd];
}
