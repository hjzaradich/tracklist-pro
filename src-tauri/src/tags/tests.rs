//! Tag reading on real (generated) files of every format, broken tags, and
//! the read-only guarantee. Files are built at test time by `test_audio` and
//! tagged with lofty in temp dirs; that writing is test setup, not app code.

use std::path::{Path, PathBuf};

use lofty::config::WriteOptions;
use lofty::tag::items::Timestamp;
use lofty::tag::{ItemKey, Tag};
use tempfile::TempDir;

use super::test_audio::{self, ape_tag, id3_frame, id3_text, id3v1_tag, id3v2, Format, Rng};
use super::*;

// ---- setup ----------------------------------------------------------------

/// Writes tags into `path` with lofty, into the format's main tag.
fn tag_file(path: &Path, fill: impl FnOnce(&mut Tag)) {
    let mut tagged = lofty::read_from_path(path).unwrap();
    let tag_type = tagged.primary_tag_type();
    if tagged.primary_tag_mut().is_none() {
        tagged.insert_tag(Tag::new(tag_type));
    }
    fill(tagged.primary_tag_mut().unwrap());
    tagged.save_to_path(path, WriteOptions::default()).unwrap();
}

fn insert(tag: &mut Tag, key: ItemKey, value: &str) {
    let tag_type = tag.tag_type();
    assert!(
        tag.insert_text(key, value.to_string()),
        "lofty can't store {key:?} in {tag_type:?}"
    );
}

/// The full set of fields a DJ's files carry, as Mixed In Key and rekordbox
/// leave them.
fn fill_all_fields(tag: &mut Tag) {
    tag.set_title("Night Drive".into());
    tag.set_artist("Test Artist".into());
    tag.set_album("Test Album".into());
    tag.set_genre("Deep House".into());
    tag.set_comment("8A - Energy 7".into());
    tag.set_track(3);
    tag.set_disk(1);
    tag.set_date(Timestamp {
        year: 2019,
        month: Some(4),
        day: Some(12),
        ..Timestamp::default()
    });
    insert(tag, ItemKey::AlbumArtist, "Various Artists");
    insert(tag, ItemKey::Composer, "A. Composer");
    insert(tag, ItemKey::Remixer, "Some Remixer");
    insert(tag, ItemKey::Label, "Test Records");
    // Vorbis comments have one BPM field, which lofty calls `Bpm`.
    if !tag.insert_text(ItemKey::IntegerBpm, "124".into()) {
        insert(tag, ItemKey::Bpm, "124");
    }
    insert(tag, ItemKey::InitialKey, "Am");
}

fn tagged_file(dir: &TempDir, format: Format) -> PathBuf {
    let path = format.write_to(dir.path(), "tagged");
    tag_file(&path, fill_all_fields);
    path
}

/// An MP3 made of the given leading bytes (usually an ID3v2 tag), silent
/// audio frames, and the given trailing bytes.
fn mp3_with(dir: &TempDir, head: &[u8], tail: &[u8]) -> PathBuf {
    let path = dir.path().join("handmade.mp3");
    std::fs::write(&path, [head, &test_audio::mp3(), tail].concat()).unwrap();
    path
}

fn kinds(read: &TagRead) -> Vec<TagProblemKind> {
    read.problems.iter().map(|p| p.kind).collect()
}

/// The key each format's own tag uses for the title.
fn native_title_key(format: Format) -> (RawTagType, &'static str) {
    match format {
        Format::Mp3 | Format::Wav | Format::Aiff => (RawTagType::Id3v2, "TIT2"),
        Format::Flac | Format::Ogg | Format::Opus => (RawTagType::VorbisComments, "TITLE"),
        Format::M4a => (RawTagType::Mp4Ilst, "\u{a9}nam"),
    }
}

// ---- every format ---------------------------------------------------------

