//! APEv2 tags, including a deliberately broken one.

const PREAMBLE: &[u8; 8] = b"APETAGEX";
const VERSION: u32 = 2000;

fn item(key: &str, value: &str) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&(value.len() as u32).to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // UTF-8 text, read-write
    out.extend_from_slice(key.as_bytes());
    out.push(0);
    out.extend_from_slice(value.as_bytes());
    out
}

fn footer(tag_size: u32, item_count: u32) -> Vec<u8> {
    let mut out = PREAMBLE.to_vec();
    out.extend_from_slice(&VERSION.to_le_bytes());
    out.extend_from_slice(&tag_size.to_le_bytes());
    out.extend_from_slice(&item_count.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // flags: footer, no header
    out.extend_from_slice(&[0; 8]);
    out
}

/// The size a broken footer claims: far more than any fixture file.
pub const BROKEN_CLAIMED_SIZE: u32 = 0x00ff_fff0;
/// The item count a broken footer claims, though only one item is present.
pub const BROKEN_CLAIMED_ITEMS: u32 = 7;

/// A broken APEv2 tag, as left by a crashed tagger: one real item, then a
/// footer whose size and item count are both wrong. A strict reader seeks to
/// before the start of the file; a lenient one skips the tag.
pub fn broken(title: &str) -> Vec<u8> {
    let mut out = item("Title", title);
    out.extend_from_slice(&footer(BROKEN_CLAIMED_SIZE, BROKEN_CLAIMED_ITEMS));
    out
}

/// A parsed APEv2 footer at the very end of `bytes`: (claimed tag size,
/// claimed item count). For tests.
pub fn read_footer(bytes: &[u8]) -> Option<(u32, u32)> {
    let f = bytes.get(bytes.len().checked_sub(32)?..)?;
    if &f[..8] != PREAMBLE {
        return None;
    }
    let size = u32::from_le_bytes(f[12..16].try_into().ok()?);
    let items = u32::from_le_bytes(f[16..20].try_into().ok()?);
    Some((size, items))
}
