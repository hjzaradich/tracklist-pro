//! The lazy matcher against the matcher as it was before titles were read on
//! demand. Its own module, not inside `matching`: the CI shards pick tests
//! by `matching::` in their names.

use std::collections::{HashMap, HashSet};

use super::matching::{
    artist_keys, fact, keys, missing, title_keys, wish_name, Matcher, WishMatch, WishMatchKind,
};
use super::parse::WishTrack;
use crate::search::tests::support::Lib;
use crate::versions::{self, Outcome};

struct Eager<'a> {
    matcher: &'a Matcher,
    by_title: HashMap<String, Vec<usize>>,
    /// The artist keys of each candidate (of a matcher that kept them all).
    artists: Vec<Vec<String>>,
}

impl<'a> Eager<'a> {
    fn new(matcher: &'a Matcher) -> Eager<'a> {
        let mut by_title: HashMap<String, Vec<usize>> = HashMap::new();
        for (at, candidate) in matcher.candidates.iter().enumerate() {
            let parsed = versions::parse_title(&candidate.title);
            for key in title_keys(&parsed) {
                by_title.entry(key).or_default().push(at);
            }
        }
        let mut artists = vec![Vec::new(); matcher.candidates.len()];
        for (key, ats) in &matcher.by_artist {
            for &at in ats {
                artists[at].push(key.clone());
            }
        }
        Eager {
            matcher,
            by_title,
            artists,
        }
    }

    fn owners(&self, wish: &WishTrack) -> Vec<WishMatch> {
        let m = self.matcher;
        if let Some(recording) = m.by_isrc_of(wish) {
            return vec![m.owned(recording, true)];
        }
        let parsed = versions::parse_title(&wish_name(wish));
        let artists = artist_keys(&wish.artist);
        let mut found: Vec<i64> = Vec::new();
        for key in title_keys(&parsed) {
            for &at in self.by_title.get(&key).into_iter().flatten() {
                let candidate = &m.candidates[at];
                if found.contains(&candidate.recording)
                    || !self.artists[at].iter().any(|a| artists.contains(a))
                {
                    continue;
                }
                let theirs = versions::parse_title(&candidate.title);
                if versions::compare(&parsed, &theirs).outcome == Outcome::SameVersion {
                    found.push(candidate.recording);
                }
            }
        }
        found.sort_unstable();
        found.into_iter().map(|r| m.owned(r, false)).collect()
    }

    fn find(&self, wish: &WishTrack) -> WishMatch {
        let m = self.matcher;
        if let Some(recording) = m.by_isrc_of(wish) {
            return m.owned(recording, true);
        }
        let parsed = versions::parse_title(&wish_name(wish));
        let artists = artist_keys(&wish.artist);
        let mut best: Option<(u8, bool, std::cmp::Reverse<i64>, usize)> = None;
        let mut seen = HashSet::new();
        for key in title_keys(&parsed) {
            for &at in self.by_title.get(&key).into_iter().flatten() {
                let candidate = &m.candidates[at];
                if !seen.insert(at) || !self.artists[at].iter().any(|a| artists.contains(a)) {
                    continue;
                }
                let rank =
                    match versions::compare(&parsed, &versions::parse_title(&candidate.title))
                        .outcome
                    {
                        Outcome::SameVersion => 3,
                        Outcome::DifferentCut => 2,
                        Outcome::DifferentRework => 1,
                        Outcome::DifferentBase | Outcome::CantTell => continue,
                    };
                let ranked = (
                    rank,
                    candidate.in_library,
                    std::cmp::Reverse(candidate.recording),
                    at,
                );
                if best.is_none_or(|b| ranked > b) {
                    best = Some(ranked);
                }
            }
        }
        let Some((rank, in_library, _, at)) = best else {
            return missing();
        };
        let candidate = &m.candidates[at];
        let (kind, key) = match (rank, in_library) {
            (3, _) => return m.owned(candidate.recording, false),
            (2, true) => (WishMatchKind::OwnedOtherCut, keys::OTHER_CUT_LIBRARY),
            (2, false) => (WishMatchKind::OwnedOtherCut, keys::OTHER_CUT_ALL_MUSIC),
            _ => (WishMatchKind::Rework, keys::REWORK),
        };
        WishMatch {
            kind,
            reason: fact(key, Some(&candidate.title)),
            recording_id: Some(candidate.recording),
            in_library,
        }
    }
}

fn wish(artist: &str, title: &str, version: Option<&str>, isrc: Option<&str>) -> WishTrack {
    WishTrack {
        artist: artist.into(),
        title: title.into(),
        version: version.map(Into::into),
        isrc: isrc.map(Into::into),
    }
}

/// Sets a file's tags to a title, an artist and an ISRC.
fn tagged_with_isrc(lib: &Lib, file: i64, title: &str, artist: &str, isrc: &str) {
    let raw = serde_json::json!({ "id3v2": [
        {"key": "TIT2", "value": {"type": "text", "text": title}},
        {"key": "TPE1", "value": {"type": "text", "text": artist}},
        {"key": "TSRC", "value": {"type": "text", "text": isrc}},
    ]})
    .to_string();
    lib.insert("UPDATE file SET raw_tags = ?1 WHERE id = ?2", (raw, file));
}

#[test]
fn matching_on_demand_answers_as_reading_every_title_up_front_did() {
    let lib = Lib::new();
    let mut made: Vec<(i64, i64)> = Vec::new();
    let mut add = |title: Option<&str>, artist: Option<&str>| {
        let made_one = lib.track(title, artist, &format!("t{}.mp3", made.len()));
        made.push(made_one);
        made_one
    };
    let (a, fa) = add(Some("Paper Harbor (Extended Mix)"), Some("Quill Ashby"));
    add(Some("Paper Harbor (Radio Edit)"), Some("Quill Ashby"));
    add(Some("Paper Harbor (Nova Reed Remix)"), Some("Quill Ashby"));
    add(Some("Paper Harbor"), Some("Quill Ashby & Nova Reed"));
    add(Some("Paper Harbor (Extended Mix)"), Some("Nova Reed"));
    add(Some("Paper Harbor (Extended Mix)"), Some("Quill Ashby"));
    add(Some("Lantern - Dub"), Some("Nova Reed feat. Quill Ashby"));
    add(Some("Caf\u{e9} Del Rio"), Some("Mara Voss"));
    add(Some("Salt and Pepper"), Some("Mara Voss"));
    add(Some("Salt & Pepper (Club Mix)"), Some("Mara Voss"));
    add(None, Some("Mara Voss"));
    add(Some("No Artist Here"), None);
    add(Some("(Intro)"), Some("Quill Ashby"));
    let (_, fiso) = add(Some("Isrc One"), Some("Dee Fen"));
    let (iso2, fiso2) = add(Some("Isrc Two"), Some("Dee Fen"));
    let (_, fgone) = add(Some("Hidden Away"), Some("Quill Ashby"));
    tagged_with_isrc(&lib, fiso, "Isrc One", "Dee Fen", "USAB12345678");
    tagged_with_isrc(&lib, fiso2, "Isrc Two", "Dee Fen", "USAB12345678");
    lib.library(a, fa);
    lib.library(iso2, fiso2);
    lib.insert("UPDATE file SET present = 0 WHERE id = ?1", (fgone,));
    let wishes = vec![
        wish("Quill Ashby", "Paper Harbor", Some("Extended Mix"), None),
        wish("Quill Ashby", "Paper Harbor", None, None),
        wish("Quill Ashby", "Paper Harbor", Some("Radio Edit"), None),
        wish("Quill Ashby", "Paper Harbor", Some("Short Mix"), None),
        wish("Quill Ashby", "Paper Harbor", Some("Nova Reed Remix"), None),
        wish(
            "Quill Ashby",
            "Paper Harbor",
            Some("Other Person Remix"),
            None,
        ),
        wish("Nova Reed", "Paper Harbor", Some("Extended Mix"), None),
        wish("Nova Reed", "Paper Harbor", None, None),
        wish("Nova Reed & Quill Ashby", "Paper Harbor", None, None),
        wish("Nova Reed", "Lantern", Some("Dub"), None),
        wish("Quill Ashby", "Lantern - Dub", None, None),
        wish("MARA VOSS", "Cafe Del Rio", None, None),
        wish("Mara Voss", "Salt & Pepper", None, None),
        wish("Mara Voss", "Salt and Pepper", Some("Club Mix"), None),
        wish("Mara Voss", "Track 1", None, None),
        wish("Quill Ashby", "(Intro)", None, None),
        wish("Nobody", "Nothing", None, None),
        wish("Quill Ashby", "Hidden Away", None, None),
        wish("Dee Fen", "Isrc Two", None, Some("us-ab-12-345678")),
        wish("Someone", "Else", None, Some("usab12345678")),
        wish("Dee Fen", "Isrc One", None, Some("USZZ99999999")),
        wish("Dee Fen", "Isrc One", None, Some("not an isrc")),
        wish("", "Paper Harbor", None, None),
    ];
    let owned_ids: Vec<i64> = made.iter().map(|(r, _)| *r).collect();
    // Everything is compared on the writer's thread; what comes back is
    // the list of differences and the kinds the set covers.
    let (differences, kinds, several) = lib
        .writer
        .call(move |c| {
            let every = Matcher::build(c)?;
            let wanting = Matcher::build_wanting(c, None, &wishes)?;
            let among = Matcher::build_wanting(c, Some(&owned_ids), &wishes)?;
            let reference = Eager::new(&every);
            let mut differences = Vec::new();
            let mut kinds = HashSet::new();
            let mut several = false;
            for w in &wishes {
                let want = (reference.find(w), reference.owners(w));
                kinds.insert(format!("{:?}", want.0.kind));
                several |= want.1.len() > 1;
                for (name, m) in [("build", &every), ("wanting", &wanting), ("among", &among)] {
                    if (m.find(w), m.owners(w)) != want {
                        differences.push(format!("{name}: {w:?}"));
                    }
                }
            }
            Ok((differences, kinds.len(), several))
        })
        .unwrap();
    assert!(differences.is_empty(), "{differences:#?}");
    // The set really is mixed: owned, other cut, rework and missing all
    // come up, and an item can be owned as several tracks.
    assert_eq!(kinds, 4);
    assert!(several);
}