fn assert_reads_every_field(format: Format) {
    let dir = tempfile::tempdir().unwrap();
    let path = tagged_file(&dir, format);

    let read = read(&path).unwrap();

    assert_eq!(read.parsed_as, format.parsed_as());
    assert_eq!(read.problems, vec![], "{format:?}");
    assert_eq!(
        read.properties.sample_rate,
        Some(format.sample_rate() as i64),
        "{format:?}"
    );

    let f = &read.fields;
    assert_eq!(f.title.as_deref(), Some("Night Drive"));
    assert_eq!(f.artist.as_deref(), Some("Test Artist"));
    assert_eq!(f.album.as_deref(), Some("Test Album"));
    assert_eq!(f.album_artist.as_deref(), Some("Various Artists"));
    assert_eq!(f.genre.as_deref(), Some("Deep House"));
    assert_eq!(f.comment.as_deref(), Some("8A - Energy 7"));
    assert_eq!(f.composer.as_deref(), Some("A. Composer"));
    assert_eq!(f.remixer.as_deref(), Some("Some Remixer"));
    assert_eq!(f.label.as_deref(), Some("Test Records"));
    assert_eq!(f.year, Some(2019));
    assert_eq!(f.track_no, Some(3));
    assert_eq!(f.disc_no, Some(1));

    // BPM and key come back as the tag's reading, not as plain fields.
    assert_eq!(
        read.analysis_from(AnalysisSource::Tag),
        Some(&AnalysisReading {
            source: AnalysisSource::Tag,
            bpm: Some(124.0),
            key: Some("8A".into()),
            key_raw: Some("Am".into()),
            energy: None,
        })
    );
    // The energy is Mixed In Key's, from its comment.
    assert_eq!(
        read.analysis_from(AnalysisSource::Mik),
        Some(&AnalysisReading {
            source: AnalysisSource::Mik,
            bpm: None,
            key: None,
            key_raw: None,
            energy: Some(7),
        })
    );

    // The raw frames hold the title under the format's own key.
    let (tag_type, key) = native_title_key(format);
    let raw = read
        .raw_tag(tag_type)
        .unwrap_or_else(|| panic!("{format:?}: no {tag_type:?} in {:?}", read.raw_tags));
    assert_eq!(
        raw.get(key).and_then(RawValue::text),
        Some("Night Drive"),
        "{format:?}: {raw:?}"
    );
}

#[test]
fn mp3_tags_are_read() {
    assert_reads_every_field(Format::Mp3);
}

#[test]
fn flac_tags_are_read() {
    assert_reads_every_field(Format::Flac);
}

#[test]
fn wav_tags_are_read() {
    assert_reads_every_field(Format::Wav);
}

#[test]
fn aiff_tags_are_read() {
    assert_reads_every_field(Format::Aiff);
}

#[test]
fn m4a_tags_are_read() {
    assert_reads_every_field(Format::M4a);
}

#[test]
fn ogg_vorbis_tags_are_read() {
    assert_reads_every_field(Format::Ogg);
}

#[test]
fn opus_tags_are_read() {
    assert_reads_every_field(Format::Opus);
}

#[test]
fn untagged_files_of_every_format_read_with_empty_fields_and_no_problems() {
    let dir = tempfile::tempdir().unwrap();
    for format in Format::ALL {
        let path = format.write_to(dir.path(), "untagged");
        let read = read(&path).unwrap_or_else(|e| panic!("{format:?}: {e}"));
        assert_eq!(read.parsed_as, format.parsed_as());
        assert_eq!(read.fields, TagFields::default(), "{format:?}");
        assert_eq!(read.analysis, vec![], "{format:?}");
        assert_eq!(read.problems, vec![], "{format:?}");
        assert!(read.properties.sample_rate.is_some(), "{format:?}");
    }
}

#[test]
fn the_format_is_read_from_the_bytes_not_the_extension() {
    let dir = tempfile::tempdir().unwrap();
    let mp3 = tagged_file(&dir, Format::Mp3);
    let fake_wav = dir.path().join("really_an_mp3.wav");
    std::fs::rename(&mp3, &fake_wav).unwrap();

    let read = read(&fake_wav).unwrap();
    assert_eq!(read.parsed_as, "MP3");
    assert_eq!(read.fields.title.as_deref(), Some("Night Drive"));
}

#[test]
fn a_comment_without_an_energy_gives_no_mik_reading() {
    let dir = tempfile::tempdir().unwrap();
    let path = Format::Flac.write_to(dir.path(), "plain");
    tag_file(&path, |tag| tag.set_comment("great track".into()));

    let read = read(&path).unwrap();
    assert_eq!(read.fields.comment.as_deref(), Some("great track"));
    assert_eq!(read.analysis_from(AnalysisSource::Mik), None);
}

#[test]
fn key_notations_in_tags_are_normalized_to_camelot_and_the_raw_value_kept() {
    let dir = tempfile::tempdir().unwrap();
    for (raw, camelot) in [("Am", "8A"), ("08A", "8A"), ("1m", "8A"), ("F#m", "11A")] {
        let path = Format::Mp3.write_to(dir.path(), "key");
        tag_file(&path, |tag| insert(tag, ItemKey::InitialKey, raw));

        let read = read(&path).unwrap();
        let reading = read.analysis_from(AnalysisSource::Tag).unwrap();
        assert_eq!(reading.key.as_deref(), Some(camelot), "{raw}");
        assert_eq!(reading.key_raw.as_deref(), Some(raw));
    }
}

// ---- raw frames -----------------------------------------------------------

