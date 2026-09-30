//! 1aC-1: the partial hash covers every byte of metadata and the edges of
//! the audio, and has no answer for a file it can't plan.

use std::io::Cursor;

use super::fixtures::*;
use crate::hash::partial::{
    partial_hash, PartialHash, EDGE, MAX_METADATA, PARTIAL_DEFINITION, PARTIAL_HASH_LEN,
};
use crate::tags::test_audio::{self, id3_text, id3v1_tag, id3v2};

fn partial(bytes: &[u8]) -> Option<PartialHash> {
    partial_hash(&mut Cursor::new(bytes), bytes.len() as u64).unwrap()
}

fn some(bytes: &[u8]) -> PartialHash {
    partial(bytes).expect("the file has a partial hash")
}

/// Bytes that aren't all alike.
fn filler(n: usize, seed: u8) -> Vec<u8> {
    (0..n)
        .map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed) ^ (i >> 8) as u8)
        .collect()
}

/// About 400 KB of MPEG frames: more than two edges, so the middle of the
/// audio isn't sampled.
fn long_audio() -> Vec<u8> {
    mp3_frames().repeat(40)
}

/// An ID3v2 tag whose comment is `len` bytes long, with `mark` at `at`.
fn tag_with_artwork(len: usize, at: usize, mark: char) -> Vec<u8> {
    let mut text = "x".repeat(len);
    text.replace_range(at..=at, &mark.to_string());
    id3v2(&[id3_text(b"COMM", &text)], 0)
}

fn mp3(tag: &[u8], audio: &[u8], trailer: &[u8]) -> Vec<u8> {
    [tag, audio, trailer].concat()
}

fn with_byte_changed(mut bytes: Vec<u8>, at: usize) -> Vec<u8> {
    bytes[at] ^= 0x55;
    bytes
}

#[test]
fn a_partial_hash_is_the_definition_then_a_32_byte_digest() {
    let hash = some(&test_audio::mp3());
    assert_eq!(hash.len(), PARTIAL_HASH_LEN);
    assert_eq!(hash[0], PARTIAL_DEFINITION);
}

#[test]
fn the_same_bytes_give_the_same_partial_hash() {
    let file = mp3(&tag_with_artwork(1000, 5, 'y'), &long_audio(), &[]);
    assert_eq!(some(&file), some(&file.clone()));
}

#[test]
fn a_same_size_tag_edit_inside_big_artwork_is_caught() {
    // 300 KB of artwork: far more than any fixed window at the start.
    let audio = long_audio();
    let before = mp3(&tag_with_artwork(300_000, 150_000, 'y'), &audio, &[]);
    let after = mp3(&tag_with_artwork(300_000, 150_000, 'z'), &audio, &[]);
    assert_eq!(before.len(), after.len());
    assert_ne!(some(&before), some(&after));
}

#[test]
fn a_same_size_edit_of_a_tag_at_the_end_is_caught() {
    let audio = long_audio();
    let tag = tag_with_artwork(200, 3, 'y');
    let before = mp3(&tag, &audio, &id3v1_tag("Night Drive"));
    let after = mp3(&tag, &audio, &id3v1_tag("Night Dries"));
    assert_eq!(before.len(), after.len());
    assert_ne!(some(&before), some(&after));
}

#[test]
fn a_tag_that_moves_the_audio_is_a_different_partial_hash() {
    let audio = long_audio();
    let before = mp3(&id3v2(&[id3_text(b"TIT2", "Night Drive")], 0), &audio, &[]);
    let after = mp3(
        &id3v2(&[id3_text(b"TIT2", "Night Drive")], 100),
        &audio,
        &[],
    );
    assert_ne!(some(&before), some(&after));
}

#[test]
fn a_change_at_either_edge_of_the_audio_is_caught() {
    let tag = tag_with_artwork(500, 7, 'y');
    let audio = long_audio();
    let file = mp3(&tag, &audio, &[]);
    let base = some(&file);
    let first_frame = tag.len() + 100;
    let last_frame = tag.len() + audio.len() - 100;
    for at in [
        first_frame,
        tag.len() + EDGE as usize - 1,
        tag.len() + audio.len() - EDGE as usize,
        last_frame,
    ] {
        assert_ne!(
            some(&with_byte_changed(file.clone(), at)),
            base,
            "byte {at} of the audio isn't covered"
        );
    }
}

