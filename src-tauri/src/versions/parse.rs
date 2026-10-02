//! Puts the pieces together: clean the name, tokenize it, set junk aside,
//! split artist from title, read the labels, and say what's doubtful.

use super::junk::{self, bracket_junk, is_site_phrase, looks_like_site};
use super::labels::{self, Found};
use super::tokenize::{collapse_spaces, flatten, tokenize, BracketKind, Group, Token};
use super::{
    Credit, CreditRole, Junk, JunkKind, Marker, MarkerForm, MarkerKind, NameSource, ParsedName,
    Qualifier,
};

/// Whether the text holds anything but spaces and plain punctuation.
fn has_content(text: &str) -> bool {
    text.chars()
        .any(|c| !c.is_whitespace() && !c.is_ascii_punctuation())
}

fn qualifier(found: Vec<Found>, text: &str, form: MarkerForm, ambiguous: bool) -> Qualifier {
    Qualifier {
        as_written: text.to_string(),
        markers: found
            .into_iter()
            .map(|f| Marker {
                kind: f.kind,
                detail: f.detail,
                text: text.to_string(),
                form,
                ambiguous,
            })
            .collect(),
    }
}

/// Whether two qualifiers say the same thing ("(dirty)" and "(Dirty)").
fn same_labels(a: &Qualifier, b: &Qualifier) -> bool {
    let labels = |q: &Qualifier| -> Vec<(MarkerKind, Option<String>)> {
        q.markers
            .iter()
            .map(|m| (m.kind, m.detail.as_ref().map(|d| d.to_lowercase())))
            .collect()
    };
    labels(a) == labels(b)
}

/// The names in a bracket that is a featuring credit: "(feat. Someone)".
fn featured(own: &str) -> Option<Vec<String>> {
    let (before, names) = labels::split_feat(own)?;
    (!has_content(&before)).then(|| labels::split_names(&names, false))
}

/// The kind of junk a bracket is, if it is junk. Besides the table's
/// patterns, a square bracket padded with spaces ("[ example-ripper ]") is
/// a rip tag unless it holds a label or a credit.
fn group_junk(group: &Group, own: &str) -> Option<JunkKind> {
    bracket_junk(own).or_else(|| {
        let padded = group.kind == BracketKind::Square
            && group.raw.starts_with("[ ")
            && group.raw.ends_with(" ]");
        (padded && labels::parse_content(own).is_none() && featured(own).is_none())
            .then_some(JunkKind::RipTag)
    })
}

/// Takes junk brackets out of the name, wherever they sit.
fn remove_junk_groups(tokens: &mut Vec<Token>, junk: &mut Vec<Junk>) {
    tokens.retain(|token| {
        let Token::Group(group) = token else {
            return true;
        };
        let own = group.own_text();
        if own.is_empty() || group.nested().next().is_some() || featured(&own).is_some() {
            return true;
        }
        match group_junk(group, &own) {
            Some(kind) => {
                junk.push(Junk {
                    kind,
                    text: group.raw.clone(),
                });
                false
            }
            None => true,
        }
    });
}

/// Splits the name at its " - " separators; empty segments are dropped.
fn split_segments(tokens: Vec<Token>) -> Vec<Vec<Token>> {
    let mut segments = vec![Vec::new()];
    for token in tokens {
        match token {
            Token::Dash => segments.push(Vec::new()),
            other => {
                if let Some(last) = segments.last_mut() {
                    last.push(other);
                }
            }
        }
    }
    segments.retain(|segment| has_content(&flatten(segment)));
    segments
}

/// A segment's text, when it has no brackets.
fn only_text(segment: &[Token]) -> Option<String> {
    segment
        .iter()
        .all(|t| matches!(t, Token::Text(_)))
        .then(|| flatten(segment))
}

/// Site names as whole " - " segments, first or last:
/// "RipSite.example - Artist - Title".
fn strip_site_segments(segments: &mut Vec<Vec<Token>>, junk: &mut Vec<Junk>) {
    while segments.len() > 1 {
        let is_site = |segment: &Vec<Token>| only_text(segment).filter(|text| is_site_phrase(text));
        let (at, text) = if let Some(text) = is_site(&segments[0]) {
            (0, text)
        } else if let Some(text) = segments.last().and_then(is_site) {
            (segments.len() - 1, text)
        } else {
            return;
        };
        junk.push(Junk {
            kind: JunkKind::SiteName,
            text,
        });
        segments.remove(at);
    }
}