#[test]
fn raw_tags_keep_frames_the_fields_dont_cover() {
    let dir = tempfile::tempdir().unwrap();
    let mut txxx = vec![3u8];
    txxx.extend_from_slice(b"SERATO_PLAYCOUNT\x003");
    let mut geob = vec![0u8];
    geob.extend_from_slice(b"application/octet-stream\x00\x00Serato Markers2\x00");
    geob.extend_from_slice(&[0xAB; 40]);
    let mut priv_ = b"www.example.com\x00".to_vec();
    priv_.extend_from_slice(&[1, 2, 3, 4, 5]);
    let mut popm = b"rekordbox\x00".to_vec();
    popm.extend_from_slice(&[196, 0, 0, 0, 3]);
    let mut comm = vec![3u8];
    comm.extend_from_slice(b"eng\x00a comment");
    let tag = id3v2(
        &[
            id3_text(b"TIT2", "Title"),
            id3_frame(b"TXXX", &txxx),
            id3_frame(b"GEOB", &geob),
            id3_frame(b"PRIV", &priv_),
            id3_frame(b"POPM", &popm),
            id3_frame(b"COMM", &comm),
        ],
        0,
    );
    let path = mp3_with(&dir, &tag, &[]);

    let read = read(&path).unwrap();
    let raw = read.raw_tag(RawTagType::Id3v2).unwrap();

    let text = |key: &str| raw.get(key).and_then(RawValue::text);
    assert_eq!(text("TIT2"), Some("Title"));
    assert_eq!(text("TXXX:SERATO_PLAYCOUNT"), Some("3"));
    assert_eq!(text("COMM::eng"), Some("a comment"));
    assert_eq!(text("POPM:rekordbox"), Some("rating=196 counter=3"));
    assert_eq!(
        raw.get("PRIV:www.example.com"),
        Some(&RawValue::Binary { len: 5 })
    );
    // Serato's cue data: kept, recorded by size.
    assert_eq!(
        raw.get("GEOB"),
        Some(&RawValue::Binary { len: geob.len() }),
        "{raw:?}"
    );
}

#[test]
fn raw_tags_list_every_tag_block_in_the_file() {
    let dir = tempfile::tempdir().unwrap();
    // ID3v2 at the start, ID3v1 at the end.
    let mut id3v1 = b"TAG".to_vec();
    let field = |s: &str, n: usize| {
        let mut v = s.as_bytes().to_vec();
        v.resize(n, 0);
        v
    };
    id3v1.extend(field("Old Title", 30));
    id3v1.extend(field("Old Artist", 30));
    id3v1.extend(field("", 30));
    id3v1.extend(field("1999", 4));
    id3v1.extend(field("", 30));
    id3v1.push(255);
    let path = mp3_with(&dir, &id3v2(&[id3_text(b"TIT2", "New Title")], 0), &id3v1);

    let read = read(&path).unwrap();
    let types: Vec<_> = read.raw_tags.iter().map(|t| t.tag_type).collect();
    assert_eq!(types, vec![RawTagType::Id3v2, RawTagType::Id3v1]);
    let v1 = read.raw_tag(RawTagType::Id3v1).unwrap();
    assert_eq!(v1.get("title").and_then(RawValue::text), Some("Old Title"));
    // The fields come from the main tag (ID3v2), not the old ID3v1 one.
    assert_eq!(read.fields.title.as_deref(), Some("New Title"));
}

