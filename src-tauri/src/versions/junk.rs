//! Store and rip junk (1bA-6): finds it, takes it out and reports it.
//!
//! Nothing here is dropped silently: every piece taken out goes into the
//! result's `junk` list as it was written, and the original string is kept
//! whole. The patterns are in [`super::tables`].

use super::tables::{
    AUDIO_EXTENSIONS, BITRATES, BITRATE_UNITS, FORMAT_WORDS, HTML_ENTITIES, RIP_TAGS, SITE_ENDINGS,
    SITE_FILLER_WORDS,
};
use super::tokenize::{collapse_spaces, is_dash, normalize};
use super::{Junk, JunkKind, NameSource};

fn report(junk: &mut Vec<Junk>, kind: JunkKind, text: &str) {
    junk.push(Junk {
        kind,
        text: text.to_string(),
    });
}

/// Takes the junk that sits around or inside the whole name off it, and
/// makes its characters plain. Brackets and " - " segments holding junk are
/// handled later, by the parser, with [`bracket_junk`] and
/// [`is_site_phrase`].
pub(super) fn clean(input: &str, source: NameSource, junk: &mut Vec<Junk>) -> String {
    let mut name = input.to_string();
    if source == NameSource::FileName {
        name = strip_folders(&name, junk);
        name = strip_extension(&name, junk);
    }
    name = decode_entities(&name, junk);
    if source == NameSource::FileName {
        name = decode_url_escapes(&name, junk);
    }
    name = normalize(&name);
    name = strip_store_id(&name, source, junk);
    name = replace_underscores(&name, source, junk);
    name = strip_track_number(&name, source, junk);
    collapse_spaces(&name)
}

fn strip_folders(name: &str, junk: &mut Vec<Junk>) -> String {
    match name.rfind(['/', '\\']) {
        Some(at) if at + 1 < name.len() => {
            report(junk, JunkKind::FolderPath, &name[..=at]);
            name[at + 1..].to_string()
        }
        _ => name.to_string(),
    }
}

fn strip_extension(name: &str, junk: &mut Vec<Junk>) -> String {
    if let Some(dot) = name.rfind('.') {
        let extension = name[dot + 1..].to_ascii_lowercase();
        if dot > 0 && AUDIO_EXTENSIONS.contains(&extension.as_str()) {
            report(junk, JunkKind::FileExtension, &name[dot..]);
            return name[..dot].to_string();
        }
    }
    name.to_string()
}

