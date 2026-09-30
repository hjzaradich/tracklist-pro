"""Tests for the rekordbox behavior check. Run from this folder:

    python -m unittest -v test_rb_check

CI-safe tests use synthetic data only: the committed fixture and a small simulated
rekordbox that follows ROADMAP §5.2. `E1ExportsLocal` runs only where the private E1
exports exist (spikes/results/, or TLP_RB_RESULTS); CI never has them.
"""

import array
import contextlib
import hashlib
import io
import math
import os
import random
import tempfile
import unittest
import wave
from pathlib import Path

import rb_check as rc
import rbxml
import tones
from rb_check import FAIL, INCON, NOTRUN, PASS

HERE = Path(__file__).resolve().parent
FIXTURE = HERE / "fixtures" / "rb-export-snippet.xml"


# ---------------------------------------------------------------- Location and parsing

class Locations(unittest.TestCase):
    def test_raw_hash_plus_and_lowercase_hex(self):
        loc = "file://localhost/C:/Kit/04%20Low%20Tide%20#1%20(100%25%20A+B),%20Caf%c3%a9%20%f0%9f%9a%80.wav"
        self.assertEqual(rbxml.decode_location(loc), "C:/Kit/04 Low Tide #1 (100% A+B), Café 🚀.wav")

    def test_lone_percent_stays_literal(self):
        self.assertEqual(rbxml.decode_location("file://localhost/C:/a%zz%2"), "C:/a%zz%2")

    def test_not_a_file(self):
        self.assertIsNone(rbxml.decode_location("soundcloud:tracks:1"))

    def test_our_encoding_is_full_and_round_trips(self):
        path = "C:/Kit/tracks/03 Plays - Low Tide #1 (100% Flip) Don't [x] & y + z, 日本語 🚀.mp3"
        ours = rbxml.encode_location(path)
        body = ours[len(rbxml.PREFIX):]
        for ch in "#()'[]&+, ":
            self.assertNotIn(ch, body)
        self.assertRegex(body, r"^(?:[A-Za-z0-9._~/:-]|%[0-9A-F]{2})+$")
        self.assertEqual(rbxml.decode_location(ours), path)
        self.assertEqual(rbxml.decode_location(rbxml.encode_location_rekordbox_style(path)), path)

    def test_rekordbox_style_matches_its_exports(self):
        path = "C:/Kit/tracks/04 Kit & Kin - Low Tide #1 (100% Flip).wav"
        self.assertEqual(rbxml.encode_location_rekordbox_style(path),
                         "file://localhost/C:/Kit/tracks/04%20Kit%20%26%20Kin%20-%20Low%20Tide%20#1%20(100%25%20Flip).wav")

    def test_backslashes(self):
        self.assertEqual(rbxml.decode_location(rbxml.encode_location("C:\\a b\\c.mp3")), "C:/a b/c.mp3")

    def test_path_key_ignores_case_and_normalization(self):
        self.assertEqual(rbxml.path_key("C:/Kit/Cafe\u0301.mp3"), rbxml.path_key("c:\\kit\\CAFÉ.MP3"))


class ParseFixture(unittest.TestCase):
    def setUp(self):
        self.lib = rbxml.parse(FIXTURE)

    def test_skips_builtins_deleted_and_streaming(self):
        self.assertEqual(sorted(t.attrs["TrackID"] for t in self.lib.tracks), ["21899570", "42749078"])

    def test_track_fields_cues_and_grid(self):
        t = self.lib.by_id["21899570"]
        self.assertEqual(t.path, "C:/Kit/tracks/04 Kit & Kin - Low Tide #1 (100% Flip).wav")
        self.assertEqual(t.attrs["Comments"], "Kit & Kin's")
        self.assertEqual(len(t.tempos), 1)
        self.assertEqual([m["Num"] for m in t.marks], ["0", "-1"])
        self.assertEqual(self.lib.by_id["42749078"].path,
                         "C:/Kit/Sub Folder/Deeper Level/06 North Wind, Late Train 🚀 Café 日本 A + B [x].m4a")

    def test_playlists_resolve_both_key_types(self):
        by_id = self.lib.playlist(("TLP Check", "by-id"))
        a, b = self.lib.by_id["42749078"].key, self.lib.by_id["21899570"].key
        self.assertEqual(by_id.entries, [a, b, None])  # 555 isn't in the collection
        by_loc = self.lib.playlist(("TLP Check", "nested", "by-location"))
        self.assertEqual(by_loc.key_type, "1")
        self.assertEqual(by_loc.entries, [b])  # fully encoded Key matches rekordbox's own style

    def test_same_name_folders_are_counted(self):
        self.assertEqual(self.lib.folders[("TLP Check",)], 2)
        self.assertEqual(self.lib.root_order[()], ["CUE Analysis Playlist", "TLP Check", "TLP Check"])

    def test_render_parse_round_trip(self):
        t = self.lib.by_id["21899570"]
        text = rbxml.render([(t.attrs, t.tempos, t.marks)],
                            [rbxml.folder("F", rbxml.playlist("p", ["21899570"]),
                                          rbxml.playlist("q", [t.attrs["Location"]], "1"))])
        back = rbxml.parse(text)
        self.assertEqual(back.tracks[0].attrs, t.attrs)
        self.assertEqual(back.tracks[0].marks, t.marks)
        self.assertEqual(back.playlist(("F", "p")).entries, [t.key])
        self.assertEqual(back.playlist(("F", "q")).entries, [t.key])