#[test]
fn raw_tags_serialize_to_json_for_storage() {
    let dir = tempfile::tempdir().unwrap();
    let path = tagged_file(&dir, Format::Flac);
    let read = read(&path).unwrap();

    let json = serde_json::to_value(&read.raw_tags).unwrap();
    let first = &json[0];
    assert_eq!(first["tag_type"], "vorbis_comments");
    let title = first["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["key"] == "TITLE")
        .unwrap();
    assert_eq!(
        title["value"],
        serde_json::json!({"type": "text", "text": "Night Drive"})
    );
}

// ---- broken tags (§5.5) ---------------------------------------------------

/// An APEv2 footer whose sizes point outside the file.
fn broken_ape_footer() -> Vec<u8> {
    let mut ape = b"APETAGEX".to_vec();
    ape.extend_from_slice(&2000u32.to_le_bytes()); // version
    ape.extend_from_slice(&0x7FFF_FFF0u32.to_le_bytes()); // tag size: huge
    ape.extend_from_slice(&0xFFFFu32.to_le_bytes()); // item count
    ape.extend_from_slice(&0u32.to_le_bytes()); // flags: footer
    ape.extend_from_slice(&[0; 8]);
    ape
}

#[test]
fn mp3_with_a_broken_ape_tag_still_reads_its_id3v2_tags_and_audio() {
    let dir = tempfile::tempdir().unwrap();
    let head = id3v2(
        &[
            id3_text(b"TIT2", "Survivor"),
            id3_text(b"TPE1", "Still Here"),
        ],
        0,
    );
    let path = mp3_with(&dir, &head, &broken_ape_footer());

    let read = read(&path).unwrap();
    assert_eq!(read.fields.title.as_deref(), Some("Survivor"));
    assert_eq!(read.fields.artist.as_deref(), Some("Still Here"));
    assert_eq!(read.properties.sample_rate, Some(44_100));
    assert_eq!(
        kinds(&read),
        vec![TagProblemKind::TagBlockSkipped],
        "{read:?}"
    );
}

#[test]
fn mp3_with_garbage_in_one_id3v2_frame_keeps_the_other_frames() {
    let dir = tempfile::tempdir().unwrap();
    // TPE1 claims text encoding 9, which doesn't exist.
    let head = id3v2(
        &[
            id3_text(b"TIT2", "Before"),
            id3_frame(b"TPE1", &[9, 0xFF, 0xFE, 0x00]),
            id3_text(b"TALB", "After"),
        ],
        0,
    );
    let path = mp3_with(&dir, &head, &[]);

    let read = read(&path).unwrap();
    assert_eq!(read.fields.title.as_deref(), Some("Before"));
    assert_eq!(read.fields.album.as_deref(), Some("After"));
    assert_eq!(read.fields.artist, None);
    assert_eq!(read.properties.sample_rate, Some(44_100));
    assert_eq!(
        kinds(&read),
        vec![TagProblemKind::FramesDropped],
        "{read:?}"
    );
    assert!(read.problems[0].detail.contains("TPE1"), "{read:?}");
    // The raw frames list what survived.
    let raw = read.raw_tag(RawTagType::Id3v2).unwrap();
    let keys: Vec<_> = raw.items.iter().map(|i| i.key.as_str()).collect();
    assert_eq!(keys, vec!["TIT2", "TALB"]);
}

#[test]
fn mp3_with_invalid_utf16_in_one_frame_keeps_the_other_frames() {
    let dir = tempfile::tempdir().unwrap();
    // UTF-16 with a BOM and an unpaired surrogate.
    let head = id3v2(
        &[
            id3_text(b"TIT2", "Before"),
            id3_frame(b"TPE1", &[1, 0xFF, 0xFE, 0x00, 0xD8, 0x41, 0x00]),
            id3_text(b"TALB", "After"),
        ],
        0,
    );
    let path = mp3_with(&dir, &head, &[]);

    let read = read(&path).unwrap();
    assert_eq!(read.fields.title.as_deref(), Some("Before"));
    assert_eq!(read.fields.album.as_deref(), Some("After"));
    assert_eq!(
        kinds(&read),
        vec![TagProblemKind::FramesDropped],
        "{read:?}"
    );
}

#[test]
fn mp3_with_a_bad_id3v2_frame_and_a_broken_ape_tag_keeps_the_good_frames() {
    let dir = tempfile::tempdir().unwrap();
    let head = id3v2(
        &[
            id3_text(b"TIT2", "Both Broken"),
            id3_frame(b"TPE1", &[9, b'x']),
        ],
        0,
    );
    let path = mp3_with(&dir, &head, &broken_ape_footer());

    let read = read(&path).unwrap();
    assert_eq!(read.fields.title.as_deref(), Some("Both Broken"));
    assert_eq!(read.properties.sample_rate, Some(44_100));
    assert_eq!(
        kinds(&read),
        vec![
            TagProblemKind::FramesDropped,
            TagProblemKind::TagBlockSkipped
        ],
        "{read:?}"
    );
}

#[test]
fn wav_with_garbage_in_one_id3_frame_keeps_the_other_frames() {
    let dir = tempfile::tempdir().unwrap();
    let tag = id3v2(
        &[
            id3_text(b"TIT2", "Before"),
            id3_frame(b"TPE1", &[9, b'x']),
            id3_text(b"TALB", "After"),
        ],
        0,
    );
    // rekordbox's WAV tags live in an `ID3 ` chunk.
    let mut wav = test_audio::wav();
    wav.extend_from_slice(b"ID3 ");
    wav.extend_from_slice(&(tag.len() as u32).to_le_bytes());
    wav.extend_from_slice(&tag);
    if tag.len() % 2 == 1 {
        wav.push(0);
    }
    let riff_size = (wav.len() - 8) as u32;
    wav[4..8].copy_from_slice(&riff_size.to_le_bytes());
    let path = dir.path().join("broken.wav");
    std::fs::write(&path, wav).unwrap();

    let read = read(&path).unwrap();
    assert_eq!(read.parsed_as, "WAV");
    assert_eq!(read.fields.title.as_deref(), Some("Before"));
    assert_eq!(read.fields.album.as_deref(), Some("After"));
    assert_eq!(read.properties.sample_rate, Some(8000));
    assert_eq!(
        kinds(&read),
        vec![TagProblemKind::FramesDropped],
        "{read:?}"
    );
}

#[test]
fn mp3_with_a_broken_ape_tag_keeps_the_id3v1_tag_after_it() {
    // mp3gain's APE tag followed by Winamp's ID3v1: a common layout.
    let dir = tempfile::tempdir().unwrap();
    let tail = [broken_ape_footer(), id3v1_tag("From ID3v1")].concat();
    let path = mp3_with(&dir, &[], &tail);

    let read = read(&path).unwrap();
    assert_eq!(read.fields.title.as_deref(), Some("From ID3v1"));
    let v1 = read.raw_tag(RawTagType::Id3v1).unwrap();
    assert_eq!(v1.get("title").and_then(RawValue::text), Some("From ID3v1"));
    assert_eq!(read.properties.sample_rate, Some(44_100));
    assert_eq!(
        kinds(&read),
        vec![TagProblemKind::TagBlockSkipped],
        "{read:?}"
    );
}

#[test]
fn a_valid_ape_tag_survives_a_broken_id3v2_frame() {
    let dir = tempfile::tempdir().unwrap();
    let head = id3v2(
        &[id3_frame(b"TPE1", &[9, b'x']), id3_text(b"TCON", "House")],
        0,
    );
    let tail = [ape_tag(&[("Album", "From APE")]), id3v1_tag("From ID3v1")].concat();
    let path = mp3_with(&dir, &head, &tail);

    let read = read(&path).unwrap();
    // Each field from the best tag that has it: ID3v2, then APE, then ID3v1.
    assert_eq!(read.fields.genre.as_deref(), Some("House"));
    assert_eq!(read.fields.album.as_deref(), Some("From APE"));
    assert_eq!(read.fields.title.as_deref(), Some("From ID3v1"));
    let types: Vec<_> = read.raw_tags.iter().map(|t| t.tag_type).collect();
    assert_eq!(
        types,
        vec![RawTagType::Id3v2, RawTagType::Ape, RawTagType::Id3v1]
    );
    // Only the bad frame is reported; the APE tag was fine.
    assert_eq!(
        kinds(&read),
        vec![TagProblemKind::FramesDropped],
        "{read:?}"
    );
}

#[test]
fn aiff_with_garbage_in_one_id3_frame_keeps_the_other_frames() {
    let dir = tempfile::tempdir().unwrap();
    let tag = id3v2(
        &[
            id3_text(b"TIT2", "Before"),
            id3_frame(b"TPE1", &[9, b'x']),
            id3_text(b"TALB", "After"),
        ],
        0,
    );
    let mut aiff = test_audio::aiff();
    aiff.extend_from_slice(b"ID3 ");
    aiff.extend_from_slice(&(tag.len() as u32).to_be_bytes());
    aiff.extend_from_slice(&tag);
    if tag.len() % 2 == 1 {
        aiff.push(0);
    }
    let form_size = (aiff.len() - 8) as u32;
    aiff[4..8].copy_from_slice(&form_size.to_be_bytes());
    let path = dir.path().join("broken.aiff");
    std::fs::write(&path, aiff).unwrap();

    let read = read(&path).unwrap();
    assert_eq!(read.parsed_as, "AIFF");
    assert_eq!(read.fields.title.as_deref(), Some("Before"));
    assert_eq!(read.fields.album.as_deref(), Some("After"));
    assert_eq!(read.properties.sample_rate, Some(8000));
    assert_eq!(
        kinds(&read),
        vec![TagProblemKind::FramesDropped],
        "{read:?}"
    );
}

#[test]
fn wav_salvage_finds_the_id3_chunk_after_an_odd_sized_chunk() {
    // RIFF pads odd-sized chunks to an even length; missing the pad byte
    // would miss the ID3 chunk.
    let dir = tempfile::tempdir().unwrap();
    let tag = id3v2(
        &[id3_text(b"TIT2", "Found"), id3_frame(b"TPE1", &[9, b'x'])],
        0,
    );
    let mut wav = test_audio::wav();
    wav.extend_from_slice(b"junk");
    wav.extend_from_slice(&3u32.to_le_bytes());
    wav.extend_from_slice(b"odd\0"); // 3 bytes + the pad byte
    wav.extend_from_slice(b"ID3 ");
    wav.extend_from_slice(&(tag.len() as u32).to_le_bytes());
    wav.extend_from_slice(&tag);
    let riff_size = (wav.len() - 8) as u32;
    wav[4..8].copy_from_slice(&riff_size.to_le_bytes());
    let path = dir.path().join("odd.wav");
    std::fs::write(&path, wav).unwrap();

    let read = read(&path).unwrap();
    assert_eq!(read.fields.title.as_deref(), Some("Found"));
    assert_eq!(
        kinds(&read),
        vec![TagProblemKind::FramesDropped],
        "{read:?}"
    );
}

#[test]
fn a_v24_tag_with_plain_frame_sizes_is_salvaged_without_cutting_frames() {
    // Old iTunes wrote v2.4 tags with plain (not synchsafe) frame sizes. A
    // 300-byte title's size, 00 00 01 2C, reads as 172 if taken as synchsafe.
    let dir = tempfile::tempdir().unwrap();
    let title = "t".repeat(299);
    let mut tit2 = b"TIT2".to_vec();
    tit2.extend_from_slice(&300u32.to_be_bytes());
    tit2.extend_from_slice(&[0, 0, 3]);
    tit2.extend_from_slice(title.as_bytes());
    let head = id3v2(
        &[
            id3_frame(b"TPE1", &[9, b'x']),
            tit2,
            id3_text(b"TALB", "Album"),
        ],
        0,
    );
    let path = mp3_with(&dir, &head, &[]);

    let read = read(&path).unwrap();
    assert_eq!(read.fields.title.as_deref(), Some(title.as_str()));
    assert_eq!(read.fields.album.as_deref(), Some("Album"));
    assert_eq!(
        kinds(&read),
        vec![TagProblemKind::FramesDropped],
        "{read:?}"
    );
    assert!(read.problems[0].detail.contains("TPE1"), "{read:?}");
}

#[test]
fn a_v24_tag_with_an_extended_header_is_salvaged() {
    let dir = tempfile::tempdir().unwrap();
    let ext = vec![0, 0, 0, 6, 1, 0];
    let frames = [
        ext,
        id3_text(b"TIT2", "Extended"),
        id3_frame(b"TPE1", &[9, b'x']),
    ]
    .concat();
    let mut head = id3v2(&[frames], 0);
    head[5] = 0x40; // extended header flag
    let path = mp3_with(&dir, &head, &[]);

    let read = read(&path).unwrap();
    assert_eq!(read.fields.title.as_deref(), Some("Extended"));
    assert_eq!(
        kinds(&read),
        vec![TagProblemKind::FramesDropped],
        "{read:?}"
    );
}

#[test]
fn garbage_in_an_id3v2_tag_is_reported_with_its_offset() {
    let dir = tempfile::tempdir().unwrap();
    let head = id3v2(
        &[
            id3_text(b"TIT2", "Kept"),
            id3_frame(b"TPE1", &[9, b'x']),
            vec![
                0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B,
            ],
        ],
        0,
    );
    let path = mp3_with(&dir, &head, &[]);

    let read = read(&path).unwrap();
    assert_eq!(read.fields.title.as_deref(), Some("Kept"));
    let detail = &read.problems[0].detail;
    assert!(detail.contains("TPE1"), "{detail}");
    assert!(detail.contains("unreadable data at tag offset"), "{detail}");
    assert!(detail.contains("(11 bytes)"), "{detail}");
}

#[test]
fn long_raw_values_are_cut_and_reported_with_their_full_length() {
    let dir = tempfile::tempdir().unwrap();
    let mut txxx = vec![3u8];
    txxx.extend_from_slice(b"NOTES\0");
    txxx.extend_from_slice("n".repeat(5000).as_bytes());
    let head = id3v2(&[id3_text(b"TIT2", "Wordy"), id3_frame(b"TXXX", &txxx)], 0);
    let path = mp3_with(&dir, &head, &[]);

    let read = read(&path).unwrap();
    let raw = read.raw_tag(RawTagType::Id3v2).unwrap();
    match raw.get("TXXX:NOTES").unwrap() {
        RawValue::Text { text, full_len } => {
            assert_eq!(text.len(), raw::MAX_TEXT_BYTES);
            assert_eq!(*full_len, Some(5000));
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(
        read.problems,
        vec![TagProblem {
            kind: TagProblemKind::ValueTruncated,
            detail: "TXXX:NOTES: 5000 bytes".into(),
        }]
    );
}

#[test]
fn mp3_with_a_bad_tdrc_still_reads_the_other_fields() {
    let dir = tempfile::tempdir().unwrap();
    let head = id3v2(
        &[
            id3_text(b"TIT2", "Dated Badly"),
            id3_text(b"TDRC", "sometime in the 90s"),
            id3_text(b"TPE1", "Artist"),
        ],
        0,
    );
    let path = mp3_with(&dir, &head, &[]);

    let read = read(&path).unwrap();
    assert_eq!(read.fields.title.as_deref(), Some("Dated Badly"));
    assert_eq!(read.fields.artist.as_deref(), Some("Artist"));
    assert_eq!(read.fields.year, None);
    // lofty's relaxed mode drops the bad frame and names it.
    assert_eq!(
        kinds(&read),
        vec![TagProblemKind::FieldsSkipped],
        "{read:?}"
    );
    assert!(read.problems[0].detail.contains("TDRC"), "{read:?}");
}

#[test]
fn mp3_with_an_id3v2_frame_overrunning_the_tag_keeps_the_frames_before_it() {
    // lofty reads what fits and doesn't complain, so no problem is listed.
    let dir = tempfile::tempdir().unwrap();
    // A frame header whose size runs far past the end of the tag.
    let mut overrun = b"TPE1".to_vec();
    overrun.extend_from_slice(&[0x7F, 0x7F, 0x7F, 0x7F, 0, 0]);
    overrun.extend_from_slice(&[3, b'x']);
    let head = id3v2(&[id3_text(b"TIT2", "Kept"), overrun], 16);
    let path = mp3_with(&dir, &head, &[]);

    let read = read(&path).unwrap();
    assert_eq!(read.fields.title.as_deref(), Some("Kept"));
    assert_eq!(read.properties.sample_rate, Some(44_100));
}

#[test]
fn flac_with_a_corrupt_comment_block_still_reads_the_audio() {
    let dir = tempfile::tempdir().unwrap();
    let flac = test_audio::flac();
    // Insert a VORBIS_COMMENT block that claims 1000 comments but holds none.
    let mut comment = Vec::new();
    comment.extend_from_slice(&4u32.to_le_bytes());
    comment.extend_from_slice(b"test");
    comment.extend_from_slice(&1000u32.to_le_bytes());
    comment.extend_from_slice(&50u32.to_le_bytes()); // first comment: 50 bytes
    comment.extend_from_slice(b"TITLE=cut short");
    let mut bytes = flac[..4].to_vec(); // "fLaC"
    let streaminfo = &flac[4..4 + 4 + 34];
    bytes.push(streaminfo[0] & 0x7F); // STREAMINFO, no longer last
    bytes.extend_from_slice(&streaminfo[1..]);
    bytes.push(0x80 | 4); // last block, VORBIS_COMMENT
    bytes.extend_from_slice(&(comment.len() as u32).to_be_bytes()[1..]);
    bytes.extend_from_slice(&comment);
    bytes.extend_from_slice(&flac[4 + 4 + 34..]);
    let path = dir.path().join("corrupt.flac");
    std::fs::write(&path, bytes).unwrap();

    let read = read(&path).unwrap();
    assert_eq!(read.parsed_as, "FLAC");
    assert_eq!(read.properties.sample_rate, Some(8000));
    assert!(!read.problems.is_empty(), "{read:?}");
}

#[test]
fn junk_bpm_and_key_values_are_listed_as_problems_and_the_rest_is_read() {
    let dir = tempfile::tempdir().unwrap();
    let head = id3v2(
        &[
            id3_text(b"TIT2", "Odd Values"),
            id3_text(b"TBPM", "fast"),
            id3_text(b"TKEY", "unknown"),
        ],
        0,
    );
    let path = mp3_with(&dir, &head, &[]);

    let read = read(&path).unwrap();
    assert_eq!(read.fields.title.as_deref(), Some("Odd Values"));
    assert_eq!(
        read.problems,
        vec![
            TagProblem {
                kind: TagProblemKind::BadValue(Field::Bpm),
                detail: "fast".into()
            },
            TagProblem {
                kind: TagProblemKind::BadValue(Field::Key),
                detail: "unknown".into()
            },
        ]
    );
    // The key is kept as written, with no Camelot value.
    let reading = read.analysis_from(AnalysisSource::Tag).unwrap();
    assert_eq!(reading.bpm, None);
    assert_eq!(reading.key, None);
    assert_eq!(reading.key_raw.as_deref(), Some("unknown"));
}

#[test]
fn a_file_that_is_not_audio_is_an_error_not_a_panic() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("notes.mp3");
    std::fs::write(&path, b"these are not the droids you are looking for").unwrap();
    assert!(read(&path).is_err());
}

#[test]
fn a_missing_file_is_an_open_error() {
    let dir = tempfile::tempdir().unwrap();
    let err = read(&dir.path().join("gone.mp3")).unwrap_err();
    assert!(matches!(err, TagReadError::Open(_)), "{err:?}");
}

#[test]
fn truncated_files_of_every_format_never_panic() {
    let dir = tempfile::tempdir().unwrap();
    for format in Format::ALL {
        let path = tagged_file(&dir, format);
        let bytes = std::fs::read(&path).unwrap();
        let cut_path = dir.path().join(format!("cut.{}", format.extension()));
        for cut in (0..bytes.len()).step_by(37) {
            std::fs::write(&cut_path, &bytes[..cut]).unwrap();
            // Ok or Err are both fine; a panic fails the test.
            let _ = read(&cut_path);
        }
    }
}

#[test]
fn randomly_corrupted_files_of_every_format_never_panic() {
    let dir = tempfile::tempdir().unwrap();
    // Fixed seed: a failure reproduces.
    let mut rng = Rng(0x7261_636b_6c69_7374);
    for format in Format::ALL {
        let path = tagged_file(&dir, format);
        let bytes = std::fs::read(&path).unwrap();
        let bad_path = dir.path().join(format!("bad.{}", format.extension()));
        for _ in 0..300 {
            let mut bad = bytes.clone();
            // A few bytes, mostly in the headers and tags at either end,
            // where the parsers make their decisions.
            for _ in 0..1 + rng.below(4) {
                let at = match rng.below(3) {
                    0 => rng.below(bad.len().min(256)),
                    1 => bad.len() - 1 - rng.below(bad.len().min(256)),
                    _ => rng.below(bad.len()),
                };
                bad[at] = match rng.below(4) {
                    0 => 0x00,
                    1 => 0xFF,
                    _ => rng.next() as u8,
                };
            }
            std::fs::write(&bad_path, &bad).unwrap();
            if let Ok(read) = read(&bad_path) {
                if let Some(ms) = read.properties.duration_ms {
                    assert!((0..=MAX_DURATION_MS as i64).contains(&ms), "{format:?}");
                }
            }
        }
    }
}

#[test]
fn an_opus_header_with_zero_channels_is_an_error_not_a_panic() {
    // lofty 0.25.4 panics on this (`expect` in its Opus properties code).
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("zero_channels.opus");
    std::fs::write(&path, test_audio::ogg_opus_with(0, None)).unwrap();

    let err = read(&path).unwrap_err();
    assert!(matches!(err, TagReadError::Unreadable(_)), "{err:?}");
}

#[test]
fn an_opus_file_with_a_huge_granule_position_never_gives_a_huge_duration() {
    // lofty overflows on this: a panic in debug builds, a garbage duration in
    // release builds. Either way the read must end sanely.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("huge_granule.opus");
    std::fs::write(&path, test_audio::ogg_opus_with(1, Some(u64::MAX - 7))).unwrap();

    match read(&path) {
        Ok(read) => assert!(
            read.properties
                .duration_ms
                .is_none_or(|ms| ms <= MAX_DURATION_MS as i64),
            "{read:?}"
        ),
        Err(err) => assert!(matches!(err, TagReadError::Unreadable(_)), "{err:?}"),
    }
}

#[test]
fn an_implausible_duration_is_dropped_and_reported() {
    let properties = lofty::properties::FileProperties::new(
        std::time::Duration::from_secs(10 * 24 * 60 * 60),
        None,
        None,
        Some(48_000),
        None,
        Some(2),
        None,
    );
    let tagged = TaggedFile::new(FileType::Opus, properties, vec![]);

    let read = from_tagged_file(&tagged);
    assert_eq!(read.properties.duration_ms, None);
    assert_eq!(read.properties.sample_rate, Some(48_000));
    assert_eq!(kinds(&read), vec![TagProblemKind::ImplausibleDuration]);
}

#[test]
fn release_builds_unwind_so_a_tag_library_panic_is_caught() {
    // `catch_unwind` can't catch anything under `panic = "abort"`.
    let manifest = include_str!("../../Cargo.toml");
    let strategy = manifest.contains("panic =") || manifest.contains("panic=");
    assert!(!strategy, "Cargo.toml sets a panic strategy");
}

// ---- read-only (CLAUDE.md: never write outside the Library) ---------------

#[test]
fn reading_leaves_every_formats_bytes_and_mtime_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    for format in Format::ALL {
        let path = tagged_file(&dir, format);
        // Put the mtime well in the past, so any write would move it.
        let old = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000_000);
        std::fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(old)
            .unwrap();
        let bytes_before = std::fs::read(&path).unwrap();

        read(&path).unwrap();

        assert_eq!(std::fs::read(&path).unwrap(), bytes_before, "{format:?}");
        let meta = std::fs::metadata(&path).unwrap();
        assert_eq!(meta.modified().unwrap(), old, "{format:?}");
    }
}