#[test]
fn a_same_size_edit_in_the_middle_of_the_audio_is_the_known_blind_spot() {
    // The partial hash reads the audio's two edges, not all of it. An edit of
    // the same length away from both ends looks like a touch: the stages
    // don't redo the file until its size or an edge changes, or a stage
    // version is bumped. Tags never edit like this (see the module docs of
    // `hash::partial`); reading all the audio would defeat the shortcut.
    let tag = tag_with_artwork(500, 7, 'y');
    let audio = long_audio();
    assert!(audio.len() > 4 * EDGE as usize, "the middle is unsampled");
    let file = mp3(&tag, &audio, &[]);
    let middle = tag.len() + audio.len() / 2;
    assert_eq!(
        some(&with_byte_changed(file.clone(), middle)),
        some(&file),
        "if the middle is sampled now, update the docs and this test"
    );
}

#[test]
fn a_file_of_another_length_never_matches() {
    let tag = tag_with_artwork(500, 7, 'y');
    let audio = long_audio();
    let shorter = mp3(&tag, &audio[..audio.len() - 417], &[]);
    assert_ne!(some(&shorter), some(&mp3(&tag, &audio, &[])));
}

#[test]
fn audio_shorter_than_two_edges_is_hashed_whole() {
    // 24 frames, about 10 KB: a change anywhere in it shows.
    let audio = mp3_frames();
    let file = mp3(&[], &audio, &[]);
    let base = some(&file);
    for at in [500, audio.len() / 2, audio.len() - 30] {
        assert_ne!(
            some(&with_byte_changed(file.clone(), at)),
            base,
            "byte {at}"
        );
    }
}

#[test]
fn audio_in_several_mp4_mdat_boxes_is_read_as_one_stream_by_its_two_edges() {
    // Two `mdat` boxes of 100 KB: 200 KB of audio, its first 64 KiB in the
    // first box and its last 64 KiB in the second, and unsampled between.
    let boxes = m4a_boxes();
    let part = |id: &[u8; 4]| {
        let body = &boxes.iter().find(|(b, _)| b == id).unwrap().1;
        mp4_box(id, body)
    };
    let (one, two) = (filler(100_000, 1), filler(100_000, 2));
    let file = |one: &[u8], two: &[u8]| {
        [
            part(b"ftyp"),
            part(b"moov"),
            mp4_box(b"mdat", one),
            mp4_box(b"mdat", two),
        ]
        .concat()
    };
    let base = some(&file(&one, &two));
    let start = file(&[], &[]).len();
    // Inside the first edge, in the first box.
    let first = with_byte_changed(file(&one, &two), start + 8 + 10_000);
    // Inside the last edge, in the second box.
    let last = with_byte_changed(file(&one, &two), start + 8 + 100_000 + 8 + 80_000);
    // Between the edges, at the join of the two boxes.
    let between = with_byte_changed(file(&one, &two), start + 8 + 99_999);
    assert_ne!(some(&first), base, "the first edge");
    assert_ne!(some(&last), base, "the last edge");
    assert_eq!(some(&between), base, "the unsampled middle");
}