# ---------------------------------------------------------------- a simulated rekordbox (§5.2)

RB_FIELDS = ("Name", "Artist", "Composer", "Album", "Grouping", "Genre", "Kind", "Size", "TotalTime", "DiscNumber",
             "TrackNumber", "Year", "AverageBpm", "DateAdded", "BitRate", "SampleRate", "Comments", "PlayCount",
             "Rating", "Remixer", "Tonality", "Label", "Mix")
# rekordbox's Classic key display, as REKORDBOX in src-tauri/src/tags/key.rs has it, by Camelot key.
RB_CLASSIC = dict(zip([f"{n}A" for n in range(1, 13)] + [f"{n}B" for n in range(1, 13)],
                      ["Abm", "Ebm", "Bbm", "Fm", "Cm", "Gm", "Dm", "Am", "Em", "Bm", "F#m", "Dbm",
                       "B", "F#", "Db", "Ab", "Eb", "Bb", "F", "C", "G", "D", "A", "E"]))
XML_TEXT = ("Name", "Artist", "Composer", "Album", "Genre", "Comments", "Rating", "Colour", "Remixer", "Label",
            "Mix", "Year", "TrackNumber", "DateAdded", "PlayCount")


class FakeRekordbox:
    """Behaves as ROADMAP §5.2 describes rekordbox 7.2. Switches turn single behaviors off."""

    def __init__(self, erase_omitted_analysis=False, keep_omitted_rating=False, keep_playcount=False,
                 resolve_absent=True, resolve_by_location=True, duplicate_on_reimport=False,
                 key_display="classic", spellings=None, convert_loaded_keys=False):
        self.tracks = {}      # path key -> {"path", "attrs", "tempos", "marks"}
        self.playlists = {}   # path tuple -> [path keys], insertion order = sidebar order
        self.auto_analysis = True
        self.rng = random.Random(7)
        self.erase_omitted_analysis = erase_omitted_analysis
        self.keep_omitted_rating = keep_omitted_rating
        self.keep_playcount = keep_playcount
        self.resolve_absent = resolve_absent
        self.resolve_by_location = resolve_by_location
        self.duplicate_on_reimport = duplicate_on_reimport
        self.extra = []       # duplicate entries, exported too
        self.key_display = key_display  # "classic" or "camelot"
        self.spellings = dict(RB_CLASSIC, **(spellings or {}))
        self.convert_loaded_keys = convert_loaded_keys  # a Camelot Tonality on re-import: convert it, or keep the string
        self.heard = {}       # path key -> Camelot key the analysis hears (the test tones); others are made up

    def _key_name(self, camelot):
        return camelot if self.key_display == "camelot" else self.spellings[camelot]

    def _new(self, path, attrs):
        t = {"path": path, "attrs": {f: "0" if f in ("PlayCount", "Rating", "Year") else "" for f in RB_FIELDS},
             "tempos": [], "marks": []}
        t["attrs"].update(attrs, TrackID=str(self.rng.randrange(10_000_000, 300_000_000)), TotalTime="200")
        if self.auto_analysis:
            h = sum(path.encode())
            heard = self.heard.get(rbxml.path_key(path), f"{h % 12 + 1}A")
            t["attrs"].update(AverageBpm=f"{170 + h % 9}.00", Tonality=self._key_name(heard))
            t["tempos"] = [{"Inizio": "0.057", "Bpm": t["attrs"]["AverageBpm"], "Metro": "4/4", "Battito": "1"}]
        return t

    def drag(self, paths):
        for p in paths:
            path = str(p).replace("\\", "/")
            self.tracks[rbxml.path_key(path)] = self._new(path, {"Name": Path(path).stem, "Comments": "file tag"})

    def import_collection(self, lib, answer=lambda label_path: True):
        for x in lib.tracks:
            cur = self.tracks.get(x.key)
            if cur is None:
                self.tracks[x.key] = self._new(x.path, {f: x.attrs[f] for f in XML_TEXT if f in x.attrs})
                continue
            if self.duplicate_on_reimport:
                self.extra.append(self._new(x.path, {}))
            if not answer(x.path):
                continue
            a = cur["attrs"]
            for f in XML_TEXT:
                if f in x.attrs:
                    if not (f == "PlayCount" and self.keep_playcount):
                        a[f] = x.attrs[f]
                elif f == "Rating" and not self.keep_omitted_rating:
                    a[f] = "0"
            for f in ("AverageBpm", "Tonality"):
                if f in x.attrs:
                    a[f] = x.attrs[f]
                    if f == "Tonality" and self.convert_loaded_keys and rc.is_camelot(a[f]):
                        a[f] = self._key_name(a[f])
                elif self.erase_omitted_analysis:
                    a[f] = "0.00" if f == "AverageBpm" else ""
                    cur["tempos"] = []

    def import_playlists(self, lib, top):
        for path, found in lib.playlists.items():
            if path[0] != top:
                continue
            entries = []
            for key, entry in zip(found[0].keys, found[0].entries):
                if found[0].key_type == "1":
                    if self.resolve_by_location and entry in self.tracks:
                        entries.append(entry)
                elif entry is not None:
                    entries.append(entry)
                elif self.resolve_absent:
                    own = [k for k, t in self.tracks.items() if t["attrs"]["TrackID"] == key]
                    entries += own
            self.playlists.pop(path, None)  # same name: replaced, and moved to the end
            self.playlists[path] = entries

    def edit(self, path, **attrs):
        self.tracks[rbxml.path_key(path)]["attrs"].update(attrs)

    def set_hot_cue(self, path):
        self.tracks[rbxml.path_key(path)]["marks"] = [
            {"Name": "", "Type": "0", "Start": "44.195", "Num": "0", "Red": "255", "Green": "55", "Blue": "111"}]

    def export(self, dest):
        rows = [({"TrackID": "153908294", "Name": "Demo Track 1", "Location": rbxml.encode_location_rekordbox_style(
            "C:/Users/someone/Music/PioneerDJ/Demo Tracks/Demo Track 1.mp3")}, [], [])]
        for t in list(self.tracks.values()) + self.extra:
            attrs = dict(t["attrs"], Location=rbxml.encode_location_rekordbox_style(t["path"]))
            rows.append((attrs, t["tempos"], t["marks"]))
        ids = {k: t["attrs"]["TrackID"] for k, t in self.tracks.items()}
        tree = rc._tree({("CUE Analysis Playlist",): [], **self.playlists}, lambda k: ids[k])
        rbxml.write(dest, rows, tree, product=("rekordbox", "7.2.19", "AlphaTheta"))


