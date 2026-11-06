import { describe, expect, it } from "vitest";
import { searchWords, storeLink, STORES } from "./linkTemplates";

const TRACK = { artist: "Artist One", title: "Title & One", version: "Dub Mix" };

describe("store links", () => {
  it("search for artist, title and version, encoded", () => {
    expect(searchWords(TRACK)).toBe("Artist One Title & One Dub Mix");
    expect(searchWords({ ...TRACK, version: null })).toBe("Artist One Title & One");
    for (const store of STORES) {
      const link = new URL(storeLink(store, TRACK));
      expect(link.protocol).toBe("https:");
      // The search words survive the round trip through the address.
      expect([...link.searchParams.values()]).toEqual(["Artist One Title & One Dub Mix"]);
    }
  });

  it("go to each store's own search page", () => {
    const hosts = STORES.map((store) => new URL(storeLink(store, TRACK)).hostname);
    expect(hosts).toEqual([
      "www.beatport.com",
      "bandcamp.com",
      "www.traxsource.com",
      "www.junodownload.com",
    ]);
  });
});
