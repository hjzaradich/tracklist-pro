// Where to look for a track in each store (ROADMAP 2.4). The app only builds
// the address and shows it with a Copy button: nothing here opens a page or
// asks a store anything (no network from the app, ROADMAP 1.2).

export const STORES = ["beatport", "bandcamp", "traxsource", "juno"] as const;
export type Store = (typeof STORES)[number];

/** Each store's search page; `{q}` is the encoded search words. */
const TEMPLATES: Record<Store, string> = {
  beatport: "https://www.beatport.com/search?q={q}",
  bandcamp: "https://bandcamp.com/search?q={q}",
  traxsource: "https://www.traxsource.com/search?term={q}",
  juno: "https://www.junodownload.com/search/?q%5Ball%5D%5B%5D={q}",
};

/** The search words for a track: artist, then title, then the version if it has one. */
export function searchWords(track: { artist: string; title: string; version: string | null }): string {
  return [track.artist, track.title, track.version ?? ""].join(" ").replace(/\s+/g, " ").trim();
}

/** The address of the search for `track` in `store`. */
export function storeLink(
  store: Store,
  track: { artist: string; title: string; version: string | null },
): string {
  return TEMPLATES[store].replace("{q}", encodeURIComponent(searchWords(track)));
}