/// A site name as the first or last word of the whole name:
/// "RipSite.example Artist - Title".
fn strip_site_words(segments: &mut [Vec<Token>], junk: &mut Vec<Junk>) {
    if let Some(Token::Text(text)) = segments.first_mut().and_then(|s| s.first_mut()) {
        let trimmed = text.trim_start().to_string();
        if let Some(word) = trimmed.split_whitespace().next() {
            let rest = &trimmed[word.len()..];
            if looks_like_site(word) && has_content(rest) {
                junk.push(Junk {
                    kind: JunkKind::SiteName,
                    text: word.to_string(),
                });
                *text = rest.to_string();
            }
        }
    }
    if let Some(Token::Text(text)) = segments.last_mut().and_then(|s| s.last_mut()) {
        let trimmed = text.trim_end().to_string();
        if let Some(word) = trimmed.split_whitespace().next_back() {
            let rest = &trimmed[..trimmed.len() - word.len()];
            if looks_like_site(word) && has_content(rest) {
                junk.push(Junk {
                    kind: JunkKind::SiteName,
                    text: word.to_string(),
                });
                *text = rest.to_string();
            }
        }
    }
}

/// The artists in front of the title: "A & B feat. C".
fn artist_credits(text: &str, credits: &mut Vec<Credit>) {
    let without_brackets: String = text
        .chars()
        .map(|c| if "()[]{}".contains(c) { ' ' } else { c })
        .collect();
    let (main, featured) = match labels::split_feat(&without_brackets) {
        Some((main, featured)) => (main, Some(featured)),
        None => (text.to_string(), None),
    };
    for name in labels::split_names(&main, true) {
        credits.push(Credit {
            name,
            role: CreditRole::Main,
        });
    }
    for name in featured.iter().flat_map(|f| labels::split_names(f, false)) {
        credits.push(Credit {
            name,
            role: CreditRole::Featured,
        });
    }
}

/// What the brackets after the title turned out to be.
#[derive(Default)]
struct Trailing {
    credits: Vec<Credit>,
    junk: Vec<Junk>,
    qualifiers: Vec<Qualifier>,
    unrecognized: Vec<String>,
}

fn classify(group: &Group, out: &mut Trailing) {
    let own = group.own_text();
    if own.is_empty() {
        if group.nested().next().is_none() {
            out.unrecognized.push(group.raw.clone());
        }
    } else if let Some(names) = featured(&own) {
        out.credits.extend(names.into_iter().map(|name| Credit {
            name,
            role: CreditRole::Featured,
        }));
    } else if let Some(kind) = group_junk(group, &own) {
        out.junk.push(Junk {
            kind,
            text: group.raw.clone(),
        });
    } else if let Some(found) = labels::parse_content(&own) {
        out.qualifiers
            .push(qualifier(found, &group.raw, MarkerForm::Bracketed, false));
    } else {
        out.unrecognized.push(group.raw.clone());
    }
    for nested in group.nested() {
        classify(nested, out);
    }
}

