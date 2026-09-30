//! The generated tree holds what the manifest says: valid audio that an
//! independent decoder (Symphonia) plays, traps broken in exactly the
//! intended way, skip files, awkward names written exactly, and duplicate,
//! version and unrelated cases whose audio really relates as claimed.

mod common;

use std::collections::{BTreeSet, HashSet};
use std::ffi::OsString;
use std::fs;

use common::{correlation, decode, shared, walk, Fixture, SHARED_BULK};
use fixture_gen::encode::{ape, m4a};
use serde_json::Value;

fn s(v: &Value) -> &str {
    v.as_str()
        .unwrap_or_else(|| panic!("expected a string, got {v}"))
}

fn read(fx: &Fixture, f: &Value) -> Vec<u8> {
    fs::read(fx.path(s(&f["path"]))).unwrap()
}

fn has_case(f: &Value, case: &str) -> bool {
    f["cases"].as_array().unwrap().iter().any(|c| c == case)
}

fn is_lossless(f: &Value) -> bool {
    matches!(f["format"].as_str(), Some("flac" | "wav" | "aiff" | "m4a"))
}

/// Symphonia's name for each container.
fn container_name(format: &str) -> &'static str {
    match format {
        "mp3" => "mp3",
        "flac" => "flac",
        "wav" => "wave",
        "aiff" => "aiff",
        "m4a" => "isomp4",
        other => panic!("unknown format {other}"),
    }
}

// ---- Valid audio --------------------------------------------------------

#[test]
fn every_file_marked_decodable_decodes_from_its_bytes_to_exactly_its_recorded_length() {
    let fx = shared();
    let mut checked = 0;
    for f in fx.files().iter().filter(|f| f["decodes"] == true) {
        let path = s(&f["path"]);
        let d = decode(&fx.path(path)).unwrap_or_else(|e| panic!("{path}: {e}"));
        assert_eq!(
            d.container,
            container_name(s(&f["format"])),
            "{path}: detected format"
        );
        assert_eq!(
            Some(u64::from(d.rate)),
            f["sample_rate"].as_u64(),
            "{path}: rate"
        );
        assert_eq!(d.channels, 1, "{path}: channels");
        assert_eq!(
            Some(d.samples.len() as u64),
            f["frames"].as_u64(),
            "{path}: decoded length"
        );
        assert!(d.samples.iter().any(|&x| x != 0), "{path}: all silence");
        checked += 1;
    }
    assert!(checked > 100, "only {checked} files checked");
}

#[test]
fn files_marked_not_decodable_really_fail_to_decode() {
    let fx = shared();
    let broken: Vec<_> = fx
        .files()
        .iter()
        .filter(|f| f["decodes"] == false)
        .collect();
    assert!(!broken.is_empty());
    for f in broken {
        let path = s(&f["path"]);
        assert!(decode(&fx.path(path)).is_err(), "{path} decoded");
    }
}

#[test]
fn every_format_appears_in_both_the_core_cases_and_the_bulk_files() {
    let fx = shared();
    for bulk in [false, true] {
        let formats: BTreeSet<&str> = fx
            .files()
            .iter()
            .filter(|f| f["role"] == "audio" && has_case(f, "bulk") == bulk)
            .map(|f| s(&f["format"]))
            .collect();
        assert_eq!(
            formats,
            BTreeSet::from(["aiff", "flac", "m4a", "mp3", "wav"]),
            "bulk = {bulk}"
        );
    }
}

#[test]
fn the_manifest_records_each_files_true_size() {
    let fx = shared();
    for f in fx.files() {
        let path = s(&f["path"]);
        let len = fs::metadata(fx.path(path)).unwrap().len();
        assert_eq!(Some(len), f["bytes"].as_u64(), "{path}");
    }
}

// ---- Traps ---------------------------------------------------------------

fn trap<'a>(fx: &'a Fixture, kind: &str) -> &'a Value {
    let found: Vec<_> = fx
        .files()
        .iter()
        .filter(|f| f["trap"]["kind"] == kind)
        .collect();
    assert_eq!(found.len(), 1, "expected one {kind} trap");
    assert_eq!(found[0]["role"], "trap");
    found[0]
}