/// Decodes `&#40;`, `&#x28;` and the named entities in the table. Anything
/// else that starts with `&` is left as it is.
fn decode_entities(name: &str, junk: &mut Vec<Junk>) -> String {
    let mut out = String::with_capacity(name.len());
    let mut rest = name;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        let after = &rest[amp + 1..];
        let decoded = after
            .find(';')
            .filter(|&end| end <= 8)
            .and_then(|end| Some((end, entity_char(&after[..end])?)));
        match decoded {
            Some((end, c)) => {
                report(junk, JunkKind::HtmlEntity, &rest[amp..amp + end + 2]);
                out.push(c);
                rest = &after[end + 1..];
            }
            None => {
                out.push('&');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

fn entity_char(body: &str) -> Option<char> {
    let code = if let Some(hex) = body.strip_prefix("#x").or(body.strip_prefix("#X")) {
        u32::from_str_radix(hex, 16).ok()?
    } else if let Some(decimal) = body.strip_prefix('#') {
        decimal.parse::<u32>().ok()?
    } else {
        return HTML_ENTITIES
            .iter()
            .find(|(name, _)| *name == body)
            .map(|&(_, c)| c);
    };
    char::from_u32(code).filter(|c| !c.is_control())
}

fn decode_url_escapes(name: &str, junk: &mut Vec<Junk>) -> String {
    const ESCAPED_SPACE: &str = "%20";
    for _ in name.matches(ESCAPED_SPACE) {
        report(junk, JunkKind::UrlEscape, ESCAPED_SPACE);
    }
    name.replace(ESCAPED_SPACE, " ")
}

/// A store's numeric ID in front: six or more digits, then `_` or `-` (or,
/// in a file name, a space).
fn strip_store_id(name: &str, source: NameSource, junk: &mut Vec<Junk>) -> String {
    let digits = name.chars().take_while(char::is_ascii_digit).count();
    if digits < 6 {
        return name.to_string();
    }
    let rest = &name[digits..];
    let separated = match rest.chars().next() {
        Some('_') | Some('-') => true,
        Some(' ') => source == NameSource::FileName,
        _ => false,
    };
    if !separated || !rest[1..].chars().any(char::is_alphanumeric) {
        return name.to_string();
    }
    report(junk, JunkKind::StoreId, &name[..digits + 1]);
    rest[1..].to_string()
}

/// Underscores become spaces in a file name, and in a title that uses them
/// more than it uses spaces.
fn replace_underscores(name: &str, source: NameSource, junk: &mut Vec<Junk>) -> String {
    let underscores = name.matches('_').count();
    let spaces = name.matches(' ').count();
    if underscores == 0 || (source == NameSource::Title && underscores <= spaces) {
        return name.to_string();
    }
    report(junk, JunkKind::Underscores, "_");
    name.replace('_', " ")
}

/// A track number in front: "01 ", "01. ", "01-", "3. ", "3) ", "1-03 ",
/// and, in a file name with an artist and a title after it, "3 - " and a
/// vinyl side ("A1 - "). A bare number with no leading zero and no
/// punctuation ("7 Lanterns") is left alone: it may be the title.
fn strip_track_number(name: &str, source: NameSource, junk: &mut Vec<Junk>) -> String {
    let chars: Vec<char> = name.chars().collect();
    let Some(cut) = track_number_len(&chars, source) else {
        return name.to_string();
    };
    let rest: String = chars[cut..].iter().collect();
    if !rest.chars().any(char::is_alphanumeric) {
        return name.to_string();
    }
    let prefix: String = chars[..cut].iter().collect();
    report(junk, JunkKind::TrackNumber, prefix.trim());
    rest
}

/// How many characters of `chars` are a track-number prefix.
fn track_number_len(chars: &[char], source: NameSource) -> Option<usize> {
    let file = source == NameSource::FileName;
    let at = |i: usize| chars.get(i).copied();
    let spaces_from = |mut i: usize| {
        while at(i) == Some(' ') {
            i += 1;
        }
        i
    };
    // " - " at `i`, and whether another " - " follows somewhere after it.
    let dash_at =
        |i: usize| at(i) == Some(' ') && at(i + 1).is_some_and(is_dash) && at(i + 2) == Some(' ');
    let dash_later = |from: usize| (from..chars.len()).any(dash_at);

    // A vinyl side: "A1 - Artist - Title".
    if file && at(0).is_some_and(|c| matches!(c.to_ascii_uppercase(), 'A'..='D')) {
        let digits = chars[1..].iter().take_while(|c| c.is_ascii_digit()).count();
        let end = 1 + digits;
        if (1..=2).contains(&digits) && dash_at(end) && dash_later(end + 3) {
            return Some(spaces_from(end + 3));
        }
    }

    let digits = chars.iter().take_while(|c| c.is_ascii_digit()).count();
    if !(1..=3).contains(&digits) {
        return None;
    }
    let leading_zero = digits >= 2 && chars[0] == '0';
    // Disc and track: "1-03 Title".
    if file
        && digits == 1
        && at(1) == Some('-')
        && at(2).is_some_and(|c| c.is_ascii_digit())
        && at(3).is_some_and(|c| c.is_ascii_digit())
        && at(4) == Some(' ')
    {
        return Some(spaces_from(4));
    }
    match at(digits)? {
        '.' | ')' if at(digits + 1) == Some(' ') => Some(spaces_from(digits + 1)),
        '.' if leading_zero && at(digits + 1).is_some_and(char::is_alphabetic) => Some(digits + 1),
        '-' if leading_zero && at(digits + 1).is_some_and(char::is_alphabetic) => Some(digits + 1),
        ' ' if dash_at(digits) => {
            (leading_zero || (file && dash_later(digits + 3))).then(|| spaces_from(digits + 3))
        }
        ' ' if leading_zero => Some(spaces_from(digits)),
        _ => None,
    }
}

/// A word, lowercased, without the punctuation around it.
fn bare_word(word: &str) -> String {
    word.trim_matches(|c: char| !c.is_alphanumeric())
        .to_lowercase()
}

/// Whether a word looks like a site's name: "ripsite.example",
/// "www.ripsite.example".
pub(super) fn looks_like_site(word: &str) -> bool {
    let word = word
        .trim_matches(|c: char| !c.is_alphanumeric())
        .to_lowercase();
    let word = match word.split_once("://") {
        Some((_, after)) => after.to_string(),
        None => word,
    };
    if let Some(rest) = word.strip_prefix("www.") {
        return rest.chars().any(char::is_alphanumeric);
    }
    let labels: Vec<&str> = word.split('.').collect();
    labels.len() >= 2
        && labels.iter().all(|label| {
            !label.is_empty() && label.chars().all(|c| c.is_alphanumeric() || c == '-')
        })
        && labels[0].chars().any(char::is_alphabetic)
        && labels.last().is_some_and(|end| SITE_ENDINGS.contains(end))
}

/// Whether the text is a site's name, alone or with filler words:
/// "RipSite.example", "Downloaded from ripsite.example".
pub(super) fn is_site_phrase(text: &str) -> bool {
    let mut sites = 0;
    for word in text.split_whitespace() {
        if looks_like_site(word) {
            sites += 1;
        } else if !SITE_FILLER_WORDS.contains(&bare_word(word).as_str()) {
            return false;
        }
    }
    sites > 0
}

/// Whether a bracket's content is a rip tag: a phrase from the table, a
/// bitrate, a format name, or the two together ("320kbps MP3").
fn is_rip_tag(content: &str) -> bool {
    let content = collapse_spaces(&content.to_lowercase());
    if RIP_TAGS.contains(&content.as_str()) {
        return true;
    }
    let words: Vec<&str> = content.split(' ').filter(|w| !w.is_empty()).collect();
    !words.is_empty()
        && words.iter().enumerate().all(|(i, word)| {
            FORMAT_WORDS.contains(word)
                || is_bitrate(word)
                || (i > 0 && BITRATE_UNITS.contains(word) && is_bitrate(words[i - 1]))
        })
}

fn is_bitrate(word: &str) -> bool {
    let number = BITRATE_UNITS
        .iter()
        .find_map(|unit| word.strip_suffix(unit))
        .unwrap_or(word);
    number
        .parse::<u32>()
        .is_ok_and(|n| BITRATES.contains(&n) && !number.starts_with('0'))
}

/// Where a bitrate with its unit starts at the end of `text`, written
/// without brackets: "Glasswing 320kbps", "Glasswing 192 kbps". A bare number
/// ("Glasswing 320") is never taken: it may be part of the title.
pub(super) fn trailing_bitrate(text: &str) -> Option<usize> {
    let text = text.trim_end();
    let word_start = text.rfind(char::is_whitespace).map_or(0, |at| {
        at + text[at..].chars().next().map_or(1, char::len_utf8)
    });
    let last = text[word_start..].to_lowercase();
    if is_bitrate_with_unit(&last) {
        return Some(word_start);
    }
    // "192 kbps": the unit on its own, a bitrate in front of it.
    if BITRATE_UNITS.contains(&last.as_str()) {
        let front = text[..word_start].trim_end();
        let number_start = front.rfind(char::is_whitespace).map_or(0, |at| {
            at + front[at..].chars().next().map_or(1, char::len_utf8)
        });
        if is_bitrate(&front[number_start..]) {
            return Some(number_start);
        }
    }
    None
}

fn is_bitrate_with_unit(word: &str) -> bool {
    BITRATE_UNITS
        .iter()
        .any(|unit| word.strip_suffix(unit).is_some() && is_bitrate(word))
}

/// Whether a bracket's content is a running time: "3:45", "1:02:03".
fn is_duration(content: &str) -> bool {
    let parts: Vec<&str> = content.trim().split(':').collect();
    (2..=3).contains(&parts.len())
        && parts.iter().enumerate().all(|(i, p)| {
            p.chars().all(|c| c.is_ascii_digit()) && (p.len() == 2 || (i == 0 && p.len() == 1))
        })
}

/// The kind of junk a bracket's content is, if it is junk.
pub(super) fn bracket_junk(content: &str) -> Option<JunkKind> {
    if is_site_phrase(content) {
        Some(JunkKind::SiteName)
    } else if is_rip_tag(content) {
        Some(JunkKind::RipTag)
    } else if is_duration(content) {
        Some(JunkKind::Duration)
    } else {
        None
    }
}
