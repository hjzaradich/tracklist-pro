import { Outlet } from "@tanstack/react-router";
import { CommandPalette } from "../palette/CommandPalette";
import { useScannedFilesSync } from "../scan/useScannedFilesSync";
import { TrackDragChip } from "../trackTable/TrackDragChip";
import { DetailsPanel } from "./DetailsPanel";
import { PlayerBar } from "./PlayerBar";
import { useTableOnScreen } from "./selectedTrack";
import { Sidebar } from "./Sidebar";
import { TopBar } from "./TopBar";
import { useNoteLibraryChanges } from "../send/useLibraryChangedAt";
import { useReloadListsWhenJobsEnd } from "./useReloadListsWhenJobsEnd";
import { ZONES, type Zones } from "./zones";
import styles from "./AppShell.module.css";

/**
 * The app's frame (ROADMAP 1.1, Layout): top bar, sidebar, the current
 * stage's screen in the center, and, once their features exist, the Details
 * panel on the right and the player along the bottom, and over them all the
 * command palette (Ctrl+K, 1cB-7). A zone that isn't
 * drawn (`zones`, by default {@link ZONES}) leaves its space to the center.
 */
export function AppShell({ zones = ZONES }: { zones?: Zones }) {
  useScannedFilesSync();
  useReloadListsWhenJobsEnd();
  useNoteLibraryChanges();
  // The panel shows the selected row of a track table: it is drawn beside one.
  const tableOnScreen = useTableOnScreen();
  const details = zones.details && tableOnScreen;
  return (
    <div
      className={styles.shell}
      data-details={details ? "on" : "off"}
      data-player={zones.player ? "on" : "off"}
    >
      <TopBar className={styles.top} search={zones.search} />
      <Sidebar className={styles.sidebar} />
      <main className={styles.center}>
        <Outlet />
      </main>
      {details && <DetailsPanel className={styles.details} />}
      {zones.player && <PlayerBar className={styles.player} />}
      <CommandPalette />
      <TrackDragChip />
    </div>
  );
}