# ---------------------------------------------------------------- the kit

def make_source(root: Path):
    """12 usable files in mixed formats, plus files the picker must skip."""
    root.mkdir(parents=True)
    for i in range(12):
        ext = (".mp3", ".flac", ".wav", ".m4a")[i % 4]
        sub = root / ("Album A" if i < 6 else "Album B")
        sub.mkdir(exist_ok=True)
        (sub / f"Artist {i:02d} - Song{ext}").write_bytes(bytes([i]) * 2048)
    (root / "tiny.mp3").write_bytes(b"x")
    (root / "._hidden.mp3").write_bytes(b"x" * 4096)
    (root / "cover.jpg").write_bytes(b"x" * 4096)


def snapshot(root: Path):
    return {str(p): (hashlib.sha256(p.read_bytes()).hexdigest(), p.stat().st_mtime_ns)
            for p in root.rglob("*") if p.is_file()}


class KitTestCase(unittest.TestCase):
    def setUp(self):
        self._min = rc.MIN_SIZE
        rc.MIN_SIZE = 1024
        self.tmp = Path(tempfile.mkdtemp(prefix="rbcheck-"))
        self.source = self.tmp / "music"
        make_source(self.source)
        self.kit_root = self.tmp / "kit"

    def tearDown(self):
        rc.MIN_SIZE = self._min
        import shutil
        shutil.rmtree(self.tmp, ignore_errors=True)