#[test]
fn every_trap_kind_is_present_once() {
    let fx = shared();
    let kinds: Vec<&str> = fx
        .files()
        .iter()
        .filter(|f| f["role"] == "trap")
        .map(|f| s(&f["trap"]["kind"]))
        .collect();
    let mut sorted = kinds.clone();
    sorted.sort();
    assert_eq!(
        sorted,
        [
            "bad_tdrc",
            "broken_ape",
            "empty_file",
            "m4a_no_moov",
            "mp3_in_wav"
        ]
    );
}

#[test]
fn the_wav_trap_is_mp3_bytes_under_a_wav_name() {
    let fx = shared();
    let f = trap(fx, "mp3_in_wav");
    assert!(s(&f["path"]).ends_with(".wav"));
    let bytes = read(fx, f);
    assert_ne!(&bytes[..4], b"RIFF", "it's a real WAV");
    assert_eq!(&bytes[..3], b"ID3");
    assert_eq!(decode(&fx.path(s(&f["path"]))).unwrap().container, "mp3");
}

#[test]
fn the_m4a_trap_has_its_audio_but_no_moov_box() {
    let fx = shared();
    let f = trap(fx, "m4a_no_moov");
    let bytes = read(fx, f);
    assert_eq!(m4a::top_level_boxes(&bytes), [*b"ftyp", *b"mdat"]);
    assert!(bytes.len() > 100_000, "the audio data is there");
    // A good M4A from the same tree does have one, so the check means something.
    let good = fx
        .files()
        .iter()
        .find(|f| f["format"] == "m4a" && f["decodes"] == true)
        .unwrap();
    assert!(m4a::top_level_boxes(&read(fx, good)).contains(b"moov"));
    let err = decode(&fx.path(s(&f["path"]))).unwrap_err();
    assert!(err.contains("moov"), "{err}");
}

/// The text of an ID3v2.4 frame, from a tag at the start of `bytes`.
fn id3v24_text(bytes: &[u8], id: &[u8; 4]) -> Option<String> {
    assert_eq!(&bytes[..4], b"ID3\x04");
    let syncsafe = |b: &[u8]| b.iter().fold(0usize, |acc, &x| (acc << 7) | usize::from(x));
    let end = 10 + syncsafe(&bytes[6..10]);
    let mut at = 10;
    while at + 10 <= end {
        let size = syncsafe(&bytes[at + 4..at + 8]);
        if &bytes[at..at + 4] == id {
            let body = &bytes[at + 10..at + 10 + size];
            assert_eq!(body[0], 3, "UTF-8");
            return Some(String::from_utf8(body[1..].to_vec()).unwrap());
        }
        at += 10 + size;
    }
    None
}

#[test]
fn the_bad_date_trap_holds_an_impossible_tdrc_and_its_audio_still_decodes() {
    let fx = shared();
    let f = trap(fx, "bad_tdrc");
    let date = id3v24_text(&read(fx, f), b"TDRC").expect("a TDRC frame");
    let parts: Vec<u32> = date.split('-').map(|p| p.parse().unwrap()).collect();
    assert!(parts[1] > 12, "month {} is valid", parts[1]);
    assert!(parts[2] > 31, "day {} is valid", parts[2]);
    // The rest of the tag is fine, and so is the audio.
    assert!(id3v24_text(&read(fx, f), b"TIT2").is_some());
    assert_eq!(f["decodes"], true);
    decode(&fx.path(s(&f["path"]))).unwrap();
}

#[test]
fn the_broken_ape_trap_claims_more_than_the_file_holds_and_its_audio_still_decodes() {
    let fx = shared();
    let f = trap(fx, "broken_ape");
    let bytes = read(fx, f);
    let (size, items) = ape::read_footer(&bytes).expect("an APE footer at the end");
    assert!(
        size as usize > bytes.len(),
        "claims {size} bytes of a {}-byte file",
        bytes.len()
    );
    let present = bytes.windows(6).filter(|w| w == b"Title\0").count();
    assert_eq!(present, 1);
    assert!(items > 1, "claims {items} items; one is there");
    assert_eq!(f["decodes"], true);
    decode(&fx.path(s(&f["path"]))).unwrap();
}

#[test]
fn the_empty_file_trap_is_zero_bytes() {
    let fx = shared();
    let f = trap(fx, "empty_file");
    assert!(read(fx, f).is_empty());
}

// ---- Skips ---------------------------------------------------------------