#[test]
fn reading_works_on_a_read_only_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = tagged_file(&dir, Format::Mp3);
    let mut perms = std::fs::metadata(&path).unwrap().permissions();
    perms.set_readonly(true);
    std::fs::set_permissions(&path, perms.clone()).unwrap();

    let read = read(&path);

    // Let the temp dir clean up.
    #[allow(clippy::permissions_set_readonly_false)]
    perms.set_readonly(false);
    std::fs::set_permissions(&path, perms).unwrap();
    assert_eq!(read.unwrap().fields.title.as_deref(), Some("Night Drive"));
}

/// If the reader asked for write access, Windows would refuse to open a file
/// another program has open with read-only sharing.
#[cfg(windows)]
#[test]
fn reading_works_while_another_program_allows_only_reading() {
    use std::os::windows::fs::OpenOptionsExt;
    const FILE_SHARE_READ: u32 = 1;

    let dir = tempfile::tempdir().unwrap();
    let path = tagged_file(&dir, Format::Flac);
    let _holder = std::fs::File::options()
        .read(true)
        .share_mode(FILE_SHARE_READ)
        .open(&path)
        .unwrap();

    let read = read(&path).unwrap();
    assert_eq!(read.fields.title.as_deref(), Some("Night Drive"));
}

#[test]
fn the_tags_module_has_no_code_that_writes_files() {
    // Test files are exempt: they build and tag fixtures in temp dirs.
    let sources = [
        ("mod.rs", include_str!("mod.rs")),
        ("key.rs", include_str!("key.rs")),
        ("raw.rs", include_str!("raw.rs")),
        ("salvage.rs", include_str!("salvage.rs")),
    ];
    let forbidden = [
        ".write(true)",
        ".append(true)",
        ".create(true)",
        ".truncate(true)",
        "File::create",
        "fs::write",
        "fs::rename",
        "fs::remove_file",
        "fs::copy",
        "set_permissions",
        "set_modified",
        "save_to",
        "write_to",
        "remove_from",
        "OpenOptions",
        "File::options",
        "set_times",
        "write_all",
        "set_len",
    ];
    for (name, source) in sources {
        for pattern in forbidden {
            assert!(
                !source.contains(pattern),
                "tags/{name} contains `{pattern}`"
            );
        }
    }
}