class Build(KitTestCase):
    def test_copies_with_awkward_names_and_never_touches_the_source(self):
        before = snapshot(self.source)
        kit = rc.build(self.source, self.kit_root)
        self.assertEqual(snapshot(self.source), before)  # the music folder is only read
        for label, rel, *_ in rc.TRACKS:
            self.assertTrue(kit.path(label).exists(), label)
            self.assertEqual(kit.files[label][:-len(kit.path(label).suffix)], rel)
        self.assertEqual({kit.path(lab).suffix for lab in rc.LABELS if lab not in rc.KEY_TONES},
                         {".mp3", ".flac", ".wav", ".m4a"})
        written = {p for p in self.tmp.rglob("*") if p.is_file()} - {Path(p) for p in before}
        self.assertTrue(all(self.kit_root in p.parents for p in written))

    def test_first_xml(self):
        kit = rc.build(self.source, self.kit_root)
        sent = rbxml.parse(self.kit_root / rc.FIRST_XML)
        self.assertEqual(len(sent.tracks), len(rc.FIRST))
        for lab in rc.FIRST:
            t = sent.one(kit.key(lab))
            self.assertIsNotNone(t, lab)
            self.assertEqual(Path(t.path), kit.path(lab))
            self.assertTrue(t.attrs["Location"].startswith(rbxml.PREFIX))
            self.assertNotIn("#", t.attrs["Location"])  # rule 5: fully encoded
        for lab in ("10", "D1", "D2"):
            self.assertIsNone(sent.one(kit.key(lab)))
        for lab in rc.KEY_TONES:  # the tones go in, but their key is left to rekordbox's analysis
            self.assertNotIn("Tonality", sent.one(kit.key(lab)).attrs)
        self.assertEqual([sent.one(kit.key(lab)).attrs["TrackNumber"] for lab in ("01", "09", "K2B", "K6B")],
                         ["1", "9", "13", "14"])
        self.assertEqual(rc.playlists_match(sent, rc.FIRST_PLAYLISTS, kit.keys())[0], PASS)

    def test_key_tones_are_generated_wavs(self):
        kit = rc.build(self.source, self.kit_root)
        meta = rc.json.loads((self.kit_root / "kit.json").read_text(encoding="utf-8"))
        self.assertEqual(list(meta["tracks"]), rc.LABELS)
        for lab, (camelot, _, pc) in rc.KEY_TONES.items():
            p = kit.path(lab)
            self.assertEqual(p.suffix, ".wav")
            self.assertEqual(meta["tracks"][lab]["source"], f"tone {camelot}")  # not from the music folder
            with wave.open(str(p)) as w:
                self.assertEqual((w.getnchannels(), w.getsampwidth(), w.getframerate()), (1, 2, tones.RATE))
                self.assertEqual(w.getnframes(), rc.TONE_SECONDS * tones.RATE)
            self.assertGreaterEqual(p.stat().st_size, rc.MIN_SIZE)
        again = self.tmp / "again.wav"
        tones.write_tone(again, 6, seconds=rc.TONE_SECONDS)
        self.assertEqual(again.read_bytes(), kit.path("K2B").read_bytes())  # deterministic

    def test_refuses_a_kit_inside_the_music_folder(self):
        with self.assertRaises(rc.CheckError):
            rc.build(self.source, self.source / "kit")
        with self.assertRaises(rc.CheckError):
            rc.build(self.source, self.tmp)  # the music folder would be inside the kit

    def test_rebuild_needs_force_and_never_deletes_a_non_kit_folder(self):
        rc.build(self.source, self.kit_root)
        with self.assertRaises(rc.CheckError):
            rc.build(self.source, self.kit_root)
        rc.build(self.source, self.kit_root, force=True)
        other = self.tmp / "not-a-kit"
        other.mkdir()
        (other / "keep.txt").write_text("x")
        with self.assertRaises(rc.CheckError):
            rc.build(self.source, other, force=True)
        self.assertTrue((other / "keep.txt").exists())

    def test_too_few_files(self):
        small = self.tmp / "small"
        small.mkdir()
        (small / "a.mp3").write_bytes(b"x" * 4096)
        with self.assertRaises(rc.CheckError):
            rc.build(small, self.tmp / "kit2")


class Session:
    """Runs CHECK.md against a FakeRekordbox, like the owner would."""

    def __init__(self, case: KitTestCase, rb: FakeRekordbox, answer_no=(), skip_edit=False,
                 skip_playlist_import=False, heard=None):
        self.kit = rc.build(case.source, case.kit_root)
        self.rb, self.ex = rb, case.kit_root / "exports"
        self.answer_no = {rbxml.path_key(self.kit.path(lab)) for lab in answer_no}
        self.skip_edit = skip_edit
        self.skip_playlist_import = skip_playlist_import  # the owner skips CHECK.md step 3.4
        heard = {**{lab: cam for lab, (cam, *_) in rc.KEY_TONES.items()}, **(heard or {})}
        rb.heard = {self.kit.key(lab): cam for lab, cam in heard.items()}

    def first(self):
        kit, rb = self.kit, self.rb
        rb.drag([kit.path("D1"), kit.path("D2")])
        sent = rbxml.parse(kit.root / rc.FIRST_XML)
        rb.import_collection(sent)
        rb.import_playlists(sent, "TLP Check")
        rb.export(self.ex / rc.EXPORTS[0])
        if not self.skip_edit:
            rb.edit(str(kit.path("01")), Name="EDITED IN RB")
        rb.set_hot_cue(str(kit.path("04")))
        rb.export(self.ex / rc.EXPORTS[1])
        return self

    def rest(self):
        kit, rb = self.kit, self.rb
        rc.prepare_next(kit, self.ex / rc.EXPORTS[1])
        rb.auto_analysis = False
        resend = rbxml.parse(kit.root / rc.RESEND_XML)
        rb.import_collection(resend, answer=lambda p: rbxml.path_key(p) not in self.answer_no)
        if not self.skip_playlist_import:
            rb.import_playlists(resend, "TLP Check")
        rb.export(self.ex / rc.EXPORTS[2])
        rb.import_playlists(rbxml.parse(kit.root / rc.T1A_XML), "TLP T1a")
        rb.export(self.ex / rc.EXPORTS[3])
        t1b = rbxml.parse(kit.root / rc.T1B_XML)
        rb.import_collection(t1b)
        rb.import_playlists(t1b, "TLP T1b")
        rb.export(self.ex / rc.EXPORTS[4])
        return self

    def results(self):
        results, _ = rc.analyze(self.kit)
        return {r.name: r for r in results}


