//! The file's text: escaping, and the elements in rekordbox's layout.
//!
//! Values are written so that any XML parser reads back exactly the text
//! given. Besides the five characters XML reserves, tab, line feed and
//! carriage return are written as character references: a parser turns a
//! literal one inside an attribute into a space.

use std::fmt::Write;

/// The first character of `value` that XML 1.0 can't carry at all, not
/// even as a character reference: the C0 controls other than tab, line
/// feed and carriage return, and U+FFFE / U+FFFF.
pub(super) fn uncarriable(value: &str) -> Option<char> {
    value.chars().find(|&c| {
        matches!(c, '\u{0}'..='\u{8}' | '\u{B}' | '\u{C}' | '\u{E}'..='\u{1F}' | '\u{FFFE}' | '\u{FFFF}')
    })
}

/// Whether `name` can be written as an attribute name. Every name
/// rekordbox writes is ASCII letters, digits and `_`; anything outside
/// XML's plain ASCII name characters is refused rather than guessed at.
pub(super) fn is_name(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
}

/// Appends `value` as the contents of a double-quoted attribute.
pub(super) fn escape_into(out: &mut String, value: &str) {
    for c in value.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            '\t' => out.push_str("&#9;"),
            '\n' => out.push_str("&#10;"),
            '\r' => out.push_str("&#13;"),
            c => out.push(c),
        }
    }
}

fn attr(out: &mut String, name: &str, value: &str) {
    out.push(' ');
    out.push_str(name);
    out.push_str("=\"");
    escape_into(out, value);
    out.push('"');
}

/// A COLLECTION track: its attributes in order. No children are ever
/// written (rule 3: the grid and cues are rekordbox's).
pub(super) struct TrackElement<'a> {
    pub attributes: &'a [(String, String)],
}

/// A PLAYLISTS node below ROOT.
pub(super) enum NodeElement {
    Folder {
        name: String,
        children: Vec<NodeElement>,
    },
    /// Entries are `TrackID`s (`KeyType="0"`, what rekordbox itself
    /// exports).
    Playlist { name: String, keys: Vec<u64> },
}

/// The whole file. `top` is ROOT's children.
pub(super) fn render(tracks: &[TrackElement<'_>], top: &[NodeElement]) -> String {
    let mut out = String::new();
    out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    out.push_str("<DJ_PLAYLISTS Version=\"1.0.0\">\n");
    out.push_str("  <PRODUCT");
    attr(&mut out, "Name", PRODUCT_NAME);
    attr(&mut out, "Version", env!("CARGO_PKG_VERSION"));
    attr(&mut out, "Company", PRODUCT_NAME);
    out.push_str("/>\n");
    let _ = writeln!(out, "  <COLLECTION Entries=\"{}\">", tracks.len());
    for track in tracks {
        out.push_str("    <TRACK");
        for (name, value) in track.attributes {
            attr(&mut out, name, value);
        }
        out.push_str("/>\n");
    }
    out.push_str("  </COLLECTION>\n");
    out.push_str("  <PLAYLISTS>\n");
    let _ = writeln!(
        out,
        "    <NODE Type=\"0\" Name=\"ROOT\" Count=\"{}\">",
        top.len()
    );
    for node in top {
        render_node(&mut out, node, 3);
    }
    out.push_str("    </NODE>\n");
    out.push_str("  </PLAYLISTS>\n");
    out.push_str("</DJ_PLAYLISTS>\n");
    out
}

/// The `PRODUCT` name: who wrote the file.
const PRODUCT_NAME: &str = "tracklist-pro";

fn render_node(out: &mut String, node: &NodeElement, depth: usize) {
    let pad = "  ".repeat(depth);
    out.push_str(&pad);
    match node {
        NodeElement::Folder { name, children } => {
            out.push_str("<NODE Type=\"0\"");
            attr(out, "Name", name);
            let _ = writeln!(out, " Count=\"{}\">", children.len());
            for child in children {
                render_node(out, child, depth + 1);
            }
            out.push_str(&pad);
            out.push_str("</NODE>\n");
        }
        NodeElement::Playlist { name, keys } => {
            out.push_str("<NODE");
            attr(out, "Name", name);
            let _ = write!(out, " Type=\"1\" KeyType=\"0\" Entries=\"{}\"", keys.len());
            if keys.is_empty() {
                out.push_str("/>\n");
                return;
            }
            out.push_str(">\n");
            for key in keys {
                let _ = writeln!(out, "{pad}  <TRACK Key=\"{key}\"/>");
            }
            out.push_str(&pad);
            out.push_str("</NODE>\n");
        }
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;
    use crate::rekordbox::attrs::unescape;

    fn escaped(value: &str) -> String {
        let mut out = String::new();
        escape_into(&mut out, value);
        out
    }

    #[test]
    fn the_five_reserved_characters_are_escaped() {
        assert_eq!(
            escaped(r#"Kit & Kin's <"x">"#),
            "Kit &amp; Kin&apos;s &lt;&quot;x&quot;&gt;"
        );
        // Text that looks like a reference is escaped, not passed through.
        assert_eq!(escaped("&amp; &#9;"), "&amp;amp; &amp;#9;");
    }

    #[test]
    fn tab_line_feed_and_carriage_return_become_character_references() {
        assert_eq!(escaped("a\tb\nc\rd\r\ne"), "a&#9;b&#10;c&#13;d&#13;&#10;e");
    }

    #[test]
    fn only_the_characters_xml_cannot_carry_are_reported() {
        assert_eq!(uncarriable("plain \t\r\n é 日本 🚀 \u{7f} \u{85}"), None);
        for c in [
            '\u{0}', '\u{1}', '\u{8}', '\u{b}', '\u{c}', '\u{e}', '\u{1f}', '\u{fffe}', '\u{ffff}',
        ] {
            assert_eq!(uncarriable(&format!("a{c}b")), Some(c), "{c:?}");
        }
    }

    #[test]
    fn attribute_names_are_plain_ascii_names() {
        for name in [
            "Name",
            "TrackID",
            "rb_local_deleted",
            "_x",
            "Future.Field-2",
        ] {
            assert!(is_name(name), "{name}");
        }
        for name in [
            "", "2x", "-x", "a b", "a=b", "a\"b", "a>b", "é", "a:b", "a/b",
        ] {
            assert!(!is_name(name), "{name:?}");
        }
    }

    proptest! {
        #[test]
        fn any_carriable_text_reads_back_exactly_through_the_readers_unescaping(
            value in proptest::collection::vec(
                prop_oneof![
                    3 => any::<char>(),
                    1 => proptest::sample::select(vec!['&', '<', '>', '"', '\'', '\t', '\n', '\r', ';', '#']),
                ],
                0..60,
            ),
        ) {
            let value: String = value
                .into_iter()
                .filter(|c| uncarriable(c.encode_utf8(&mut [0; 4])).is_none())
                .collect();
            let written = escaped(&value);
            // Nothing a parser would treat specially is left raw.
            prop_assert!(!written.contains(['<', '>', '"', '\'', '\t', '\n', '\r']));
            let (read, bad) = unescape(&written);
            prop_assert!(!bad);
            prop_assert_eq!(read, value);
        }
    }
}
