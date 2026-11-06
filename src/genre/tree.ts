import type { GenreNode } from "../bindings";

/** A genre with how deep it sits, in tree order (a branch, then what is below it). */
export interface Row {
  node: GenreNode;
  depth: number;
}

/**
 * The nodes as rows in tree order. The backend sends siblings in their
 * order; a node whose parent is missing is shown at the top, so a stale list
 * never hides a genre.
 */
export function rows(nodes: readonly GenreNode[]): Row[] {
  const ids = new Set(nodes.map((n) => n.id));
  const children = new Map<number | null, GenreNode[]>();
  for (const node of nodes) {
    const key =
      node.parentId !== null && ids.has(node.parentId) ? node.parentId : null;
    children.set(key, [...(children.get(key) ?? []), node]);
  }
  const out: Row[] = [];
  const seen = new Set<number>();
  const walk = (parent: number | null, depth: number) => {
    for (const node of children.get(parent) ?? []) {
      if (seen.has(node.id)) continue;
      seen.add(node.id);
      out.push({ node, depth });
      walk(node.id, depth + 1);
    }
  };
  walk(null, 0);
  return out;
}

/** The ids of `id` and everything below it: where it can't be moved. */
export function withDescendants(
  nodes: readonly GenreNode[],
  id: number,
): Set<number> {
  const down = new Set<number>([id]);
  let grew = true;
  while (grew) {
    grew = false;
    for (const node of nodes) {
      if (
        node.parentId !== null &&
        down.has(node.parentId) &&
        !down.has(node.id)
      ) {
        down.add(node.id);
        grew = true;
      }
    }
  }
  return down;
}
