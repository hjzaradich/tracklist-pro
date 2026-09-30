import { Outlet } from "@tanstack/react-router";
import { useScannedFilesSync } from "../scan/useScannedFilesSync";
import { DetailsPanel } from "./DetailsPanel";
import { PlayerBar } from "./PlayerBar";
import { Sidebar } from "./Sidebar";
import { TopBar } from "./TopBar";
import styles from "./AppShell.module.css";

/**
 * The app's frame (ROADMAP 1.1, Layout): top bar, sidebar, the current
 * stage's screen in the center, the Details panel on the right and the player
 * along the bottom. Every zone is a placeholder until its feature arrives.
 */
export function AppShell() {
  useScannedFilesSync();
  return (
    <div className={styles.shell}>
      <TopBar className={styles.top} />
      <Sidebar className={styles.sidebar} />
      <main className={styles.center}>
        <Outlet />
      </main>
      <DetailsPanel className={styles.details} />
      <PlayerBar className={styles.player} />
    </div>
  );
}