#[test]
fn every_supported_container_gets_a_partial_hash_and_a_metadata_edit_changes_it() {
    // The same samples with a different tag, in every layout the audio_hash
    // fixtures cover, differ only in metadata: the partial hashes differ too.
    type Layouts = Vec<(&'static str, Vec<u8>)>;
    let groups: [(&str, Layouts); 4] = [
        ("wav", wav_variants()),
        ("aiff", aiff_variants()),
        ("flac", flac_variants()),
        ("mp4", mp4_variants()),
    ];
    for (format, variants) in groups {
        let hashes: Vec<PartialHash> = variants
            .iter()
            .map(|(name, bytes)| {
                partial(bytes).unwrap_or_else(|| panic!("{format}: {name} has no partial hash"))
            })
            .collect();
        for (i, a) in hashes.iter().enumerate() {
            for b in &hashes[i + 1..] {
                assert_ne!(a, b, "{format}: two layouts share a partial hash");
            }
        }
    }
    for (name, bytes) in [
        ("mp3", test_audio::mp3()),
        ("wav", test_audio::wav()),
        ("aiff", test_audio::aiff()),
        ("flac", test_audio::flac()),
        ("m4a", test_audio::m4a()),
    ] {
        assert!(partial(&bytes).is_some(), "{name}");
    }
}

/// The generated M4A with its title set in `moov`, after the samples.
fn mp4_titled(title: &str) -> Vec<u8> {
    [
        m4a_as(&[b"ftyp", b"mdat"], &[]),
        mp4_box(b"moov", &moov_with_ilst(title)),
    ]
    .concat()
}

#[test]
fn a_same_size_metadata_edit_is_caught_in_every_container() {
    let id3_a = id3v2(&[id3_text(b"TIT2", "Night Drive")], 512);
    let id3_b = id3v2(&[id3_text(b"TIT2", "Night Dries")], 512);
    assert_eq!(id3_a.len(), id3_b.len());
    let (fmt, data) = (
        chunk(b"fmt ", &wav_fmt(), false),
        chunk(b"data", &samples(), false),
    );
    let wav = |id3: &[u8]| {
        riff(
            b"RIFF",
            &[fmt.clone(), data.clone(), chunk(b"id3 ", id3, false)],
        )
    };
    let (comm, ssnd) = (
        chunk(b"COMM", &aiff_comm(), true),
        chunk(b"SSND", &ssnd(0), true),
    );
    let aiff = |id3: &[u8]| {
        form(
            b"AIFF",
            &[comm.clone(), ssnd.clone(), chunk(b"ID3 ", id3, true)],
        )
    };
    let flac = |title: &str| flac_with(&[(6, title.as_bytes().to_vec())]);
    let cases = [
        ("wav", wav(&id3_a), wav(&id3_b)),
        ("aiff", aiff(&id3_a), aiff(&id3_b)),
        ("flac", flac("Night Drive"), flac("Night Dries")),
        ("mp4", mp4_titled("Night Drive"), mp4_titled("Night Dries")),
    ];
    for (name, before, after) in cases {
        assert_eq!(before.len(), after.len(), "{name}");
        assert_ne!(some(&before), some(&after), "{name}");
    }
}

#[test]
fn a_file_the_audio_plan_cant_read_has_no_partial_hash() {
    let junk = filler(50_000, 9);
    assert_eq!(partial(&[]), None, "an empty file");
    assert_eq!(partial(&junk), None, "an unknown format");
    assert_eq!(
        partial(&test_audio::ogg_vorbis()),
        None,
        "Ogg has no ranges"
    );
    let truncated = test_audio::m4a()[..40].to_vec();
    assert_eq!(partial(&truncated), None, "a truncated MP4");
}

#[test]
fn a_file_with_more_than_4_mib_of_metadata_has_no_partial_hash() {
    let audio = long_audio();
    let over = MAX_METADATA as usize + 1000;
    let under = MAX_METADATA as usize - 100_000;
    assert_eq!(
        partial(&mp3(&tag_with_artwork(over, 5, 'y'), &audio, &[])),
        None
    );
    assert!(partial(&mp3(&tag_with_artwork(under, 5, 'y'), &audio, &[])).is_some());
}

#[test]
fn reading_a_file_that_shrinks_is_an_error_not_a_shorter_hash() {
    let file = mp3(&tag_with_artwork(1000, 5, 'y'), &long_audio(), &[]);
    // Claims to be longer than it is.
    let result = partial_hash(&mut Cursor::new(&file), file.len() as u64 + 5000);
    assert!(
        !matches!(result, Ok(Some(_))),
        "a hash of a file that ended early: {result:?}"
    );
}

// ---- stored by the hash stage ------------------------------------------------

mod stored {
    use crate::db::Writer;
    use crate::hash::partial::PARTIAL_HASH_LEN;
    use crate::hash::state::{record, Record, Result};
    use crate::hash::Skip;

    fn writer_with_a_file() -> (tempfile::TempDir, Writer) {
        let dir = tempfile::tempdir().unwrap();
        let writer = Writer::open(&crate::write_guard::test_path(
            dir.path(),
            crate::db::DB_FILE_NAME,
        ))
        .unwrap();
        writer
            .call(|c| {
                c.execute_batch(
                    "INSERT INTO volume (identity, kind) VALUES ('serial=NTFS-1A2B3C4D', 'external');
                     INSERT INTO music_folder (volume_id, rel_path, rel_path_key)
                         VALUES (1, 'Music', 'Music');
                     INSERT INTO file (music_folder_id, rel_path, rel_path_key, size, mtime)
                         VALUES (1, 'a.mp3', 'a.mp3', 100, 5000);",
                )
            })
            .unwrap();
        (dir, writer)
    }

    fn hashed(mtime: i64, partial: Option<[u8; PARTIAL_HASH_LEN]>) -> Record {
        Record {
            id: 1,
            size: Some(100),
            mtime: Some(mtime),
            result: Result::Hashed {
                blake3: [7; 32],
                audio: Err(Skip::UnknownFormat),
                partial,
            },
        }
    }

    fn stored_partial(writer: &Writer) -> Option<Vec<u8>> {
        writer
            .call(|c| c.query_row("SELECT partial_hash FROM file", [], |r| r.get(0)))
            .unwrap()
    }

    #[test]
    fn the_hash_stage_stores_the_partial_hash_with_the_hashes() {
        let (_dir, writer) = writer_with_a_file();
        writer
            .call(|c| record(c, &[hashed(5000, Some([9; PARTIAL_HASH_LEN]))]))
            .unwrap();
        assert_eq!(stored_partial(&writer), Some(vec![9; PARTIAL_HASH_LEN]));
    }

    #[test]
    fn a_partial_hash_is_stored_only_if_the_row_still_has_the_stat_it_was_hashed_at() {
        let (_dir, writer) = writer_with_a_file();
        // A walk moved the mtime while the file was being hashed.
        writer
            .call(|c| record(c, &[hashed(4000, Some([9; PARTIAL_HASH_LEN]))]))
            .unwrap();
        assert_eq!(stored_partial(&writer), None);
    }

    #[test]
    fn hashing_a_file_that_has_no_partial_hash_clears_an_older_one() {
        let (_dir, writer) = writer_with_a_file();
        writer
            .call(|c| record(c, &[hashed(5000, Some([9; PARTIAL_HASH_LEN]))]))
            .unwrap();
        writer.call(|c| record(c, &[hashed(5000, None)])).unwrap();
        assert_eq!(stored_partial(&writer), None);
    }
}

/// Local only (`cargo test --lib -- --ignored partial_hash_cost --nocapture`):
/// what one partial hash costs next to hashing the whole file, on 100
/// synthetic 6 MB MP3s with 300 KB of artwork. Warm cache, so it's the CPU
/// and syscall cost; the bytes read are the point on a cold drive.
#[test]
#[ignore = "timing, local only"]
fn partial_hash_cost() {
    use std::time::Instant;
    let dir = tempfile::tempdir().unwrap();
    let audio = mp3_frames().repeat(600); // about 6 MB
    let paths: Vec<_> = (0..100)
        .map(|i| {
            let tag = tag_with_artwork(300_000, 1000 + i, 'y');
            let path = dir.path().join(format!("t{i}.mp3"));
            std::fs::write(&path, mp3(&tag, &audio, &id3v1_tag("x"))).unwrap();
            path
        })
        .collect();
    let size = std::fs::metadata(&paths[0]).unwrap().len();
    let started = Instant::now();
    for p in &paths {
        let mut f = std::fs::File::open(p).unwrap();
        let len = f.metadata().unwrap().len();
        assert!(partial_hash(&mut f, len).unwrap().is_some());
    }
    let partial = started.elapsed();
    let started = Instant::now();
    for p in &paths {
        let mut f = std::fs::File::open(p).unwrap();
        crate::hash::hash_reader(&mut f, &mut vec![0; crate::hash::BUFFER], &mut |_| false)
            .unwrap();
    }
    let whole = started.elapsed();
    println!(
        "file {size} bytes: partial hash {:?} per file (reads about {} KiB), whole-file hashes {:?} per file",
        partial / 100,
        (300_000 + 2 * EDGE as usize + 200) / 1024,
        whole / 100
    );
}
