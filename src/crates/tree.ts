import type { Crate } from "../bindings";

/** A folder or crate with what is in it. */
export interface TreeNode {
  crate: Crate;
  children: TreeNode[];
}

/**
 * The tree the backend's flat list describes. The list comes in each
 * folder's own order (the order a send writes), and that order is kept.
 */
export function buildTree(list: Crate[]): TreeNode[] {
  const nodes = new Map<number, TreeNode>(list.map((c) => [c.id, { crate: c, children: [] }]));
  const top: TreeNode[] = [];
  for (const crate of list) {
    const node = nodes.get(crate.id);
    if (node === undefined) continue;
    const parent = crate.parentId === null ? undefined : nodes.get(crate.parentId);
    (parent?.children ?? top).push(node);
  }
  return top;
}

/**
 * Finds the names from the top of the tree down to a crate or folder, or none
 * if it isn't there. The lookup over `list` is built once, so asking for
 * every crate's path costs one pass over the list, not one per crate.
 */
export function pathFinder(list: Crate[]): (id: number) => string[] {
  const byId = new Map(list.map((c) => [c.id, c]));
  return (id) => {
    const names: string[] = [];
    const seen = new Set<number>();
    for (let at = byId.get(id); at !== undefined && !seen.has(at.id); ) {
      seen.add(at.id);
      names.unshift(at.name);
      at = at.parentId === null ? undefined : byId.get(at.parentId);
    }
    return names;
  };
}

/** The names from the top of the tree down to `id`, or none if it isn't there. */
export function pathOf(list: Crate[], id: number): string[] {
  return pathFinder(list)(id);
}

/** The ids of every folder and crate somewhere below `id`. */
export function descendantIds(list: Crate[], id: number): Set<number> {
  const below = new Set<number>();
  let grew = true;
  while (grew) {
    grew = false;
    for (const crate of list) {
      if (crate.parentId !== null && (crate.parentId === id || below.has(crate.parentId))) {
        if (!below.has(crate.id)) {
          below.add(crate.id);
          grew = true;
        }
      }
    }
  }
  return below;
}

/** The ids of the folders above `id`, nearest first. */
export function ancestorIds(list: Crate[], id: number): number[] {
  const byId = new Map(list.map((c) => [c.id, c]));
  const up: number[] = [];
  for (let at = byId.get(id)?.parentId ?? null; at !== null && !up.includes(at); ) {
    up.push(at);
    at = byId.get(at)?.parentId ?? null;
  }
  return up;
}

/** Folder and crate names joined the way a path reads. */
export const PATH_SEPARATOR = " › ";

/** Every node in the order the tree is drawn, with its depth (0 at the top). */
export function walk(list: Crate[]): { crate: Crate; depth: number }[] {
  const out: { crate: Crate; depth: number }[] = [];
  const visit = (nodes: TreeNode[], depth: number) => {
    for (const node of nodes) {
      out.push({ crate: node.crate, depth });
      visit(node.children, depth + 1);
    }
  };
  visit(buildTree(list), 0);
  return out;
}

/** A hand-made crate and the name it is chosen by. */
export interface CrateChoice {
  crate: Crate;
  label: string;
}

/** Hand-made crates in tree order, each named by where it is ("Friday residency › Warmup"). */
export function crateChoices(list: Crate[]): CrateChoice[] {
  const pathTo = pathFinder(list);
  return walk(list)
    .filter(({ crate }) => crate.kind === "static")
    .map(({ crate }) => ({ crate, label: pathTo(crate.id).join(PATH_SEPARATOR) }));
}

/** The places something can be moved to: the top, then every folder not in `except`. */
export function folderChoices(
  list: Crate[],
  except: Set<number>,
): { id: number; label: string }[] {
  const pathTo = pathFinder(list);
  return walk(list)
    .filter(({ crate }) => crate.kind === "folder" && !except.has(crate.id))
    .map(({ crate }) => ({ id: crate.id, label: pathTo(crate.id).join(PATH_SEPARATOR) }));
}