#[test]
fn skip_files_are_real_appledouble_and_ds_store_files_among_the_music() {
    let fx = shared();
    let skips: Vec<_> = fx.files().iter().filter(|f| f["role"] == "skip").collect();
    let kinds: BTreeSet<&str> = skips.iter().map(|f| s(&f["skip"])).collect();
    assert_eq!(kinds, BTreeSet::from(["apple_double", "ds_store"]));
    for f in skips {
        let path = s(&f["path"]);
        let name = path.rsplit('/').next().unwrap();
        let bytes = read(fx, f);
        match s(&f["skip"]) {
            "apple_double" => {
                assert!(name.starts_with("._"), "{path}");
                assert_eq!(
                    &bytes[..4],
                    [0x00, 0x05, 0x16, 0x07],
                    "{path}: AppleDouble magic"
                );
                // It shadows a real audio file next to it, as macOS leaves them.
                let sibling = format!("{}/{}", path.rsplit_once('/').unwrap().0, &name[2..]);
                assert_eq!(fx.file(&sibling)["role"], "audio", "{sibling}");
            }
            "ds_store" => {
                assert_eq!(name, ".DS_Store");
                assert_eq!(&bytes[4..8], b"Bud1", "{path}");
            }
            other => panic!("unknown skip {other}"),
        }
    }
}

// ---- Awkward names -------------------------------------------------------

/// The exact names in a folder, as the OS lists them.
fn listing(fx: &Fixture, dir: &str) -> BTreeSet<OsString> {
    fs::read_dir(fx.path(dir))
        .unwrap_or_else(|e| panic!("{dir}: {e}"))
        .map(|e| e.unwrap().file_name())
        .collect()
}

#[test]
fn every_file_is_on_disk_under_exactly_the_name_the_manifest_gives() {
    let fx = shared();
    for f in fx.files() {
        let path = s(&f["path"]);
        let (dir, name) = path.rsplit_once('/').unwrap();
        assert!(
            listing(fx, dir).contains(&OsString::from(name)),
            "{path}: not listed under that exact name"
        );
    }
}

#[test]
fn the_tree_holds_nothing_the_manifest_leaves_out() {
    let fx = shared();
    let on_disk: BTreeSet<String> = walk(&fx.root)
        .into_iter()
        .map(|(parts, _)| {
            parts
                .iter()
                .map(|p| p.to_str().unwrap().to_owned())
                .collect::<Vec<_>>()
                .join("/")
        })
        .collect();
    let mut listed: BTreeSet<String> = fx
        .files()
        .iter()
        .map(|f| s(&f["path"]).to_owned())
        .collect();
    listed.insert("fixture-manifest.json".into());
    assert_eq!(on_disk, listed);
}

#[test]
fn trailing_dot_and_trailing_space_folders_sit_beside_look_alike_siblings() {
    let fx = shared();
    let music = listing(fx, "music");
    for (odd, plain) in [("Q.V.X.", "Q.V.X"), ("Drift Unit ", "Drift Unit")] {
        assert!(music.contains(&OsString::from(odd)), "{odd:?} missing");
        assert!(music.contains(&OsString::from(plain)), "{plain:?} missing");
    }
    // Same file names inside, different files: opening the wrong one is caught.
    let a = fs::read(fx.path("music/Q.V.X./Monolith/Luma.mp3")).unwrap();
    let b = fs::read(fx.path("music/Q.V.X/Monolith/Luma.mp3")).unwrap();
    assert_ne!(a, b);
    let a = fs::read(fx.path("music/Drift Unit /Halvane.flac")).unwrap();
    let b = fs::read(fx.path("music/Drift Unit/Halvane.flac")).unwrap();
    assert_ne!(a, b);
}

#[test]
fn reserved_device_names_are_ordinary_audio_files() {
    let fx = shared();
    for name in ["CON.mp3", "AUX.mp3"] {
        let path = format!("music/Misc/{name}");
        assert!(listing(fx, "music/Misc").contains(&OsString::from(name)));
        assert!(fs::metadata(fx.path(&path)).unwrap().is_file());
        assert_eq!(decode(&fx.path(&path)).unwrap().container, "mp3", "{path}");
    }
}

#[test]
fn nfc_and_nfd_twins_are_two_distinct_files() {
    let fx = shared();
    let nfc = "Caf\u{e9} Vireo.mp3";
    let nfd = "Cafe\u{301} Vireo.mp3";
    let misc = listing(fx, "music/Misc");
    assert!(misc.contains(&OsString::from(nfc)));
    assert!(misc.contains(&OsString::from(nfd)));
    let a = fs::read(fx.path(&format!("music/Misc/{nfc}"))).unwrap();
    let b = fs::read(fx.path(&format!("music/Misc/{nfd}"))).unwrap();
    assert_ne!(a, b, "the twins are different recordings");
}

