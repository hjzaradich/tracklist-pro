import { Link, useLocation } from "@tanstack/react-router";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import type { Crate } from "../bindings";
import { useAddReport, useAddSelectionToCrate } from "../crates/addSelection";
import { ancestorIds, buildTree, type TreeNode } from "../crates/tree";
import { useCrates } from "../crates/useCrates";
import { useTrackDrag } from "../trackTable/trackDrag";
import { STAGES, type StageId } from "./stages";
import styles from "./Sidebar.module.css";

interface SidebarProps {
  className?: string;
  /** Count badges per stage. Nothing feeds them yet. */
  counts?: Partial<Record<StageId, number>>;
}

/**
 * The stages in workflow order, each with a count badge slot, then the
 * Crates section (1.14): the Crates stage and, under it, the tree of folders
 * and crates (1cA-11), or a line saying there are none. A crate or folder
 * opens on the Crates screen (`/crates?crate=ID`). Tracks dragged out of a
 * track table (1cB-13) can be dropped on a crate: it lights up under the
 * pointer; a folder does not (it holds crates, not tracks), and neither does
 * a stage.
 */
export function Sidebar({ className = "", counts = {} }: SidebarProps) {
  const { t } = useTranslation("shell");
  const workflow = STAGES.filter((stage) => stage.id !== "crates");
  const all = useCrates().data;
  // How many crates there are (folders don't count); unknown until loaded.
  const crates = all?.filter((c) => c.kind === "static").length;
  const { pathname, search } = useLocation({
    select: (l) => ({ pathname: l.pathname, search: l.search as { crate?: unknown } }),
  });
  const chosen = pathname === "/crates" && typeof search.crate === "number" ? search.crate : null;

  return (
    <nav aria-label={t("stages.label")} className={`${styles.sidebar} ${className}`}>
      <ul className={styles.list}>
        {workflow.map((stage) => (
          <li key={stage.id}>
            <StageLink to={stage.path} label={t(`stages.${stage.id}`)} count={counts[stage.id]} />
          </li>
        ))}
      </ul>
      <div className={styles.section}>
        <StageLink
          to="/crates"
          label={t("stages.crates")}
          count={counts.crates ?? (crates ? crates : undefined)}
        />
        {all !== undefined && all.length === 0 && (
          <p className={styles.empty}>{t("sidebar.noCrates")}</p>
        )}
        {all !== undefined && all.length > 0 && (
          <CrateTree nodes={buildTree(all)} all={all} chosen={chosen} />
        )}
      </div>
    </nav>
  );
}

/** What the tree needs to take a drop of tracks (1cB-13). */
interface DropOn {
  /** Whether tracks are being dragged now. */
  dragging: boolean;
  /** The crate or folder under the pointer while they are. */
  over: number | null;
  hover: (id: number | null) => void;
  /** The pointer was released over this crate or folder. */
  drop: (crate: Crate) => void;
}

function useDropOnCrates(): DropOn {
  const dragging = useTrackDrag((state) => state.ids !== null);
  const over = useTrackDrag((state) => state.over);
  const hover = useTrackDrag((state) => state.hover);
  const addSelection = useAddSelectionToCrate();
  return {
    dragging,
    over,
    hover,
    drop: (crate) => {
      const { ids, end } = useTrackDrag.getState();
      if (ids === null) return;
      end();
      // A folder holds crates, never tracks: nothing is added, and it says so.
      if (crate.kind === "folder") useAddReport.getState().show({ kind: "folder" });
      else void addSelection(crate, ids);
    },
  };
}

/**
 * The tree under the Crates stage. Folders open and close; the one holding
 * the chosen crate is always open, so the choice is never hidden.
 */
function CrateTree({
  nodes,
  all,
  chosen,
}: {
  nodes: TreeNode[];
  all: Crate[];
  chosen: number | null;
}) {
  const { t } = useTranslation("shell");
  const [closed, setClosed] = useState<ReadonlySet<number>>(new Set());
  const dropOn = useDropOnCrates();
  const forcedOpen = new Set(chosen === null ? [] : ancestorIds(all, chosen));
  const toggle = (id: number) =>
    setClosed((before) => {
      const next = new Set(before);
      if (!next.delete(id)) next.add(id);
      return next;
    });
  return (
    <ul className={styles.tree} aria-label={t("sidebar.treeLabel")}>
      <TreeItems nodes={nodes} closed={closed} forcedOpen={forcedOpen} toggle={toggle} dropOn={dropOn} />
    </ul>
  );
}

function TreeItems({
  nodes,
  closed,
  forcedOpen,
  toggle,
  dropOn,
}: {
  nodes: TreeNode[];
  closed: ReadonlySet<number>;
  forcedOpen: ReadonlySet<number>;
  toggle: (id: number) => void;
  dropOn: DropOn;
}) {
  const { t } = useTranslation("shell");
  return nodes.map(({ crate, children }) => {
    const folder = crate.kind === "folder";
    const open = !closed.has(crate.id) || forcedOpen.has(crate.id);
    return (
      <li key={crate.id}>
        <div
          className={styles.treeRow}
          data-drop={
            dropOn.dragging && dropOn.over === crate.id ? (folder ? "refused" : "target") : undefined
          }
          onPointerEnter={() => dropOn.dragging && dropOn.hover(crate.id)}
          onPointerLeave={() => dropOn.dragging && dropOn.hover(null)}
          onPointerUp={() => dropOn.drop(crate)}
        >
          {folder ? (
            <button
              type="button"
              className={styles.toggle}
              aria-expanded={open}
              aria-label={t(open ? "sidebar.collapse" : "sidebar.expand", { name: crate.name })}
              onClick={() => toggle(crate.id)}
            >
              <span aria-hidden="true">{open ? "▾" : "▸"}</span>
            </button>
          ) : (
            <span className={styles.toggle} aria-hidden="true" />
          )}
          <Link to="/crates" search={{ crate: crate.id }} className={styles.link}>
            <span className={styles.label}>{crate.name}</span>
          </Link>
        </div>
        {folder && open && children.length > 0 && (
          <ul className={`${styles.tree} ${styles.nested}`}>
            <TreeItems
              nodes={children}
              closed={closed}
              forcedOpen={forcedOpen}
              toggle={toggle}
              dropOn={dropOn}
            />
          </ul>
        )}
      </li>
    );
  });
}

interface StageLinkProps {
  to: (typeof STAGES)[number]["path"];
  label: string;
  count?: number;
}

function StageLink({ to, label, count }: StageLinkProps) {
  return (
    <Link to={to} className={styles.link}>
      <span className={styles.label}>{label}</span>
      <CountBadge count={count} />
    </Link>
  );
}

/** A stage's count, e.g. how many decisions are waiting in Review. */
export function CountBadge({ count }: { count?: number }) {
  const { i18n } = useTranslation();
  if (count === undefined) return null;
  return (
    <span className={styles.badge} data-testid="count-badge">
      {new Intl.NumberFormat(i18n.language).format(count)}
    </span>
  );
}