pub(super) fn parse(input: &str, source: NameSource) -> ParsedName {
    let mut junk = Vec::new();
    let cleaned = junk::clean(input, source, &mut junk);
    let mut tokens = tokenize(&cleaned);
    remove_junk_groups(&mut tokens, &mut junk);
    let mut segments = split_segments(tokens);
    strip_site_segments(&mut segments, &mut junk);
    strip_site_words(&mut segments, &mut junk);

    // Labels after a " - ": "Title - Extended Mix". A file name keeps two
    // segments, its artist and its title, whatever they say.
    let keep = match source {
        NameSource::Title => 1,
        NameSource::FileName => 2,
    };
    let mut dash_qualifiers = Vec::new();
    while segments.len() > keep {
        let Some(text) = segments.last().and_then(|s| only_text(s)) else {
            break;
        };
        let Some(found) = labels::parse_content(&text) else {
            break;
        };
        dash_qualifiers.insert(0, qualifier(found, &text, MarkerForm::DashSegment, true));
        segments.pop();
    }

    let mut credits = Vec::new();
    if source == NameSource::FileName && segments.len() >= 2 {
        let artist = segments.remove(0);
        artist_credits(&flatten(&artist), &mut credits);
    }
    let mut title: Vec<Token> = Vec::new();
    for (n, segment) in segments.into_iter().enumerate() {
        if n > 0 {
            title.push(Token::Text(" - ".to_string()));
        }
        title.extend(segment);
    }

    // Featuring credits written in the title's text. The word needs a
    // title in front of it: "Featuring You" alone is a title.
    let mut seen = false;
    for token in &mut title {
        match token {
            Token::Text(text) => {
                if let Some((before, names)) = labels::split_feat(text) {
                    if seen || has_content(&before) {
                        credits.extend(labels::split_names(&names, false).into_iter().map(
                            |name| Credit {
                                name,
                                role: CreditRole::Featured,
                            },
                        ));
                        *text = before;
                    }
                }
                seen |= has_content(text);
            }
            _ => seen = true,
        }
    }

    // Brackets after the last of the title's text are its qualifiers;
    // brackets inside the title stay part of it.
    let last_text = title
        .iter()
        .rposition(|t| matches!(t, Token::Text(text) if has_content(text)));
    let trailing_tokens = title.split_off(last_text.map_or(0, |at| at + 1));
    let mut trailing = Trailing::default();
    for token in &trailing_tokens {
        if let Token::Group(group) = token {
            classify(group, &mut trailing);
        }
    }
    credits.extend(trailing.credits);
    junk.extend(trailing.junk);
    let mut unrecognized = trailing.unrecognized;
    let mut core = flatten(&title);

    // Label words at the end of the title, without brackets: kept in the
    // title, and listed as possible markers.
    let mut qualifiers = Vec::new();
    let mut folded = 0;
    if let Some((front, tail, found)) = labels::bare_tail(&core) {
        qualifiers.push(qualifier(found, &tail, MarkerForm::Bare, true));
        core = front;
        folded = 1;
    }
    qualifiers.extend(trailing.qualifiers);
    qualifiers.extend(dash_qualifiers);

    if core.is_empty() && qualifiers.is_empty() {
        // Nothing but brackets or junk: the name is its own title.
        core = match unrecognized.is_empty() {
            false => unrecognized.remove(0),
            true => cleaned.clone(),
        };
    }
    // A title can't be empty: "(Intro)" alone is a title.
    if core.is_empty() && folded == 0 {
        folded = 1;
    }
    // The same label twice: the first belongs to the title
    // ("Velo (dirty) (dirty)" is the dirty cut of "Velo (dirty)").
    if folded < qualifiers.len()
        && qualifiers[folded + 1..]
            .iter()
            .any(|later| same_labels(later, &qualifiers[folded]))
    {
        folded += 1;
    }
    let folded = folded.min(qualifiers.len());
    for qualifier in &mut qualifiers[..folded] {
        for marker in &mut qualifier.markers {
            marker.ambiguous = true;
        }
    }

    // "A x B": a mashup's parts. Without a mashup label, the "x" itself is
    // the (doubtful) marker.
    let (mashup_parts, separator) = match labels::mashup_parts(&core) {
        Some((parts, word)) => {
            let labelled = qualifiers
                .iter()
                .flat_map(|q| &q.markers)
                .any(|m| m.kind == MarkerKind::Mashup);
            let marker = (!labelled).then_some(Marker {
                kind: MarkerKind::Mashup,
                detail: None,
                text: word,
                form: MarkerForm::Separator,
                ambiguous: true,
            });
            (parts, marker)
        }
        None => (Vec::new(), None),
    };

    let flat = |qualifiers: &[Qualifier]| -> Vec<Marker> {
        qualifiers
            .iter()
            .flat_map(|q| q.markers.iter().cloned())
            .collect()
    };
    let mut markers = flat(&qualifiers[folded..]);
    markers.extend(separator.iter().cloned());
    let mut parsed = ParsedName {
        original: input.to_string(),
        source,
        base_title: String::new(),
        credits,
        markers,
        title_markers: flat(&qualifiers[..folded]),
        mashup_parts,
        unrecognized,
        junk,
        core: collapse_spaces(&core),
        qualifiers,
        folded,
        separator,
    };
    parsed.base_title = parsed.base_with(folded);
    parsed
}