#[test]
fn names_cover_emoji_cjk_url_specials_and_a_path_past_max_path() {
    let fx = shared();
    let names: Vec<&str> = fx
        .with_case("awkward-name")
        .iter()
        .map(|f| s(&f["path"]))
        .collect();
    let any = |pred: &dyn Fn(&str) -> bool| names.iter().any(|n| pred(n));
    assert!(any(&|n| n.chars().any(|c| c as u32 >= 0x1F300)), "emoji");
    assert!(any(&|n| n.contains('\u{200d}')), "a ZWJ emoji sequence");
    assert!(
        any(&|n| n.chars().any(|c| ('\u{3040}'..='\u{9fff}').contains(&c))),
        "Japanese/Chinese"
    );
    assert!(
        any(&|n| n.chars().any(|c| ('\u{ac00}'..='\u{d7a3}').contains(&c))),
        "Korean"
    );
    for c in ['#', '%', '+', '&', '\''] {
        assert!(any(&|n| n.contains(c)), "{c}");
    }
    assert!(any(&|n| n.contains("%20")), "a literal %20");
    assert!(any(&|n| n.ends_with(".MP3")), "an uppercase extension");
    assert!(any(&|n| n.len() > 260), "a path over 260 characters");
}

// ---- Duplicates ----------------------------------------------------------

fn groups(fx: &Fixture) -> Vec<(String, Vec<&Value>)> {
    fx.manifest["duplicate_groups"]
        .as_array()
        .unwrap()
        .iter()
        .map(|g| {
            let files = g["files"]
                .as_array()
                .unwrap()
                .iter()
                .map(|p| fx.file(s(p)))
                .collect();
            (s(&g["recording"]).to_owned(), files)
        })
        .collect()
}

#[test]
fn duplicates_come_in_every_format_and_several_bitrates() {
    let fx = shared();
    let (_, glass) = groups(fx)
        .into_iter()
        .find(|(r, _)| r == "glasswing")
        .unwrap();
    let formats: BTreeSet<&str> = glass.iter().map(|f| s(&f["format"])).collect();
    assert_eq!(
        formats,
        BTreeSet::from(["aiff", "flac", "m4a", "mp3", "wav"])
    );
    let bitrates: BTreeSet<u64> = glass
        .iter()
        .filter_map(|f| f["bitrate_kbps"].as_u64())
        .collect();
    assert!(bitrates.len() >= 2, "{bitrates:?}");
    let titles: BTreeSet<&str> = glass
        .iter()
        .filter_map(|f| f["tags"]["title"].as_str())
        .collect();
    assert!(titles.len() >= 2, "tags differ between copies: {titles:?}");
}

#[test]
fn lossless_copies_of_a_recording_decode_to_identical_samples() {
    let fx = shared();
    let mut compared = 0;
    for (rec, files) in groups(fx) {
        let lossless: Vec<_> = files.iter().filter(|f| is_lossless(f)).collect();
        let Some(first) = lossless.first() else {
            continue;
        };
        let reference = decode(&fx.path(s(&first["path"]))).unwrap().samples;
        for f in &lossless[1..] {
            let d = decode(&fx.path(s(&f["path"]))).unwrap().samples;
            assert!(d == reference, "{rec}: {} differs", f["path"]);
            compared += 1;
        }
    }
    assert!(compared >= 3, "only {compared} comparisons");
}

#[test]
fn lossy_copies_of_a_recording_match_the_lossless_audio_closely() {
    let fx = shared();
    let mut compared = 0;
    for (rec, files) in groups(fx) {
        let Some(reference) = files.iter().find(|f| is_lossless(f)) else {
            continue;
        };
        let reference = decode(&fx.path(s(&reference["path"]))).unwrap();
        for f in files.iter().filter(|f| f["format"] == "mp3") {
            let d = decode(&fx.path(s(&f["path"]))).unwrap();
            // A different rip starts later; compare from where the audio starts.
            let lead_in = d.samples.len().saturating_sub(reference.samples.len());
            let c = correlation(&reference.samples, &d.samples, lead_in);
            assert!(c > 0.97, "{rec}: {} correlates only {c:.3}", f["path"]);
            compared += 1;
        }
    }
    assert!(compared >= 8, "only {compared} comparisons");
}

