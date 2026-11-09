//! The file's own artist and title, and when a hit from a text search counts
//! as the same track (ROADMAP 2.2; Run 6A, S2b).
//!
//! A search by name finds look-alikes, so a hit is taken only when it matches
//! closely ([`close`]): the titles are equal once case, punctuation and a
//! bracketed version are folded away; the artists share a name (each credit
//! is split on `&`, `,`, `feat.`, `ft.`, `featuring`, `x`, `and`, `vs`, and one
//! name must match exactly: "Kit feat. Y" has "Kit", "Kit Sebastian" does
//! not); and where both titles carry a bracketed version it is the same one.
//! Anything looser is dropped.

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
/// the module docs: the titles are equal once folded, the artists share a
/// name ([`artists_match`]), and where both titles carry a bracketed version
/// the versions are the same.
pub fn close(want: &Names, hit_title: &str, hit_artist: &str) -> bool {
    let title = fold(&base_title(&want.title));
    !title.is_empty()
        && title == fold(&base_title(hit_title))
        && artists_match(&want.artist, hit_artist)
        && versions_agree(&want.title, hit_title)
}

/// Whether `hit_artist` is the artist of `want`, by the artist half of
/// [`close`]: for a hit whose own title isn't the track's (a Discogs release
/// that contains it).
pub fn artist_matches(want: &Names, hit_artist: &str) -> bool {
    artists_match(&want.artist, hit_artist)
}

/// The words that join the names of a credit ("A feat. B", "A x B").
const JOINERS: [&str; 6] = ["feat", "ft", "featuring", "x", "and", "vs"];

/// The names in a credit, folded: split on `&`, `,`, `feat.`, `ft.`,
/// `featuring`, `x`, `and` and `vs`, and on `/`, `;` and `+` when whitespace
/// is on both sides ("A / B" is two names, "AC/DC" is one). A joiner at the
/// start or end of the credit, or beside another, is part of a name
/// ("Malcolm X"). For matching only: the tags keep their own rule
/// (`versions::split_names`).
pub fn credit_names(credit: &str) -> Vec<String> {
    let mut spaced = String::new();
    let mut depth = 0u32;
    let chars: Vec<char> = credit.chars().collect();
    for (at, &c) in chars.iter().enumerate() {
        let spaced_out = at > 0
            && chars[at - 1].is_whitespace()
            && chars.get(at + 1).is_some_and(|n| n.is_whitespace());
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            _ if depth > 0 => {}
            '&' | ',' => spaced.push_str(" | "),
            ';' | '/' | '+' if spaced_out => spaced.push_str(" | "),
            c if c.is_alphanumeric() => spaced.extend(c.to_lowercase()),
            _ => spaced.push(' '),
        }
    }
    let words: Vec<&str> = spaced.split_whitespace().collect();
    let is_joiner = |i: usize| {
        JOINERS.contains(&words[i])
            && i > 0
            && i + 1 < words.len()
            && words[i - 1] != "|"
            && words[i + 1] != "|"
            && !JOINERS.contains(&words[i + 1])
    };
    let mut names = Vec::new();
    let mut current: Vec<&str> = Vec::new();
    for (i, &word) in words.iter().enumerate() {
        if word == "|" || is_joiner(i) {
            if !current.is_empty() {
                names.push(current.join(" "));
                current.clear();
            }
        } else {
            current.push(word);
        }
    }
    if !current.is_empty() {
        names.push(current.join(" "));
    }
    names
}

/// Whether two credits share a name exactly (after folding): "Kit feat. Y"
/// has "Kit"; "Kit Sebastian" doesn't.
pub fn artists_match(a: &str, b: &str) -> bool {
    let (a, b) = (credit_names(a), credit_names(b));
    a.iter().any(|name| b.contains(name))
}

/// The version a title carries in brackets or after a dash ("Velo (Extended
/// Mix)": "extended mix"), folded; `None` when the parser sees none it is
/// sure of.
fn version_of(title: &str) -> Option<String> {
    let parsed = parse_title(title);
    let texts: Vec<String> = parsed
        .markers
        .iter()
        .filter(|m| !m.ambiguous)
        .map(|m| {
            m.text
                .trim()
                .trim_start_matches(['-', '(', '[', '{'])
                .trim_end_matches([')', ']', '}'])
                .chars()
                .map(|c| {
                    if c.is_alphanumeric() {
                        c.to_lowercase().collect::<String>()
                    } else {
                        " ".to_owned()
                    }
                })
                .collect::<String>()
        })
        .collect();
    let joined = texts
        .join(" ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    Some(joined).filter(|v| !v.is_empty())
}

/// Whether both titles carry a version and it is the same one.
pub fn same_version(want_title: &str, hit_title: Option<&str>) -> bool {
    match (version_of(want_title), hit_title.and_then(version_of)) {
        (Some(a), Some(b)) => a == b,
        _ => false,
    }
}

/// Two versions, where both titles carry one, must be the same; a title
/// without one agrees with any.
fn versions_agree(want_title: &str, hit_title: &str) -> bool {
    match (version_of(want_title), version_of(hit_title)) {
        (Some(a), Some(b)) => a == b,
        _ => true,
    }
}