def by_prefix(results, prefix):
    return next(r for name, r in results.items() if name.startswith(prefix))


class Scenario(KitTestCase):
    def test_rekordbox_as_documented_passes_everything(self):
        results = Session(self, FakeRekordbox()).first().rest().results()
        bad = [f"{r.status} {r.name}: {r.detail}" for r in results.values() if r.status not in (PASS, rc.INFO)]
        self.assertEqual(bad, [])
        self.assertIn("TrackID (KeyType 0) and by Location (KeyType 1)", results["T1 verdict"].detail)

    def test_report_text(self):
        s = Session(self, FakeRekordbox()).first().rest()
        results, lib = rc.analyze(s.kit)
        text = rc.report(results, lib, s.kit)
        self.assertIn("rekordbox 7.2.19", text)
        self.assertIn("Summary:", text)
        with contextlib.redirect_stdout(io.StringIO()) as out:
            self.assertEqual(rc.main(["analyze", "--kit", str(s.kit.root)]), 0)
        self.assertIn("T3 verdict", out.getvalue())
        self.assertTrue((s.ex / "report.txt").exists())

    def test_partial_exports_are_not_run(self):
        s = Session(self, FakeRekordbox()).first()
        results = s.results()
        self.assertEqual(results["Awkward paths resolve"].status, PASS)
        self.assertEqual(results["Omitted Rating resets to 0"].status, NOTRUN)
        self.assertEqual(results["T3 verdict"].status, NOTRUN)
        self.assertEqual(results["T1 verdict"].status, NOTRUN)

    def test_t3_fails_when_omission_erases_analysis(self):
        results = Session(self, FakeRekordbox(erase_omitted_analysis=True)).first().rest().results()
        self.assertEqual(results["T3 verdict"].status, FAIL)
        self.assertEqual(by_prefix(results, "04 ").status, FAIL)
        self.assertIn("Tonality", by_prefix(results, "05 ").detail)
        self.assertEqual(by_prefix(results, "07 ").status, PASS)

    def test_t3_inconclusive_when_answered_no(self):
        results = Session(self, FakeRekordbox(), answer_no=["04"]).first().rest().results()
        self.assertEqual(by_prefix(results, "04 ").status, INCON)
        self.assertEqual(results["T3 verdict"].status, INCON)
        self.assertIn("04", results["Tracks the resend updated"].detail)

    def test_t1_fails_when_absent_tracks_dont_resolve(self):
        results = Session(self, FakeRekordbox(resolve_absent=False)).first().rest().results()
        self.assertEqual(by_prefix(results, "T1a by-id").status, FAIL)
        self.assertEqual(by_prefix(results, "T1a by-location").status, PASS)
        self.assertEqual(results["T1 verdict"].status, PASS)
        self.assertIn("Location", results["T1 verdict"].detail)
        self.assertNotIn("TrackID", results["T1 verdict"].detail)

    def test_t1_fails_when_nothing_resolves(self):
        results = Session(self, FakeRekordbox(resolve_absent=False, resolve_by_location=False)).first().rest().results()
        self.assertEqual(results["T1 verdict"].status, FAIL)

    def test_changed_rating_behavior_is_reported(self):
        results = Session(self, FakeRekordbox(keep_omitted_rating=True)).first().rest().results()
        self.assertEqual(results["Omitted Rating resets to 0"].status, FAIL)

    def test_changed_playcount_behavior_is_reported(self):
        results = Session(self, FakeRekordbox(keep_playcount=True)).first().rest().results()
        self.assertEqual(results['PlayCount="0" wipes plays'].status, FAIL)

    def test_duplicates_are_reported(self):
        results = Session(self, FakeRekordbox(duplicate_on_reimport=True)).first().rest().results()
        self.assertEqual(results["No duplicate tracks, ever"].status, FAIL)

    def test_next_refuses_an_export_without_the_edit(self):
        s = Session(self, FakeRekordbox(), skip_edit=True).first()
        with self.assertRaises(rc.CheckError) as err:
            rc.prepare_next(s.kit, s.ex / rc.EXPORTS[1])
        self.assertIn("step 2.1", str(err.exception))
        self.assertFalse((s.kit.root / rc.RESEND_XML).exists())

    def test_next_refuses_a_missing_export(self):
        kit = rc.build(self.source, self.kit_root)
        with self.assertRaises(rc.CheckError):
            rc.prepare_next(kit, kit.root / "exports" / rc.EXPORTS[1])

    def test_resend_follows_rules_1_3_5(self):
        s = Session(self, FakeRekordbox()).first()
        rc.prepare_next(s.kit, s.ex / rc.EXPORTS[1])
        before = rbxml.parse(s.ex / rc.EXPORTS[1])
        sent = rbxml.parse(s.kit.root / rc.RESEND_XML)
        k = s.kit.key
        for t in sent.tracks:
            self.assertEqual(t.tempos + t.marks, [])  # rule 3 (A)
            self.assertNotIn("#", t.attrs["Location"])  # rule 5
        control = sent.one(k("07")).attrs
        rb = before.one(k("07")).attrs
        self.assertEqual({f: control[f] for f in rb if f not in ("Location", "Comments")},
                         {f: rb[f] for f in rb if f not in ("Location", "Comments")})  # rule 1
        self.assertNotIn("Rating", sent.one(k("02")).attrs)
        self.assertEqual(sent.one(k("03")).attrs["PlayCount"], "0")
        self.assertFalse({"Tonality", "AverageBpm"} & set(sent.one(k("04")).attrs))
        self.assertNotIn("Tonality", sent.one(k("05")).attrs)
        self.assertIn("AverageBpm", sent.one(k("05")).attrs)
        self.assertNotIn("AverageBpm", sent.one(k("06")).attrs)
        self.assertIsNone(sent.one(k("09")))
        t1a = rbxml.parse(s.kit.root / rc.T1A_XML)
        self.assertEqual(t1a.tracks, [])  # playlists only

    def test_step_34_skipped_is_named_plainly(self):
        s = Session(self, FakeRekordbox(), skip_playlist_import=True).first().rest()
        results = s.results()
        step = results["Step 3.4 done (Import Playlist before export 03)"]
        self.assertEqual(step.status, FAIL)
        self.assertIn("step 3.4 (Import Playlist) was skipped", step.detail)
        self.assertIn("start again from step 0", step.detail)
        self.assertNotIn("export 03-after-resend.xml again", step.detail)  # that would spoil the T1 baseline
        for name in ("Same-name playlists are replaced", "Playlists missing from the XML stay"):
            self.assertEqual(results[name].status, INCON, name)
            self.assertIn("step 3.4", results[name].detail)
        t1 = [r for name, r in results.items() if name.startswith(("T1a ", "T1b "))]
        self.assertEqual(len(t1), 4)
        for r in t1:
            self.assertEqual(r.status, INCON, r.name)
            self.assertIn("isn't a clean starting point for T1", r.detail)
        self.assertEqual(results["T1 verdict"].status, INCON)
        self.assertEqual(results["Omitted Rating resets to 0"].status, PASS)  # the rest still runs
        with contextlib.redirect_stdout(io.StringIO()) as out:
            self.assertEqual(rc.main(["analyze", "--kit", str(s.kit.root)]), 1)
        self.assertIn("was skipped", out.getvalue())

    def test_step_34_done(self):
        results = Session(self, FakeRekordbox()).first().rest().results()
        self.assertEqual(results["Step 3.4 done (Import Playlist before export 03)"].status, PASS)

    def test_resend_sends_the_tones_camelot_keys(self):
        s = Session(self, FakeRekordbox()).first()
        rc.prepare_next(s.kit, s.ex / rc.EXPORTS[1])
        sent = rbxml.parse(s.kit.root / rc.RESEND_XML)
        self.assertEqual(sent.one(s.kit.key("K2B")).attrs["Tonality"], "6B")  # swapped, in Camelot
        self.assertEqual(sent.one(s.kit.key("K6B")).attrs["Tonality"], "2B")
        self.assertIn("AverageBpm", sent.one(s.kit.key("K2B")).attrs)  # everything else as rekordbox has it

    def test_key_spellings_reported(self):
        results = Session(self, FakeRekordbox()).first().rest().results()
        k2, k6 = by_prefix(results, "K2B 2B (F# major)"), by_prefix(results, "K6B 6B (Bb major)")
        self.assertEqual((k2.status, k6.status), (PASS, PASS))
        self.assertIn("'F#' for 2B", k2.detail)
        self.assertIn("'Bb' for 6B", k6.detail)
        seen = results["Key spellings rekordbox wrote (export 02)"].detail
        self.assertIn("F# (2B)", seen)
        self.assertIn("Bb (6B)", seen)
        self.assertEqual(results["Camelot key sent on a re-import (K2B)"].detail,
                         "sent '6B'; before 'F#', after '6B': rekordbox kept the string as sent")

    def test_other_spelling_fails(self):
        results = Session(self, FakeRekordbox(spellings={"2B": "Gb", "6B": "A#"})).first().rest().results()
        k2, k6 = by_prefix(results, "K2B "), by_prefix(results, "K6B ")
        self.assertEqual((k2.status, k6.status), (FAIL, FAIL))
        self.assertIn("'Gb' for 2B", k2.detail)
        self.assertIn("'A#' for 6B", k6.detail)

    def test_loaded_camelot_key_converted(self):
        results = Session(self, FakeRekordbox(convert_loaded_keys=True)).first().rest().results()
        self.assertEqual(results["Camelot key sent on a re-import (K2B)"].detail,
                         "sent '6B'; before 'F#', after 'Bb': rekordbox converted it to 'Bb', its Classic spelling of 6B")
        self.assertIn("converted it to 'F#'", results["Camelot key sent on a re-import (K6B)"].detail)

    def test_key_loaded_outcomes(self):
        t = lambda key: rbxml.Track(attrs={"Tonality": key})
        self.assertIn("ignored it", rc.key_loaded(t("6B"), t("F#"), t("F#")))
        self.assertIn("isn't the key sent", rc.key_loaded(t("6B"), t("F#"), t("Am")))
        self.assertIn("can't tell", rc.key_loaded(t("6B"), t("6B"), t("6B")))

    def test_tone_heard_in_another_key_is_inconclusive(self):
        results = Session(self, FakeRekordbox(), heard={"K2B": "2A"}).first().rest().results()
        k2 = by_prefix(results, "K2B ")
        self.assertEqual(k2.status, INCON)
        self.assertIn("heard Ebm (2A)", k2.detail)

    def test_camelot_display_warns_at_next_and_is_inconclusive(self):
        s = Session(self, FakeRekordbox(key_display="camelot")).first()
        warnings = rc.prepare_next(s.kit, s.ex / rc.EXPORTS[1])
        self.assertTrue(any("Classic (step 0.5)" in w and "'2B'" in w for w in warnings), warnings)
        s.rest()
        self.assertEqual(by_prefix(s.results(), "K2B ").status, INCON)

    def test_key_helpers(self):
        cases = {"F#": "2B", "Gb": "2B", "Bb": "6B", "A#": "6B", "Abm": "1A", "G#m": "1A", "Am": "8A",
                 "C": "8B", "E": "12B", "Dbm": "12A", "F#maj": "2B", "Bb minor": "3A", "2B": "2B", "12a": "12A",
                 "13B": None, "": None, "x": None, "f#": None}
        self.assertEqual({k: rc.camelot_of(k) for k in cases}, cases)
        for camelot, name in RB_CLASSIC.items():
            self.assertEqual(rc.camelot_of(name), camelot)
        self.assertTrue(rc.is_camelot(" 6B "))
        self.assertFalse(rc.is_camelot("Bb"))

    def test_camelot_shift(self):
        self.assertEqual(rc._camelot_shift("4A"), "10A")
        self.assertEqual(rc._camelot_shift("12B"), "6B")
        self.assertEqual(rc._camelot_shift("Fm"), "1A")


