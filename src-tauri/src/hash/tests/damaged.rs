//! Damaged files: cut short, corrupted, or changing while read. Nothing
//! panics, blake3 is always exact, and the audio_hash is never a guess.

use std::io::{self, Cursor, Read, Seek, SeekFrom};

use super::fixtures::*;
use crate::hash::{hash_reader, Skip};
use crate::tags::test_audio::Rng;

/// Every fixture with tags around it, so cuts land in tags too.
fn tagged_files() -> Vec<(String, Vec<u8>)> {
    let mut files: Vec<(String, Vec<u8>)> = every_format()
        .into_iter()
        .map(|(n, b)| (n.to_owned(), b))
        .collect();
    let lead = &leading_tags()[2].1;
    let trail = &trailing_tags()[4].1;
    files.push((
        "tagged mp3".into(),
        [lead.clone(), mp3_frames(), trail.clone()].concat(),
    ));
    for (group, variants) in [
        ("wav", wav_variants()),
        ("aiff", aiff_variants()),
        ("flac", flac_variants()),
        ("mp4", mp4_variants()),
        ("ogg", ogg_vorbis_variants()),
    ] {
        for (name, bytes) in variants.into_iter().skip(1).take(2) {
            files.push((format!("{group}: {name}"), bytes));
        }
    }
    files
}

#[test]
fn every_truncation_of_every_format_hashes_without_panicking_and_blake3_stays_exact() {
    for (name, bytes) in tagged_files() {
        // Every length for small files; a stride for the big ones.
        let step = (bytes.len() / 3000).max(1);
        for len in (0..bytes.len()).step_by(step) {
            let cut = &bytes[..len];
            let hashed = hash(cut);
            assert_eq!(
                hashed.blake3,
                *blake3::hash(cut).as_bytes(),
                "{name} at {len}"
            );
        }
    }
}

#[test]
fn a_container_cut_inside_its_audio_is_truncated_never_hashed() {
    // Formats whose headers say how long the audio is.
    for (name, bytes) in every_format() {
        if matches!(name, "mp3" | "adts" | "flac") {
            continue;
        }
        // Not the last byte: for WAVE and AIFF that's the pad byte after
        // odd-length samples, and the audio is whole without it.
        for cut in [bytes.len() - 2, bytes.len() - 100, bytes.len() / 2 + 7] {
            let hashed = hash(&bytes[..cut]);
            assert!(
                matches!(hashed.audio, Err(Skip::Truncated | Skip::Malformed)),
                "{name} cut to {cut}: {:?}",
                hashed.audio
            );
        }
    }
}

#[test]
fn an_ogg_stream_without_its_end_of_stream_page_is_truncated() {
    let full = ogg_vorbis_with(vorbis_comment_packet(&[]));
    let last_page = full.windows(4).rposition(|w| w == b"OggS").unwrap();
    assert_eq!(hash(&full[..last_page]).audio, Err(Skip::Truncated));
}

#[test]
fn random_corruption_never_panics() {
    let mut rng = Rng(0x5eed_1ab6);
    for (name, bytes) in tagged_files() {
        for round in 0..150 {
            let mut bad = bytes.clone();
            for _ in 0..1 + rng.below(8) {
                match rng.below(3) {
                    0 => {
                        let at = rng.below(bad.len());
                        bad[at] = rng.next() as u8;
                    }
                    1 => {
                        let at = rng.below(bad.len());
                        bad.insert(at, rng.next() as u8);
                    }
                    _ => {
                        if bad.len() > 1 {
                            let at = rng.below(bad.len());
                            bad.remove(at);
                        }
                    }
                }
            }
            let hashed = hash(&bad);
            assert_eq!(
                hashed.blake3,
                *blake3::hash(&bad).as_bytes(),
                "{name} round {round}"
            );
        }
    }
}

#[test]
fn sizes_that_point_far_past_the_end_are_truncated_or_malformed_not_a_panic() {
    let mut huge_data = wav_plain();
    let data = huge_data.windows(4).position(|w| w == b"data").unwrap();
    huge_data[data + 4..data + 8].copy_from_slice(&0xFFFF_FFF0u32.to_le_bytes());
    let mut huge_box = crate::tags::test_audio::m4a();
    huge_box[..4].copy_from_slice(&1u32.to_be_bytes()); // 64-bit size next
    let mut huge_block = flac_with(&[(1, vec![0; 16])]);
    huge_block[42 + 1..42 + 4].copy_from_slice(&[0xFF, 0xFF, 0xFF]);
    for (name, bytes) in [
        ("wav data", huge_data),
        ("mp4 box", huge_box),
        ("flac block", huge_block),
    ] {
        let hashed = hash(&bytes);
        assert!(
            matches!(hashed.audio, Err(Skip::Truncated | Skip::Malformed)),
            "{name}: {:?}",
            hashed.audio
        );
    }
}

/// A file that says it's `len` bytes long but ends after `ends`, as a file
/// shrinking while it's read does.
struct Shrinking {
    inner: Cursor<Vec<u8>>,
    ends: u64,
}

impl Read for Shrinking {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let left = self.ends.saturating_sub(self.inner.position());
        let n = (buf.len() as u64).min(left) as usize;
        self.inner.read(&mut buf[..n])
    }
}

impl Seek for Shrinking {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        self.inner.seek(pos)
    }
}

#[test]
fn a_file_that_shrinks_while_it_is_read_gets_no_audio_hash() {
    for (name, bytes) in every_format() {
        let ends = bytes.len() as u64 - 3;
        let mut file = Shrinking {
            inner: Cursor::new(bytes),
            ends,
        };
        let hashed = hash_reader(&mut file, &mut [0; 512], &mut |_| false)
            .unwrap()
            .unwrap();
        assert_eq!(hashed.audio, Err(Skip::Truncated), "{name}");
        assert_eq!(hashed.len, ends, "{name}");
    }
}

#[test]
fn a_file_that_shrinks_while_its_tags_and_chunks_are_read_never_panics() {
    // Every place the plan reads a fixed-size field near the end or inside
    // a chunk: an APEv2 footer (32 bytes), a Lyrics3 tail (15), an ID3v2.4
    // footer (10), ID3v1 (3), SSND's offset fields (8), STREAMINFO, MP4
    // sample entries. The file claims its full length but ends at `ends`.
    let frames = mp3_frames();
    let mut files: Vec<(String, Vec<u8>)> = trailing_tags()
        .into_iter()
        .map(|(name, tag)| (format!("mp3 + {name}"), [frames.clone(), tag].concat()))
        .collect();
    files.push(("aiff".into(), aiff_plain()));
    files.push(("aifc".into(), aifc_none()));
    files.push(("flac".into(), flac_with(&[(4, vec![0; 40])])));
    files.push(("mp4".into(), crate::tags::test_audio::m4a()));
    files.push(("wav".into(), wav_plain()));
    for (name, bytes) in files {
        let len = bytes.len();
        // Every cut in the last 300 bytes and the first 600, a stride between.
        let cuts = (0..len.min(600))
            .chain((600..len.saturating_sub(300)).step_by(97))
            .chain(len.saturating_sub(300)..len);
        for ends in cuts {
            let mut file = Shrinking {
                inner: Cursor::new(bytes.clone()),
                ends: ends as u64,
            };
            let hashed = hash_reader(&mut file, &mut [0; 256], &mut |_| false)
                .unwrap()
                .unwrap();
            assert_eq!(
                hashed.audio,
                Err(Skip::Truncated),
                "{name} ending at {ends}"
            );
        }
    }
}
