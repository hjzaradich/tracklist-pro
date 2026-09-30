"""Reading and writing rekordbox XML for the rekordbox behavior check.

Standard library only. Follows ROADMAP §5.3:
  - `Location` is decoded by hand. rekordbox leaves `# ( ) , + !` raw and
    writes lowercase `%xx`, so a URL parser (which cuts at `#`) must not be used.
  - Tracks are matched by their decoded path, never by TrackID.
  - Built-in demo tracks, samples, deleted rows and streaming entries are skipped.
"""

import unicodedata
import xml.etree.ElementTree as ET
from dataclasses import dataclass, field
from pathlib import Path
from xml.sax.saxutils import quoteattr

PREFIX = "file://localhost/"

# Unreserved characters, which neither our writer nor rekordbox encodes.
_UNRESERVED = set(b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~")
# rekordbox 7.2 additionally leaves these raw (seen in its exports, E1 session).
_RB_RAW = _UNRESERVED | set(b"/:(),+#!")
# ROADMAP 1.9 rule 5: we encode everything except the path separators.
_OUR_RAW = _UNRESERVED | set(b"/:")

KIND = {".mp3": "MP3 File", ".flac": "FLAC File", ".wav": "WAV File", ".aiff": "AIFF File",
        ".aif": "AIFF File", ".m4a": "M4A File"}


def _encode(path: str, raw: set, hexfmt: str) -> str:
    out = []
    for byte in path.replace("\\", "/").encode("utf-8"):
        out.append(chr(byte) if byte in raw else hexfmt % byte)
    return PREFIX + "".join(out)


def encode_location(path) -> str:
    """Our writer's Location: fully percent-encoded, uppercase hex (ROADMAP 1.9 rule 5)."""
    return _encode(str(path), _OUR_RAW, "%%%02X")


def encode_location_rekordbox_style(path) -> str:
    """How rekordbox 7.2 itself writes a Location. Used to compare with its exports."""
    return _encode(str(path), _RB_RAW, "%%%02x")


def decode_location(location: str):
    """Decodes a Location to a Windows-style path with forward slashes, by hand.

    Returns None for anything that isn't a local file (streaming entries etc.).
    Only `%xx` is special; `#`, `+` and everything else are literal.
    """
    if not location.lower().startswith(PREFIX):
        return None
    rest = location[len(PREFIX):]
    buf = bytearray()
    i = 0
    while i < len(rest):
        ch = rest[i]
        if ch == "%" and _is_hex(rest[i + 1:i + 3]):
            buf.append(int(rest[i + 1:i + 3], 16))
            i += 3
        else:
            buf.extend(ch.encode("utf-8"))
            i += 1
    return buf.decode("utf-8", errors="replace")


def _is_hex(s: str) -> bool:
    return len(s) == 2 and all(c in "0123456789abcdefABCDEF" for c in s)


def path_key(path) -> str:
    """Comparison key for a path: forward slashes, NFC, case-folded (Windows paths are case-insensitive)."""
    return unicodedata.normalize("NFC", str(path).replace("\\", "/")).casefold()


def is_builtin(path: str) -> bool:
    """rekordbox's own demo tracks and samples, which every export contains (§5.3)."""
    p = path.replace("\\", "/").lower()
    return "/music/rekordbox/sampler/" in p or "/music/pioneerdj/demo tracks/" in p


@dataclass
class Track:
    attrs: dict
    tempos: list = field(default_factory=list)
    marks: list = field(default_factory=list)
    path: str = ""

    @property
    def key(self) -> str:
        return path_key(self.path)


@dataclass
class Playlist:
    path: tuple            # folder names down to the playlist name, below ROOT
    key_type: str          # "0" = TrackID, "1" = Location
    keys: list             # raw Key values, in order
    entries: list          # path_key of each resolved entry, or None if it didn't resolve


@dataclass
class Library:
    product: dict
    tracks: list
    by_key: dict           # path_key -> [Track]; more than one means a duplicate
    by_id: dict            # TrackID -> Track
    playlists: dict        # path tuple -> [Playlist]; more than one means a same-name duplicate
    folders: dict          # path tuple -> number of folders with that path
    root_order: dict       # folder path tuple -> child names in order

    def one(self, key):
        found = self.by_key.get(key, [])
        return found[0] if len(found) == 1 else None

    def playlist(self, path):
        found = self.playlists.get(tuple(path), [])
        return found[0] if len(found) == 1 else None


def parse(source) -> Library:
    """Parses a rekordbox XML file (path) or XML text (str starting with '<')."""
    if isinstance(source, str) and source.lstrip().startswith("<"):
        root = ET.fromstring(source)
    else:
        root = ET.parse(source).getroot()
    product = dict(root.find("PRODUCT").attrib) if root.find("PRODUCT") is not None else {}
    tracks, by_key, by_id = [], {}, {}
    coll = root.find("COLLECTION")
    for el in (coll.findall("TRACK") if coll is not None else []):
        attrs = dict(el.attrib)
        if attrs.get("rb_local_deleted", "0") not in ("0", ""):
            continue
        path = decode_location(attrs.get("Location", ""))
        if path is None or is_builtin(path):
            continue
        t = Track(attrs=attrs, tempos=[dict(x.attrib) for x in el.findall("TEMPO")],
                  marks=[dict(x.attrib) for x in el.findall("POSITION_MARK")], path=path)
        tracks.append(t)
        by_key.setdefault(t.key, []).append(t)
        if "TrackID" in attrs:
            by_id[attrs["TrackID"]] = t
    playlists, folders, order = {}, {}, {}
    pl_root = root.find("PLAYLISTS")
    top = pl_root.find("NODE") if pl_root is not None else None
    if top is not None:
        _walk(top, (), by_id, playlists, folders, order)
    return Library(product, tracks, by_key, by_id, playlists, folders, order)


def _walk(node, path, by_id, playlists, folders, order):
    order[path] = [c.get("Name", "") for c in node.findall("NODE")]
    for child in node.findall("NODE"):
        cpath = path + (child.get("Name", ""),)
        if child.get("Type") == "0":
            folders[cpath] = folders.get(cpath, 0) + 1
            _walk(child, cpath, by_id, playlists, folders, order)
        else:
            key_type = child.get("KeyType", "0")
            keys = [t.get("Key", "") for t in child.findall("TRACK")]
            entries = []
            for k in keys:
                if key_type == "1":
                    p = decode_location(k)
                    entries.append(path_key(p) if p is not None else None)
                else:
                    t = by_id.get(k)
                    entries.append(t.key if t else None)
            playlists.setdefault(cpath, []).append(Playlist(cpath, key_type, keys, entries))


# ---------- writing ----------

def folder(name, *children):
    return ("folder", name, list(children))


def playlist(name, keys, key_type="0"):
    return ("playlist", name, key_type, list(keys))


def _node_lines(node, depth):
    pad = "  " * depth
    if node[0] == "folder":
        _, name, children = node
        lines = [f'{pad}<NODE Type="0" Name={quoteattr(name)} Count="{len(children)}">']
        for c in children:
            lines += _node_lines(c, depth + 1)
        lines.append(f"{pad}</NODE>")
        return lines
    _, name, key_type, keys = node
    if not keys:
        return [f'{pad}<NODE Name={quoteattr(name)} Type="1" KeyType="{key_type}" Entries="0"/>']
    lines = [f'{pad}<NODE Name={quoteattr(name)} Type="1" KeyType="{key_type}" Entries="{len(keys)}">']
    lines += [f"{pad}  <TRACK Key={quoteattr(str(k))}/>" for k in keys]
    lines.append(f"{pad}</NODE>")
    return lines


def track_line(attrs: dict, tempos=(), marks=()) -> str:
    head = "    <TRACK " + " ".join(f"{k}={quoteattr(str(v))}" for k, v in attrs.items())
    if not tempos and not marks:
        return head + "/>"
    kids = [f"      <TEMPO {' '.join(f'{k}={quoteattr(v)}' for k, v in t.items())}/>" for t in tempos]
    kids += [f"      <POSITION_MARK {' '.join(f'{k}={quoteattr(v)}' for k, v in m.items())}/>" for m in marks]
    return "\n".join([head + ">", *kids, "    </TRACK>"])


def render(tracks, nodes, product=("tracklist-pro", "0.0-check", "tracklist-pro")) -> str:
    """tracks: list of attribute dicts, or (attrs, tempos, marks) tuples. nodes: top-level playlist nodes."""
    lines = ['<?xml version="1.0" encoding="UTF-8"?>', '<DJ_PLAYLISTS Version="1.0.0">',
             f"  <PRODUCT Name={quoteattr(product[0])} Version={quoteattr(product[1])} "
             f"Company={quoteattr(product[2])}/>",
             f'  <COLLECTION Entries="{len(tracks)}">']
    for t in tracks:
        lines.append(track_line(*t) if isinstance(t, tuple) else track_line(t))
    lines += ["  </COLLECTION>", "  <PLAYLISTS>", f'    <NODE Type="0" Name="ROOT" Count="{len(nodes)}">']
    for n in nodes:
        lines += _node_lines(n, 3)
    lines += ["    </NODE>", "  </PLAYLISTS>", "</DJ_PLAYLISTS>", ""]
    return "\n".join(lines)


def write(dest: Path, tracks, nodes, **kw):
    Path(dest).write_text(render(tracks, nodes, **kw), encoding="utf-8")