class ToneKey(unittest.TestCase):
    """The tones really are in their keys: the tonic is the strongest pitch class and nothing is off the scale."""

    @staticmethod
    def chroma(pc, seconds=0.5):
        samples = array.array("h", tones._bar(pc))[:int(tones.RATE * seconds)]  # a tonic chord: enough, and fast
        energy = [0.0] * 12
        for midi in range(36, 86):  # Goertzel at every note from C2 to C#6
            coeff = 2 * math.cos(2 * math.pi * 440 * 2 ** ((midi - 69) / 12) / tones.RATE)
            s1 = s2 = 0.0
            for x in samples:
                s1, s2 = x + coeff * s1 - s2, s1
            energy[midi % 12] += s1 * s1 + s2 * s2 - coeff * s1 * s2
        return energy

    def test_each_tone_is_in_its_key(self):
        for lab, (_, _, pc) in rc.KEY_TONES.items():
            energy = self.chroma(pc)
            self.assertEqual(energy.index(max(energy)), pc, lab)
            off_scale = [(pc + i) % 12 for i in (1, 3, 6, 8, 10)]
            self.assertLess(max(energy[i] for i in off_scale), max(energy) * 0.05, lab)


# ---------------------------------------------------------------- local only: the E1 session's real exports

REPO = HERE.parent.parent
RESULTS = Path(os.environ.get("TLP_RB_RESULTS", REPO / "spikes" / "results"))
E1 = ["a-after-v1.xml", "b-after-edits.xml", "c0-cues-set.xml", "c-after-replace.xml", "d-after-history.xml",
      "e-after-v2.xml", "e2-mytag-comments.xml", "f-ground-truth.xml"]