#[test]
fn byte_copies_match_their_source_exactly() {
    let fx = shared();
    let copies: Vec<_> = fx
        .files()
        .iter()
        .filter(|f| f["byte_copy_of"].is_string())
        .collect();
    assert!(copies.len() >= 5);
    for f in copies {
        let src = fx.file(s(&f["byte_copy_of"]));
        assert_eq!(read(fx, f), read(fx, src), "{}", f["path"]);
        assert_eq!(f["recording"], src["recording"]);
    }
}

#[test]
fn the_gig_stick_copy_uses_rekordboxs_contents_artist_album_layout() {
    let fx = shared();
    let gig = fx.with_case("gig-stick-copy");
    assert!(gig.len() >= 5);
    for f in gig {
        let parts: Vec<&str> = s(&f["path"]).split('/').collect();
        // music / <folder> / Contents / Artist / Album / file
        assert_eq!(parts.len(), 6, "{}", f["path"]);
        assert_eq!(parts[2], "Contents", "{}", f["path"]);
    }
}

#[test]
fn a_retagged_copy_has_identical_mp3_frames_and_a_different_tag() {
    let fx = shared();
    let retag = fx.with_case("same-audio-stream");
    assert_eq!(retag.len(), 1);
    let a = read(fx, retag[0]);
    let b = read(
        fx,
        fx.file("music/Downloads/Nemora Vale - Glasswing (320).mp3"),
    );
    let tag_len = |x: &[u8]| {
        10 + x[6..10]
            .iter()
            .fold(0usize, |acc, &v| (acc << 7) | usize::from(v))
    };
    assert_eq!(a[tag_len(&a)..], b[tag_len(&b)..], "audio frames differ");
    assert_ne!(a[..tag_len(&a)], b[..tag_len(&b)], "tags are the same");
}

// ---- Versions and unrelated pairs ---------------------------------------

fn decode_recording(fx: &Fixture, rec: &str) -> Vec<i16> {
    let files = fx.files_of(rec);
    let f = files.iter().find(|f| is_lossless(f)).unwrap_or(&files[0]);
    decode(&fx.path(s(&f["path"]))).unwrap().samples
}

fn links(fx: &Fixture) -> &Vec<Value> {
    fx.manifest["version_links"].as_array().unwrap()
}

#[test]
fn versions_cover_cuts_and_every_kind_of_rework() {
    let fx = shared();
    let labels: BTreeSet<(&str, &str)> = links(fx)
        .iter()
        .map(|l| (s(&l["kind"]), s(&l["label"])))
        .collect();
    for want in [
        ("cut", "extended"),
        ("cut", "radio edit"),
        ("cut", "clean/dirty"),
        ("rework", "remix"),
        ("rework", "vip"),
        ("rework", "bootleg"),
        ("rework", "flip"),
        ("rework", "cover"),
        ("rework", "live"),
        ("rework", "mashup"),
    ] {
        assert!(labels.contains(&want), "{want:?} missing");
    }
}

#[test]
fn a_cut_sits_sample_for_sample_inside_its_longer_version() {
    let fx = shared();
    let mut checked = 0;
    for l in links(fx) {
        let Some(c) = l.get("contains") else { continue };
        let outer = decode_recording(fx, s(&c["outer"]));
        let inner = decode_recording(fx, s(&c["inner"]));
        let at = c["at_frame"].as_u64().unwrap() as usize;
        assert!(inner.len() < outer.len());
        // Lossless both sides is exact; an MP3 side is compared by shape.
        let corr = correlation(&inner, &outer, at);
        assert!(corr > 0.97, "{}: {corr:.3} at frame {at}", l["label"]);
        assert!(
            correlation(&inner, &outer, 0) < 0.8,
            "{}: also matches at 0",
            l["label"]
        );
        checked += 1;
    }
    assert_eq!(checked, 2);
    // Extended vs Original are both FLAC: the match is exact.
    let ext = decode_recording(fx, "paper-harbor-extended");
    let orig = decode_recording(fx, "paper-harbor-original");
    let at = links(fx).iter().find(|l| l["label"] == "extended").unwrap()["contains"]["at_frame"]
        .as_u64()
        .unwrap() as usize;
    assert!(ext[at..at + orig.len()] == orig[..]);
}

