//! The file's own artist and title, and when a hit from a text search counts
//! as the same track (ROADMAP 2.2; Run 6A, S2b).
//!
//! A search by name finds look-alikes, so a hit is taken only when it matches
//! closely ([`close`]): the titles are equal once case, punctuation and a
//! bracketed version are folded away, and the artists are equal after
//! folding, or one holds the other as whole words ("Daft Punk" in "Daft Punk
//! feat. Pharrell", not "Dan" in "Danny"). Anything looser is dropped.

use crate::send_values::tag_values;
use crate::versions::{parse_file_name, parse_title, CreditRole};

/// The artist and title a name search goes by.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Names {
    pub artist: String,
    pub title: String,
}

/// A file's artist and title: its tags' `Artist` and `Name`, and for what
/// the tags lack, the file's name read as "Artist - Title (Mix)". `None`
/// unless both are there.
pub fn names_of(raw_tags: Option<&str>, rel_path: &str) -> Option<Names> {
    let tags = raw_tags
        .map(|raw| tag_values(raw, &["Name", "Artist"]))
        .unwrap_or_default();
    let tag = |attribute: &str| {
        tags.iter()
            .find(|t| t.attribute == attribute)
            .map(|t| t.value.trim().to_owned())
            .filter(|v| !v.is_empty())
    };
    let (mut title, mut artist) = (tag("Name"), tag("Artist"));
    if title.is_none() || artist.is_none() {
        let name = rel_path.rsplit(['/', '\\']).next().unwrap_or(rel_path);
        let parsed = parse_file_name(name);
        if title.is_none() {
            title = Some(parsed.base_title.trim().to_owned()).filter(|t| !t.is_empty());
        }
        if artist.is_none() {
            let main: Vec<&str> = parsed
                .credits
                .iter()
                .filter(|c| c.role == CreditRole::Main)
                .map(|c| c.name.as_str())
                .collect();
            artist = Some(main.join(" & ")).filter(|a| !a.is_empty());
        }
    }
    Some(Names {
        artist: artist?,
        title: title?,
    })
}

/// The title without its bracketed or dashed version ("Velo (Extended
/// Mix)" gives "Velo"); the title itself when nothing else is left.
pub fn base_title(title: &str) -> String {
    let base = parse_title(title).base_title;
    if base.trim().is_empty() {
        title.trim().to_owned()
    } else {
        base.trim().to_owned()
    }
}

/// Lower case, bracketed parts dropped, anything that isn't a letter or a
/// digit read as a space, runs of spaces as one.
pub fn fold(text: &str) -> String {
    let mut out = String::new();
    let mut depth = 0u32;
    for c in text.chars() {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            _ if depth > 0 => {}
            c if c.is_alphanumeric() => out.extend(c.to_lowercase()),
            _ => out.push(' '),
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Whether `hit_title` and `hit_artist` are the track `want`, by the rule in
/// the module docs.
pub fn close(want: &Names, hit_title: &str, hit_artist: &str) -> bool {
    let title = fold(&base_title(&want.title));
    !title.is_empty()
        && title == fold(&base_title(hit_title))
        && same_artist(&fold(&want.artist), &fold(hit_artist))
}

/// Equal, or one's words are a run of whole words of the other's.
fn same_artist(a: &str, b: &str) -> bool {
    if a.is_empty() || b.is_empty() {
        return false;
    }
    let (a, b): (Vec<&str>, Vec<&str>) = (a.split(' ').collect(), b.split(' ').collect());
    let (short, long) = if a.len() <= b.len() { (a, b) } else { (b, a) };
    long.windows(short.len()).any(|w| w == short.as_slice())
}
