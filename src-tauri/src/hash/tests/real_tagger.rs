//! 1aB-6 with a real tagger: lofty adds, grows and removes every tag kind
//! each format carries, the way taggers and rekordbox rewrite files, and
//! the audio_hash never moves. Writing with lofty is test setup, in temp
//! dirs.

use std::path::Path;

use lofty::config::WriteOptions;
use lofty::picture::{MimeType, Picture, PictureType};
use lofty::tag::{Accessor, Tag, TagExt, TagType};

use super::fixtures::hash;
use crate::tags::test_audio::Format;

/// The tag kinds lofty writes into each format.
fn tag_types(format: Format) -> &'static [TagType] {
    match format {
        Format::Mp3 => &[TagType::Id3v2, TagType::Id3v1, TagType::Ape],
        Format::Wav => &[TagType::Id3v2, TagType::RiffInfo],
        Format::Aiff => &[TagType::Id3v2, TagType::AiffText],
        Format::Flac | Format::Ogg | Format::Opus => &[TagType::VorbisComments],
        Format::M4a => &[TagType::Mp4Ilst],
    }
}

fn holds_pictures(tag_type: TagType) -> bool {
    matches!(
        tag_type,
        TagType::Id3v2 | TagType::Ape | TagType::VorbisComments | TagType::Mp4Ilst
    )
}

fn picture(size: usize) -> Picture {
    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    png.extend((0..size).map(|i| (i * 7) as u8));
    Picture::unchecked(png)
        .pic_type(PictureType::CoverFront)
        .mime_type(MimeType::Png)
        .build()
}

/// Writes a `tag_type` tag into `path`, with `comment` and, where the
/// kind holds one, a cover of `cover` bytes.
fn write_tag(path: &Path, tag_type: TagType, comment: &str, cover: usize) {
    let mut tag = Tag::new(tag_type);
    tag.set_title("Night Drive".into());
    tag.set_artist("Test Artist".into());
    tag.set_comment(comment.into());
    if holds_pictures(tag_type) && cover > 0 {
        tag.push_picture(picture(cover));
    }
    tag.save_to_path(path, WriteOptions::default())
        .unwrap_or_else(|e| panic!("lofty can't write {tag_type:?} to {path:?}: {e}"));
}

fn read(path: &Path) -> Vec<u8> {
    std::fs::read(path).unwrap()
}

#[test]
fn a_real_tagger_adding_growing_and_removing_every_tag_kind_keeps_the_audio_hash() {
    let dir = tempfile::tempdir().unwrap();
    for format in Format::ALL {
        let path = format.write_to(dir.path(), "track");
        let untagged = read(&path);
        let expected = hash(&untagged)
            .audio
            .unwrap_or_else(|e| panic!("{format:?}: {e:?}"));
        let mut seen = vec![untagged];

        for &tag_type in tag_types(format) {
            write_tag(&path, tag_type, "8A - Energy 7", 1000);
            seen.push(read(&path));
            // Grown: a long comment and a bigger cover, as rekordbox's
            // My Tag comment or a new artwork would.
            write_tag(&path, tag_type, &"My Tag / Peak ".repeat(400), 60_000);
            seen.push(read(&path));
            // Shrunk again.
            write_tag(&path, tag_type, "x", 0);
            seen.push(read(&path));
        }
        // Removed, kind by kind.
        for &tag_type in tag_types(format) {
            // lofty's `remove_from_path` opens the file read-only, so hand
            // it a writable one.
            let mut file = std::fs::File::options()
                .read(true)
                .write(true)
                .open(&path)
                .unwrap();
            tag_type
                .remove_from(&mut file, WriteOptions::default())
                .unwrap_or_else(|e| panic!("{format:?}: removing {tag_type:?}: {e}"));
            drop(file);
            seen.push(read(&path));
        }

        for (n, bytes) in seen.iter().enumerate() {
            let hashed = hash(bytes);
            assert_eq!(
                hashed.audio,
                Ok(expected),
                "{format:?}: step {n} changed the audio_hash"
            );
        }
        // The tagger really changed the files each time it added a tag.
        let tagged = &seen[1];
        assert_ne!(hash(tagged).blake3, hash(&seen[0]).blake3, "{format:?}");
    }
}
