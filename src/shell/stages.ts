/**
 * The workflow stages, in order (ROADMAP 1.11). The sidebar lists them in
 * this order, and each has its own route and screen. Inbox (Phase 2) and
 * Playlists (Phase 3) join the list when their phase arrives.
 *
 * Each stage's name lives once, in the shell namespace (`stages.<id>`), and
 * both the sidebar and the stage's screen read it from there.
 */
export const STAGES = [
  { id: "overview", path: "/overview" },
  { id: "review", path: "/review" },
  { id: "allMusic", path: "/all-music" },
  { id: "library", path: "/library" },
  { id: "crates", path: "/crates" },
] as const;

export type StageId = (typeof STAGES)[number]["id"];

/** Where the app opens until "reopen where you left off" arrives (1.13). */
export const HOME_STAGE_PATH = "/overview";
