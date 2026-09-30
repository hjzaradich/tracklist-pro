//! ID3v2.3, ID3v2.4 and ID3v1 tags.

use super::{Field, Tags};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Id3Version {
    V23,
    V24,
}

/// A 28-bit syncsafe integer (7 bits per byte).
pub fn syncsafe(v: u32) -> [u8; 4] {
    assert!(v < 1 << 28, "too big for syncsafe");
    [
        (v >> 21) as u8 & 0x7f,
        (v >> 14) as u8 & 0x7f,
        (v >> 7) as u8 & 0x7f,
        v as u8 & 0x7f,
    ]
}

fn frame_id(field: Field, version: Id3Version) -> &'static [u8; 4] {
    match field {
        Field::Title => b"TIT2",
        Field::Artist => b"TPE1",
        Field::Album => b"TALB",
        Field::Genre => b"TCON",
        Field::Year => match version {
            Id3Version::V23 => b"TYER",
            Id3Version::V24 => b"TDRC",
        },
        Field::Track => b"TRCK",
        Field::Bpm => b"TBPM",
        Field::Key => b"TKEY",
    }
}

/// A text frame's body: an encoding byte, then the text. v2.4 uses UTF-8;
/// v2.3 uses Latin-1 for ASCII and UTF-16 with a BOM otherwise, like most
/// taggers.
fn text_body(text: &str, version: Id3Version) -> Vec<u8> {
    let mut body = Vec::new();
    match version {
        Id3Version::V24 => {
            body.push(3);
            body.extend_from_slice(text.as_bytes());
        }
        Id3Version::V23 if text.is_ascii() => {
            body.push(0);
            body.extend_from_slice(text.as_bytes());
        }
        Id3Version::V23 => {
            body.push(1);
            body.extend_from_slice(&[0xff, 0xfe]);
            for unit in text.encode_utf16() {
                body.extend_from_slice(&unit.to_le_bytes());
            }
        }
    }
    body
}

/// A tag from raw text frames, in the order given. Empty if `frames` is.
pub fn tag_from_frames(frames: &[(&[u8; 4], &str)], version: Id3Version) -> Vec<u8> {
    if frames.is_empty() {
        return Vec::new();
    }
    let mut body = Vec::new();
    for (id, text) in frames {
        let data = text_body(text, version);
        body.extend_from_slice(*id);
        match version {
            Id3Version::V23 => body.extend_from_slice(&(data.len() as u32).to_be_bytes()),
            Id3Version::V24 => body.extend_from_slice(&syncsafe(data.len() as u32)),
        }
        body.extend_from_slice(&[0, 0]); // frame flags
        body.extend_from_slice(&data);
    }
    let mut out = Vec::with_capacity(body.len() + 10);
    out.extend_from_slice(b"ID3");
    out.push(match version {
        Id3Version::V23 => 3,
        Id3Version::V24 => 4,
    });
    out.extend_from_slice(&[0, 0]); // revision, flags
    out.extend_from_slice(&syncsafe(body.len() as u32));
    out.extend_from_slice(&body);
    out
}

/// An ID3v2 tag holding `tags`. Empty if there's nothing to write.
pub fn tag(tags: &Tags, version: Id3Version) -> Vec<u8> {
    let frames: Vec<(&[u8; 4], &str)> = tags
        .fields()
        .into_iter()
        .map(|(f, v)| (frame_id(f, version), v))
        .collect();
    tag_from_frames(&frames, version)
}

/// A 128-byte ID3v1 tag at the end of a file. Non-Latin-1 characters become
/// `?`, as old taggers do.
pub fn v1(tags: &Tags) -> [u8; 128] {
    fn put(out: &mut [u8], text: Option<&str>) {
        let bytes: Vec<u8> = text
            .unwrap_or("")
            .chars()
            .map(|c| if (c as u32) < 0x100 { c as u8 } else { b'?' })
            .collect();
        let n = bytes.len().min(out.len());
        out[..n].copy_from_slice(&bytes[..n]);
    }
    let mut out = [0u8; 128];
    out[..3].copy_from_slice(b"TAG");
    put(&mut out[3..33], tags.title.as_deref());
    put(&mut out[33..63], tags.artist.as_deref());
    put(&mut out[63..93], tags.album.as_deref());
    put(&mut out[93..97], tags.year.as_deref());
    out[127] = 255; // genre: none
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn syncsafe_uses_seven_bits_per_byte() {
        assert_eq!(syncsafe(0x7f), [0, 0, 0, 0x7f]);
        assert_eq!(syncsafe(0x80), [0, 0, 1, 0]);
    }

    #[test]
    fn a_v24_tag_has_the_header_and_frames_in_order() {
        let t = tag(&Tags::new("A", "B").year("2001"), Id3Version::V24);
        assert_eq!(&t[..4], b"ID3\x04");
        let s = String::from_utf8_lossy(&t);
        let (a, b, c) = (
            s.find("TIT2").unwrap(),
            s.find("TPE1").unwrap(),
            s.find("TDRC").unwrap(),
        );
        assert!(a < b && b < c);
    }

    #[test]
    fn no_tags_writes_no_tag() {
        assert!(tag(&Tags::default(), Id3Version::V23).is_empty());
    }
}