#[test]
fn clean_and_dirty_cuts_are_the_same_length_and_nearly_the_same_audio() {
    let fx = shared();
    for l in links(fx).iter().filter(|l| l["label"] == "clean/dirty") {
        let a = decode_recording(fx, s(&l["a"]));
        let b = decode_recording(fx, s(&l["b"]));
        assert_eq!(a.len(), b.len());
        assert!(a != b);
        let c = correlation(&a, &b, 0);
        assert!(c > 0.9, "{} vs {}: {c:.3}", l["a"], l["b"]);
    }
}

#[test]
fn reworks_share_little_audio_with_what_they_rework() {
    let fx = shared();
    for l in links(fx).iter().filter(|l| l["kind"] == "rework") {
        let a = decode_recording(fx, s(&l["a"]));
        let b = decode_recording(fx, s(&l["b"]));
        let c = correlation(&a, &b, 0);
        assert!(c < 0.8, "{}: {c:.3}", l["label"]);
    }
}

#[test]
fn the_live_version_has_the_originals_exact_tags_but_different_audio() {
    let fx = shared();
    let live = &fx.files_of("paper-harbor-live")[0];
    let original = &fx.files_of("paper-harbor-original")[0];
    assert_eq!(live["tags"], original["tags"]);
    assert!(
        decode_recording(fx, "paper-harbor-live") != decode_recording(fx, "paper-harbor-original")
    );
}

#[test]
fn unrelated_pairs_share_no_audio() {
    let fx = shared();
    let pairs = fx.manifest["not_related"].as_array().unwrap();
    assert!(pairs.len() >= 2);
    for p in pairs {
        let a = decode_recording(fx, s(&p["a"]));
        let b = decode_recording(fx, s(&p["b"]));
        let c = correlation(&a, &b, 0);
        assert!(c < 0.3, "{} vs {}: {c:.3}", p["a"], p["b"]);
    }
}

// ---- The manifest itself ------------------------------------------------

#[test]
fn every_id_the_manifest_mentions_is_a_recording_with_files() {
    let fx = shared();
    let ids: HashSet<&str> = fx.manifest["recordings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| s(&r["id"]))
        .collect();
    for id in &ids {
        assert!(!fx.files_of(id).is_empty(), "{id} has no files");
    }
    let groups = groups(fx);
    let mentioned = links(fx)
        .iter()
        .chain(fx.manifest["not_related"].as_array().unwrap())
        .flat_map(|l| [s(&l["a"]), s(&l["b"])])
        .chain(groups.iter().map(|(r, _)| r.as_str()));
    for id in mentioned {
        assert!(ids.contains(id), "{id} isn't a recording");
    }
    for f in fx.files() {
        if let Some(rec) = f["recording"].as_str() {
            assert!(
                ids.contains(rec) || has_case(f, "bulk"),
                "{}: {rec}",
                f["path"]
            );
        }
    }
}

#[test]
fn a_recording_with_several_files_is_a_duplicate_group_and_nothing_else_is() {
    let fx = shared();
    let listed: BTreeSet<String> = groups(fx).into_iter().map(|(r, _)| r).collect();
    let mut counts = std::collections::BTreeMap::<&str, usize>::new();
    for f in fx.files() {
        if let Some(r) = f["recording"].as_str() {
            *counts.entry(r).or_default() += 1;
        }
    }
    let multi: BTreeSet<String> = counts
        .into_iter()
        .filter(|&(_, n)| n > 1)
        .map(|(r, _)| r.to_owned())
        .collect();
    assert_eq!(listed, multi);
}

#[test]
fn bulk_files_are_all_distinct_recordings_and_stay_tiny() {
    let fx = shared();
    let bulk = fx.with_case("bulk");
    assert_eq!(bulk.len(), SHARED_BULK);
    assert_eq!(fx.manifest["counts"]["bulk"], SHARED_BULK);
    let recs: HashSet<&str> = bulk.iter().map(|f| s(&f["recording"])).collect();
    assert_eq!(recs.len(), bulk.len());
    let contents: HashSet<Vec<u8>> = bulk.iter().map(|f| read(fx, f)).collect();
    assert_eq!(
        contents.len(),
        bulk.len(),
        "two bulk files have the same bytes"
    );
    for f in bulk {
        assert!(
            f["bytes"].as_u64().unwrap() < 40_000,
            "{} is {} bytes",
            f["path"],
            f["bytes"]
        );
    }
}