@unittest.skipUnless((RESULTS / "e-after-v2.xml").exists(), "local only: needs the private E1 exports")
class E1ExportsLocal(unittest.TestCase):
    """The same checks against rekordbox 7.2.19's real exports from the E1 session (a different kit)."""

    @classmethod
    def setUpClass(cls):
        cls.libs = {n: rbxml.parse(RESULTS / n) for n in E1 if (RESULTS / n).exists()}
        e = cls.libs["e-after-v2.xml"]
        cls.keys = {}
        for t in e.tracks:
            name = t.path.rsplit("/", 1)[-1]
            if "/tlp-rb-kit/" in t.path and name[:2].isdigit():
                cls.keys[f"t{int(name[:2])}"] = t.key
        cls.sent = {n: rbxml.parse(RESULTS.parent / "rb-kit" / n) for n in ("v1.xml", "v2.xml")
                    if (RESULTS.parent / "rb-kit" / n).exists()}

    def lab(self, *labels):
        return [f"t{i}" for i in labels]

    def one(self, export, label):
        return self.libs[export].one(self.keys[label])

    def test_every_location_round_trips_in_rekordbox_style(self):
        import xml.etree.ElementTree as ET
        count = 0
        for n in self.libs:
            for el in ET.parse(RESULTS / n).getroot().iter("TRACK"):
                loc = el.get("Location")
                if loc and loc.startswith(rbxml.PREFIX):
                    self.assertEqual(rbxml.encode_location_rekordbox_style(rbxml.decode_location(loc)), loc)
                    count += 1
        self.assertGreater(count, 300)

    def test_kit_found(self):
        self.assertEqual(len(self.keys), 8)
        keys = {k: v for k, v in self.keys.items() if k != "t8"}
        self.assertEqual(rc.paths_resolve(self.libs["a-after-v1.xml"], keys)[0], PASS)

    @unittest.skipUnless((RESULTS.parent / "rb-kit" / "v1.xml").exists(), "needs the E1 v1.xml")
    def test_first_import(self):
        keys = {k: v for k, v in self.keys.items() if k != "t8"}
        a, v1 = self.libs["a-after-v1.xml"], self.sent["v1.xml"]
        self.assertEqual(rc.ids_reassigned(v1, a, keys)[0], PASS)
        self.assertEqual(rc.xml_analysis_ignored(v1.one(self.keys["t4"]), a.one(self.keys["t4"]))[0], PASS)
        self.assertEqual(rc.xml_values_win(v1, a, keys)[0], PASS)

    def test_ids_stable_and_no_duplicates(self):
        libs = [(n, lib) for n, lib in self.libs.items() if n != "f-ground-truth.xml"]
        keys = {k: v for k, v in self.keys.items() if k != "t8"}
        self.assertEqual(rc.ids_stable(libs, keys)[0], PASS)
        self.assertEqual(rc.no_duplicates(list(self.libs.items()), self.keys)[0], PASS)

    def test_first_playlists(self):
        k = self.keys
        exp = {("TLP Kit", "kit-a"): self.lab(1, 2, 3, 6), ("TLP Kit", "kit-b"): self.lab(4, 5, 7),
               ("TLP Kit", "nested", "kit-deep"): self.lab(2, 1)}
        self.assertEqual(rc.playlists_match(self.libs["a-after-v1.xml"], exp, k,
                                            exact_folders=[("TLP Kit",)])[0], PASS)

    def test_reimport_behaviors(self):
        d, e = "d-after-history.xml", "e-after-v2.xml"
        o = self.one
        if "v2.xml" in self.sent:
            v2_name = self.sent["v2.xml"].one(self.keys["t1"]).attrs["Name"]
            self.assertEqual(rc.overwritten(o(d, "t1"), o(e, "t1"), "Name", v2_name)[0], PASS)
        self.assertEqual(rc.reset_by_send(o(d, "t6"), o(e, "t6"), "Rating")[0], PASS)
        self.assertEqual(rc.reset_by_send(o(d, "t1"), o(e, "t1"), "PlayCount",
                                          control=(o(d, "t4"), o(e, "t4")))[0], PASS)
        self.assertEqual(rc.still_there(o(d, "t5"), o(e, "t5"))[0], PASS)
        exp = {("TLP Kit", "kit-a"): self.lab(3, 2, 1), ("TLP Kit", "kit-b-renamed"): self.lab(4, 6, 7),
               ("TLP Kit", "kit-c"): self.lab(8), ("TLP Kit", "nested", "kit-deep"): self.lab(2, 1)}
        allowed = {**exp, ("TLP Kit", "kit-b"): self.lab(4, 5, 7)}
        self.assertEqual(rc.playlists_match(self.libs[e], exp, self.keys, exact_folders=[("TLP Kit",)],
                                            allowed=allowed)[0], PASS)
        self.assertEqual(rc.playlists_match(self.libs[e], {("TLP Kit", "kit-b"): self.lab(4, 5, 7)},
                                            self.keys)[0], PASS)

    def test_e1_hints_at_t3(self):
        """E1's v2 omitted Tonality/AverageBpm/TEMPO/POSITION_MARK for t1, t2, t3, t6, t7 and got "Yes".
        Everything was kept, but auto-analysis was on, so a silent re-analysis can't be ruled out:
        the check turns it off for that step."""
        for lab in ("t1", "t2", "t3", "t6", "t7"):
            status, detail = rc.analysis_kept(self.one("d-after-history.xml", lab),
                                              self.one("e-after-v2.xml", lab), ("Tonality", "AverageBpm"))
            self.assertEqual(status, PASS, f"{lab}: {detail}")


if __name__ == "__main__":
    unittest.main()
