"""Minimal read-only reader for the history tables of a rekordbox export.pdb (DeviceSQL).

Layout per Deep Symmetry's crate-digger analysis (rekordbox_pdb.ksy):
file header: u32 0, u32 page_len, u32 num_tables, u32 next_unused, u32 ?, u32 sequence, u32 0,
then num_tables x (u32 type, u32 empty_candidate, u32 first_page, u32 last_page).
Page header (0x28 bytes): u32 0, u32 page_index, u32 type, u32 next_page, u32 ?, u32 ?,
u8 num_rows_small, u8 ?, u8 ?, u8 page_flags, u16 free_size, u16 used_size, u16 ?, u16 num_rows_large, ...
Heap starts at 0x28. Row index grows backwards from the page end in groups of 16:
16 x u16 offsets (last row first), then u16 present-flags, then u16 ?.
"""
import struct
import sys

TYPES = {0: "tracks", 1: "genres", 2: "artists", 3: "albums", 4: "labels", 5: "keys", 6: "colors",
         7: "playlist_tree", 8: "playlist_entries", 11: "history_playlists", 12: "history_entries",
         13: "artwork", 16: "columns", 19: "history"}


def dstring(buf, off):
    kind = buf[off]
    if kind == 0x40:  # long ASCII
        n = struct.unpack_from("<H", buf, off + 1)[0]
        return buf[off + 4:off + n].decode("latin1")
    if kind == 0x90:  # long UTF-16LE
        n = struct.unpack_from("<H", buf, off + 1)[0]
        return buf[off + 4:off + n].decode("utf-16le")
    n = (kind >> 1) - 1  # short ASCII
    return buf[off + 1:off + 1 + n].decode("latin1")


def rows(data, page_len, first, last):
    page = first
    seen = set()
    while page not in seen:
        seen.add(page)
        base = page * page_len
        p = data[base:base + page_len]
        if len(p) < 0x28:
            break
        next_page = struct.unpack_from("<I", p, 12)[0]
        flags = p[0x1b]
        nsmall = p[0x18]
        nlarge = struct.unpack_from("<H", p, 0x22)[0]
        n = nlarge if nlarge > nsmall and nlarge != 0x1fff else nsmall
        if flags & 0x40 == 0:  # data page
            for g in range((n + 15) // 16):
                gbase = page_len - g * 0x24
                present = struct.unpack_from("<H", p, gbase - 4)[0]
                for i in range(min(16, n - g * 16)):
                    if present & (1 << i):
                        off = struct.unpack_from("<H", p, gbase - 6 - 2 * i)[0]
                        yield p, 0x28 + off
        if page == last:
            break
        page = next_page


def main(path):
    data = open(path, "rb").read()
    page_len, ntables = struct.unpack_from("<II", data, 4)
    tables = {}
    for t in range(ntables):
        ttype, _, first, last = struct.unpack_from("<IIII", data, 0x1c + 16 * t)
        tables[ttype] = (first, last)
    print("page_len", page_len, "| tables:", {TYPES.get(k, k): v for k, v in tables.items()})
    names = {}
    if 11 in tables:
        for p, o in rows(data, page_len, *tables[11]):
            pid = struct.unpack_from("<I", p, o)[0]
            names[pid] = dstring(p, o + 4)
    print("history playlists:", names)
    entries = []
    if 12 in tables:
        for p, o in rows(data, page_len, *tables[12]):
            entries.append(struct.unpack_from("<III", p, o))  # track_id, playlist_id, entry_index
    print("history entries:", len(entries))
    for tid, pid, idx in sorted(entries, key=lambda e: (e[1], e[2]))[:40]:
        print(f"  {names.get(pid, pid)} #{idx}: track {tid}")
    return entries


if __name__ == "__main__":
    main(sys.argv[1])
