//! The grouping rules: what two files are to each other (1bB-1 to 1bB-5;
//! ROADMAP 1.4, 1.5).
//!
//! A pure decision library: no database, no files, no jobs, no IPC. Given
//! what is known about two files, [`decide`] answers with one [`Outcome`]
//! (duplicate, cut link, rework link, different, or needs review) and the
//! [`Reason`]s for it. A reason is a code plus parameters, never English
//! text; its wording arrives with the Review screen (1bD).
//!
//! Nothing here is wired in yet: the grouping job (1bC-1) calls it and
//! stores what it says. The rules don't decide *when* a pair is asked
//! about, or whether to wait for a fingerprint that hasn't been made yet:
//! that is the job's business. Asked about a file with no fingerprint,
//! they answer for what is known ([`Audio::Unknown`]).
//!
//! # The two inputs
//!
//! **The audio** ([`Audio`]), sorted into one of seven cases:
//!
//! | Case | Means |
//! |---|---|
//! | identical | the same `audio_hash`, or the same `blake3` |
//! | full | coverage ≥ 90% on both files and a difference score ≤ 4 (ROADMAP 1.4, unchanged; the main alignment only) |
//! | one inside the other | ≥ 90% of exactly one file is found in the other, over every alignment, with a difference score ≤ 4 over everything found |
//! | partial | anything else `matching` reports as matched ([`MIN_SHARED_ITEMS`]) |
//! | none | the fingerprints were compared and nothing matched |
//! | not compared | both files have fingerprints, and nobody compared them |
//! | unknown | a file has no fingerprint |
//!
//! Whether two files are "identical" is the caller's to work out; the
//! rules take it as given. Measurements are checked when they are made
//! ([`Measured::new`]): a share outside 0 to 1 is refused, not judged.
//!
//! **The names** ([`FileFacts`]): each file's title tag and file name, both
//! parsed, and its artist tag. [`crate::versions::compare`] says how two
//! names relate (the same, another cut, another rework, another title, a
//! mashup naming the other title as a part, or unclear), and whether the
//! artists agree, differ, or differ only because one file is credited to
//! the remixer.
//!
//! # The caller's contract: "none" is a finding, "not compared" isn't
//!
//! The matching pass only compares the pairs its block index proposes, and
//! it rarely proposes a weak match: two files can share a good part of
//! their audio and have no stored comparison. So a missing comparison
//! never counts as "no audio shared" ([`Audio::NotCompared`]).
//!
//! **Before asking, the grouping job compares directly
//! ([`crate::matching::compare()`]) every pair whose names are the same, or
//! where one is a mashup naming the other as a part.** Those are the pairs
//! a weak match changes the answer for and the names alone would link.
//! "Not compared" reaching the rules for such a pair is the caller's gap;
//! the rules then answer:
//!
//! - the same names → needs review (`notCompared`), never "different";
//! - anything else → what they answer when no audio is shared.
//!
//! Shared audio also changes the answer under one title with different
//! artists, and under names that can't be told (review instead of
//! "different"). The contract doesn't ask for those pairs to be compared:
//! never compared, they are "different".
//!
//! # A file's version is what either of its names says
//!
//! A version label can sit in the title tag, in the file name, or in both.
//! A file's labels are those of both names together
//! ([`crate::versions::ParsedName::with_labels_of`]), so each file is one
//! set of labels, and the two files' sets are compared. A file tagged
//! "Velo" and named "Velo (Dirty).mp3" is a Dirty cut whatever the other
//! file's names look like; two files that both say "(Dirty)", one in its
//! tag and one in its file name, are the same version. As within one name
//! (ROADMAP 1.5), a version word is only a label when the other file
//! lacks it.
//!
//! # A file's own two names
//!
//! Labels are only what a name brackets or sets off with " - ". Label
//! words a title keeps ("Velo Dirty"), and a mashup's "x", can't be
//! carried from one name to the other. So a file's own title tag and file
//! name, with their labels put together, are set side by side through
//! [`crate::versions::compare`], whatever the other file is called:
//!
//! - **The same, or unrelated titles:** nothing more to learn.
//! - **They differ by a cut:** one holds in its title the word the other
//!   brackets ("Come Clean" beside "Come Clean (Clean)"). The file carries
//!   that cut, and both its names get it.
//! - **Anything else** (a bare "Dirty" at the end of one, a mashup's "x"
//!   in one): the file's version is in doubt.
//!
//! **The net under the parser.** The parser only reads a label in
//! brackets at the end of a title, after " - ", or as the very last words.
//! "Velo Dirty 128.mp3", "Dirty Velo.mp3", "Velo-Dirty.mp3" and "Velo
//! (Clean) - Halden Rook.mp3" are, to it, titles unrelated to the tag
//! "Velo". So when the two names come out unrelated, they are looked at
//! word by word (split at spaces, hyphens, underscores, brackets and
//! dots): if one name holds the other's whole title, as whole words
//! anywhere, and any of its remaining words is a label the parser knows
//! ([`crate::versions::label_kind`], up to three words, "Original" aside),
//! the file's version is in doubt too. Words of the file's own artist
//! (its artist tag, or the artist part of the name) and labels the parser
//! did read are not "remaining words": a file by "Dub Kestrel" raises
//! nothing. Without an artist tag or a " - ", an artist's label word can't
//! be told from a label, and counts.
//!
//! A doubt only counts with the audio matched, and only when the pair's
//! names would otherwise be the same, or have the same labels under two
//! titles: the pair's names are then unclear (`ownNamesDisagree`). On a
//! full match that means review instead of a merge; one file inside the
//! other stays a cut link, and identical audio a duplicate. Two files with
//! the same doubt (the same two titles) don't differ by it.
//!
//! # Two readings of the titles
//!
//! The *titles* are still read twice: by title tag (a file with none
//! stands in with its file name), and by file name. One reading is picked,
//! and the table applied to it:
//!
//! - A reading that says *another cut* or *another rework* always wins:
//!   both readings carry the files' whole labels, so it is the one that
//!   puts the two titles together.
//! - **When the audio says "one production"** (identical, full, one inside
//!   the other), the most cautious reading comes next, so that nothing
//!   merges that either reading would keep apart: *mashup part*, then
//!   *unclear*, then *same*.
//! - **Otherwise the names decide**, and *same* comes before *mashup part*
//!   and *unclear*: the reading under which the names agree says the most.
//! - *Another title* is always last: it never hides what the other reading
//!   says.
//!
//! Of two readings that say the same kind of thing, the tags' wins.
//!
//! **Under two titles on both readings**, nothing above sets the labels
//! side by side. With the audio matched (full, or one inside the other)
//! they are then compared whatever the titles
//! ([`crate::versions::compare_labels`]): "Velo (Clean)" and "Nightdrive
//! (Dirty)" with the same audio are a cut link, not a duplicate. The
//! decision says so with `titlesDiffer` at the end of its reasons.
//!
//! # The table
//!
//! | Audio | Names | Outcome | Deciding reason |
//! |---|---|---|---|
//! | identical | any | duplicate | `identicalAudio` |
//! | full | same, or another title with the same labels | duplicate | `sameAudio` |
//! | full | another cut | cut link | `versionVeto` |
//! | full | another rework, unclear, mashup part | needs review | `sameAudioNamesDisagree` |
//! | one inside the other | same, another cut, unclear | cut link | `audioInsideOther` |
//! | one inside the other | another rework, another title | needs review | `audioInsideOtherNamesDisagree` |
//! | one inside the other, partial | mashup part | rework link (mashup) | `mashupContains` |
//! | partial | same | needs review | `sameTagsPartialAudio` |
//! | partial | unclear, or the artists differ | needs review | `partialAudioNamesUnclear` |
//! | partial, none, not compared, unknown | another cut | cut link | `namesSayCut` |
//! | partial, none, not compared, unknown | another rework | rework link | `namesSayRework` |
//! | none | same | different | `fingerprintVeto` |
//! | not compared | same | needs review | `notCompared` |
//! | unknown | same | needs review | `noFingerprint` |
//! | none, not compared, unknown | same title, the artists differ | different | `sameTitleDifferentArtists` |
//! | none, not compared, unknown | mashup part | different | `mashupPart` |
//! | none, not compared, unknown | unclear | different | `namesUnclear` |
//! | partial, none, not compared, unknown | another title | different | `titlesDiffer` |
//!
//! A file whose names give both a rework and a cut the other lacks counts
//! as another rework, as within one name.
//!
//! The first reason of a [`Decision`] is the deciding one. The ones after
//! it are facts worth showing beside it: `namesDiffer` on a duplicate whose
//! names don't agree, `artistsDiffer`, `creditedToRemixer`, what the names
//! said when the audio overruled or doubted them, and `titlesDiffer` when
//! the labels were compared under two titles.
//!
//! A link made by names alone (`namesSayCut`, `namesSayRework`) needs the
//! artists to agree: the same title can be a different song. One file being
//! credited to the remixer the names give counts as agreeing.
//!
//! # The final guard
//!
//! Everything above reads names as the parser does, and the parser only
//! reads a label where convention puts one. So one invariant is checked
//! last, in front of every merge on a full (not identical) audio match:
//!
//! **Two files with a full audio match merge silently only if their names
//! carry the same version words.**
//!
//! - A file's version words are the label words the parser knows
//!   ([`crate::versions::label_kind`]: the longest label first, up to three
//!   words, "Original" aside), in brackets or bare, anywhere in its title
//!   tag or its file name, split at spaces, hyphens, underscores, brackets
//!   and dots. Each label counts as often as the one name that writes it
//!   most does: "Come Clean" named "Come Clean (Clean).mp3" is Clean
//!   twice; named "nightdrive promo.mp3", once. Intro and Outro together
//!   are Intro-Outro.
//! - The file's artist is set aside only where a name spells the whole
//!   artist out, as words in a row: by "Dub Kestrel", "Dub Kestrel - Velo"
//!   carries no version word, and "Velo Dub 128" carries Dub. The artists
//!   are the artist tag's, a title tag's "feat.", and a file name's artist
//!   part when the title tag bears it out (the tag's title is in the
//!   name's title part and not in its artist part, and the artist part is
//!   more than label words).
//! - If the two files' version words differ, a merge becomes "needs
//!   review" (`labelWordsDiffer`, with the labels each file has more of).
//!   A pair the table already keeps apart (a cut link, review) is left as
//!   it is. Identical audio still merges whatever the names; one file
//!   inside the other and the rows without matched audio are untouched.
//!
//! The price is review items for pairs a DJ would call the same: a title
//! that holds a label word ("Come Clean") beside the same audio under
//! another title, or an untagged file by an artist named with a label
//! word beside one that doesn't name the artist.
//!
//! # What only merges on audio
//!
//! Nothing is ever a duplicate on names. Only identical or full audio
//! merges, and full audio doesn't when the two files' labels differ, or
//! can't be told, or their names don't carry the same version words:
//! Clean and Dirty fingerprint identically.
//!
//! # Known limits
//!
//! - **A truncated copy looks like a cut.** A file cut short holds audio
//!   that sits inside the whole file, so with the same names it gets a cut
//!   link. The rules have no quality input; the grouping job (1bC) has the
//!   quality verdict and decides what to do with such a pair.
//!   [`Pair`] is built through [`Pair::new`] so that a new input doesn't
//!   break its callers.
//! - **The score line for "one inside the other" is 4**, the duplicate
//!   rule's. A real cut whose found share is over 90% at a weaker score is
//!   a partial match, and is linked only if the names say it is a cut. The
//!   line is one constant ([`DUPLICATE_SCORE`]).
//! - **A remix credited to its remixer** agrees with any track of the same
//!   title, whoever made it: the names can't say which song was remixed.
//! - **A label word fused to another word** ("VeloDirty.mp3", "Velo
//!   Dirty128.mp3") is not seen by anything here: there is no word break
//!   to find it by. Such a file merges with its plain twin on full audio.
//! - **Words the parser doesn't know as labels** ("Acoustic", "Vocal Mix")
//!   are no version words.
//! - **A bare label word beside the same label in brackets** ("Velo
//!   Dirty.mp3" beside "Velo (Dirty).mp3", both tagged "Velo") is a cut
//!   link, though both are probably the Dirty cut: linked, never merged.

use std::collections::BTreeMap;

use serde::{Serialize, Serializer};

use unicode_normalization::char::is_combining_mark;
use unicode_normalization::UnicodeNormalization;

use crate::matching;
use crate::versions::{self, MarkerKind, ParsedName};

#[cfg(test)]
mod tests;

/// The duplicate rule's coverage: at least this share of a file, 0 to 1
/// (ROADMAP 1.4).
pub const DUPLICATE_COVERAGE: f32 = 0.9;

/// The duplicate rule's difference score: at most this many differing bits
/// per fingerprint item (ROADMAP 1.4).
pub const DUPLICATE_SCORE: f64 = 4.0;

/// Two files share audio when at least this many fingerprint items match,
/// over every alignment. It is 1, the least `matching` ever reports: its
/// main alignment keeps any stretch of one item or more (about 0.12 s),
/// and a secondary alignment only stretches of
/// [`matching::compare::MIN_SECONDARY_ITEMS`] or more. The one place to
/// raise the floor if short chance matches turn out to matter.
pub const MIN_SHARED_ITEMS: usize = 1;

/// The doubt given when the two files' names are the same, but one file's
/// own title tag and file name disagree about its version in a way the
/// parser can't settle (see "A file's own two names" in the module docs).
pub const OWN_NAMES_DISAGREE: &str = "ownNamesDisagree";

/// One of the two files of a [`Pair`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Side {
    A,
    B,
}

impl Side {
    pub fn other(self) -> Side {
        match self {
            Side::A => Side::B,
            Side::B => Side::A,
        }
    }
}

impl From<versions::Side> for Side {
    fn from(side: versions::Side) -> Side {
        match side {
            versions::Side::A => Side::A,
            versions::Side::B => Side::B,
        }
    }
}

/// What `matching` measured between two files' full-track fingerprints.
/// The definitions are in [`crate::matching`]'s docs.
///
/// Only made through [`Measured::new`] or from a
/// [`matching::Comparison`], which both refuse numbers that can't be
/// measurements: a share written as a percent would otherwise pass for a
/// full match.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Measured {
    /// The share of A the main alignment matched, 0 to 1.
    coverage_a: f32,
    /// The share of B the main alignment matched, 0 to 1.
    coverage_b: f32,
    /// The difference score over the main alignment.
    score: f64,
    /// The share of A found anywhere in B, over every alignment.
    found_a: f32,
    /// The share of B found anywhere in A, over every alignment.
    found_b: f32,
    /// The difference score over everything found: every alignment's
    /// stretches, weighted by length. Equal to `score` when the main
    /// alignment is the only one.
    found_score: f64,
    /// How many fingerprint items matched, over every alignment.
    matched_items: usize,
}

/// Why numbers were refused as measurements. Each names the field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BadMeasurement {
    /// A share isn't a number from 0 to 1.
    NotAShare(&'static str),
    /// A difference score isn't a number from 0 to 32.
    NotAScore(&'static str),
    /// Nothing matched, yet a share of a file is said to be matched.
    SharesWithNothingMatched,
}

impl std::fmt::Display for BadMeasurement {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BadMeasurement::NotAShare(field) => write!(f, "{field} is not a share from 0 to 1"),
            BadMeasurement::NotAScore(field) => write!(f, "{field} is not a score from 0 to 32"),
            BadMeasurement::SharesWithNothingMatched => {
                write!(f, "no items matched, but a share of a file is matched")
            }
        }
    }
}

impl std::error::Error for BadMeasurement {}

impl Measured {
    /// Measurements, checked: every share is a number from 0 to 1, every
    /// score a number from 0 to [`matching::compare::NO_MATCH_SCORE`], and
    /// with no item matched every share is 0.
    pub fn new(
        coverage_a: f32,
        coverage_b: f32,
        score: f64,
        found_a: f32,
        found_b: f32,
        found_score: f64,
        matched_items: usize,
    ) -> Result<Measured, BadMeasurement> {
        let shares = [
            ("coverage_a", coverage_a),
            ("coverage_b", coverage_b),
            ("found_a", found_a),
            ("found_b", found_b),
        ];
        for (field, share) in shares {
            if !(0.0..=1.0).contains(&share) {
                return Err(BadMeasurement::NotAShare(field));
            }
        }
        for (field, value) in [("score", score), ("found_score", found_score)] {
            if !(0.0..=matching::compare::NO_MATCH_SCORE).contains(&value) {
                return Err(BadMeasurement::NotAScore(field));
            }
        }
        if matched_items == 0 && shares.iter().any(|(_, share)| *share > 0.0) {
            return Err(BadMeasurement::SharesWithNothingMatched);
        }
        Ok(Measured {
            coverage_a,
            coverage_b,
            score,
            found_a,
            found_b,
            found_score,
            matched_items,
        })
    }

    /// The same measurements with the two files swapped.
    pub fn swapped(self) -> Measured {
        Measured {
            coverage_a: self.coverage_b,
            coverage_b: self.coverage_a,
            found_a: self.found_b,
            found_b: self.found_a,
            ..self
        }
    }
}

impl TryFrom<&matching::Comparison> for Measured {
    type Error = BadMeasurement;

    /// A comparison's measurements, with the found shares and the score
    /// over every alignment worked out from its stretches. Checked like
    /// [`Measured::new`]: a stored comparison is read from the database.
    fn try_from(comparison: &matching::Comparison) -> Result<Measured, BadMeasurement> {
        let matched_items: usize = comparison.segments.iter().map(|s| s.items).sum();
        let found_score = match matched_items {
            0 => matching::compare::NO_MATCH_SCORE,
            _ => {
                comparison
                    .segments
                    .iter()
                    .map(|s| s.score * s.items as f64)
                    .sum::<f64>()
                    / matched_items as f64
            }
        };
        Measured::new(
            comparison.coverage_a,
            comparison.coverage_b,
            comparison.score,
            comparison.found_a(),
            comparison.found_b(),
            found_score,
            matched_items,
        )
    }
}

/// What is known about the two files' audio.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Audio {
    /// The very same audio: the files share an `audio_hash`, or a `blake3`
    /// (which also covers files with no `audio_hash`).
    Identical,
    /// `matching` compared the two fingerprints, and this is what it
    /// found. A comparison that found nothing is the only way to say "no
    /// audio shared".
    Compared(Measured),
    /// Both files have fingerprints, but nobody compared them: the block
    /// index didn't propose the pair. That is no finding at all (the index
    /// rarely proposes weak matches), so it never counts as "no audio
    /// shared".
    NotCompared,
    /// A file has no fingerprint, so the audio can't be compared.
    Unknown,
}

impl Audio {
    /// The same with the two files swapped.
    pub fn swapped(self) -> Audio {
        match self {
            Audio::Compared(measured) => Audio::Compared(measured.swapped()),
            other => other,
        }
    }
}

/// One name of a file, parsed, with and without its credits.
#[derive(Debug, Clone)]
struct Name {
    /// With the artist tag's artists among its credits.
    credited: ParsedName,
    /// With no credits at all: what the name alone says.
    uncredited: ParsedName,
}

impl Name {
    fn new(parsed: ParsedName, artist_tag: Option<&str>) -> Name {
        let mut credited = parsed.clone();
        if let Some(tag) = artist_tag {
            credited.credits.extend(versions::parse_artists(tag));
        }
        let mut uncredited = parsed;
        uncredited.credits.clear();
        Name {
            credited,
            uncredited,
        }
    }
}

/// What is known about one file's names: its title tag, if it has one, and
/// its file name, each parsed, and its artist tag.
///
/// The file's version labels are those of both names together: each name
/// here carries the labels the other one has and it lacks
/// ([`ParsedName::with_labels_of`]).
#[derive(Debug, Clone)]
pub struct FileFacts {
    tagged: Option<Name>,
    file_name: Name,
    /// Set when the file's own two names disagree in a way the labels
    /// don't settle: the two titles they give, as compared. Two files
    /// with the same doubt don't differ by it.
    own_doubt: Option<String>,
    /// The version words the file's names carry, for the final guard.
    label_words: LabelWords,
}

impl FileFacts {
    /// `title_tag` and `artist_tag` are the file's tags as written (a blank
    /// title tag counts as none); `file_name` is its name on disk. The
    /// artist tag is split into artists as the artist part of a file name
    /// is, and counts for both names.
    pub fn new(title_tag: Option<&str>, artist_tag: Option<&str>, file_name: &str) -> FileFacts {
        let by_file_name = versions::parse_file_name(file_name);
        let title_tag = title_tag.filter(|title| !title.trim().is_empty());
        let by_tag = title_tag.map(versions::parse_title);
        let label_words = label_words(
            title_tag.zip(by_tag.as_ref()),
            (file_name, &by_file_name),
            artist_tag,
        );
        let Some(by_tag) = by_tag else {
            return FileFacts {
                tagged: None,
                file_name: Name::new(by_file_name, artist_tag),
                own_doubt: None,
                label_words,
            };
        };
        // A file's own two names are evidence about that file: with their
        // labels put together, they are set side by side.
        let both = |tag: ParsedName, file_name: ParsedName| {
            let (tag, file_name) = (Name::new(tag, artist_tag), Name::new(file_name, artist_tag));
            let own = versions::compare(&tag.uncredited, &file_name.uncredited).outcome;
            (tag, file_name, own)
        };
        let (mut tagged, mut file_name, mut own) = both(
            by_tag.with_labels_of(&by_file_name),
            by_file_name.with_labels_of(&by_tag),
        );
        // They still differ by a cut: one holds in its title the word the
        // other brackets ("Come Clean" beside "Come Clean (Clean)"). The
        // file carries that cut, so both names get it.
        if own == versions::Outcome::DifferentCut {
            (tagged, file_name, own) = both(
                by_tag.with_every_label_of(&by_file_name),
                by_file_name.with_every_label_of(&by_tag),
            );
        }
        // The same, or unrelated: nothing more to learn. Anything else (a
        // bare "Dirty" at the end of one, a mashup's "x" in one) can't be
        // settled, and the file's version is in doubt. So it is when the
        // parser reads two unrelated titles, but one name holds the
        // other's whole title and a label word besides.
        let (tag, name) = (&tagged.uncredited, &file_name.uncredited);
        let unrelated = own == versions::Outcome::DifferentBase
            && !hides_a_label_word(tag, &by_tag.credits, artist_tag, &name.base_title)
            && !hides_a_label_word(name, &by_file_name.credits, artist_tag, &tag.base_title);
        let own_doubt = match own {
            versions::Outcome::SameVersion => None,
            _ if unrelated => None,
            _ => Some(format!(
                "{} / {}",
                tagged.uncredited.base_title.to_lowercase(),
                file_name.uncredited.base_title.to_lowercase()
            )),
        };
        FileFacts {
            tagged: Some(tagged),
            file_name,
            own_doubt,
            label_words,
        }
    }

    /// The file's first name: its title tag's, else its file name's.
    fn first(&self) -> &Name {
        self.tagged.as_ref().unwrap_or(&self.file_name)
    }
}

/// The words of a name: lowercase, without accents or apostrophes, split
/// at everything that isn't a letter or a digit (spaces, hyphens,
/// underscores, brackets, dots). Dots join letters spelling a label:
/// "V.I.P." is the word `vip`, "X.V.I.P.128" is `x`, `vip` and `128`.
fn words(text: &str) -> Vec<String> {
    let folded: String = text
        .nfd()
        .filter(|c| !is_combining_mark(*c) && *c != '\'')
        .flat_map(char::to_lowercase)
        .collect();
    let mut found = Vec::new();
    for chunk in folded.split(|c: char| !c.is_alphanumeric() && c != '.') {
        let pieces: Vec<&str> = chunk.split('.').filter(|piece| !piece.is_empty()).collect();
        let mut at = 0;
        while at < pieces.len() {
            let single =
                |piece: &str| piece.chars().count() == 1 && piece.chars().all(char::is_alphabetic);
            // A run of single letters: "v", "i", "p".
            let run = pieces[at..]
                .iter()
                .take_while(|piece| single(piece))
                .count();
            if run < 2 {
                found.push(pieces[at].to_string());
                at += 1;
                continue;
            }
            // Within it, the longest stretch that spells a label, from the
            // left; the other letters stay words of their own.
            let end = at + run;
            let mut from = at;
            while from < end {
                let spelled = (from + 2..=end).rev().find_map(|to| {
                    let joined: String = pieces[from..to].concat();
                    versions::label_kind(&joined).map(|_| (to, joined))
                });
                match spelled {
                    Some((to, joined)) => {
                        found.push(joined);
                        from = to;
                    }
                    None => {
                        found.push(pieces[from].to_string());
                        from += 1;
                    }
                }
            }
            at = end;
        }
    }
    found
}

/// Where `run` sits in `words`, as whole words in a row.
fn position_of(words: &[String], run: &[String]) -> Option<usize> {
    if run.is_empty() {
        return None;
    }
    words.windows(run.len()).position(|window| window == run)
}

/// The version words a file's names carry: for each label the parser
/// knows, how often it is written in the one name that writes it most.
type LabelWords = BTreeMap<MarkerKind, usize>;

/// The label words in one name, wherever they are and however they are
/// set off: its words are read from the left, the longest label first
/// ("radio edit" before "radio"), "Original" aside. `set_aside` are the
/// file's artists: each is taken out once, where its whole name stands in
/// this name as words in a row, so "Dub Kestrel" is no dub, and a "Dub"
/// elsewhere in the name still is.
fn label_words_in(name: &str, set_aside: &[Vec<String>]) -> LabelWords {
    let mut rest = words(name);
    for artist in set_aside {
        if let Some(at) = position_of(&rest, artist) {
            rest.drain(at..at + artist.len());
        }
    }
    let mut found = LabelWords::new();
    for kind in labels_among(&rest).0 {
        if kind != MarkerKind::Original {
            *found.entry(kind).or_default() += 1;
        }
    }
    found
}

/// The labels among `words`, read from the left, the longest first, and
/// how many words are no part of any.
fn labels_among(words: &[String]) -> (Vec<MarkerKind>, usize) {
    let (mut kinds, mut others) = (Vec::new(), 0);
    let mut at = 0;
    while at < words.len() {
        let longest = (1..=versions::tables::MAX_LABEL_WORDS.min(words.len() - at))
            .rev()
            .find_map(|n| versions::label_kind(&words[at..at + n].join(" ")).map(|kind| (kind, n)));
        match longest {
            Some((kind, n)) => {
                kinds.push(kind);
                at += n;
            }
            None => {
                others += 1;
                at += 1;
            }
        }
    }
    (kinds, others)
}

/// The version words of a file: those of its title tag (as written, with
/// its parse) and of its file name, each label counted as often as the
/// name that writes it most does. "Come Clean" named "Come Clean
/// (Clean).mp3" is Clean twice; named "nightdrive promo.mp3", once.
///
/// The artist tag's artists are set aside, and so are the artists each
/// name itself gives (a title tag's "feat."; a file name's artist part).
/// A file name's artist part is only trusted as the artist when the
/// file's title tag says so: the tag's title stands in the name's title
/// part and not in its artist part, and the artist part isn't label words
/// and nothing else. "Velo Clean - Halden Rook.mp3" and "Clean - Velo.mp3"
/// tagged "Velo" keep their "Clean"; so does any file name with no title
/// tag, where nothing says which side of the dash the artist is on.
fn label_words(
    tag: Option<(&str, &ParsedName)>,
    file_name: (&str, &ParsedName),
    artist_tag: Option<&str>,
) -> LabelWords {
    let artists = |own: &[versions::Credit]| {
        let mut artists: Vec<Vec<String>> = Vec::new();
        let tagged = artist_tag.map(versions::parse_artists).unwrap_or_default();
        for credit in own.iter().chain(&tagged) {
            let artist = words(&credit.name);
            if !artist.is_empty() && !artists.contains(&artist) {
                artists.push(artist);
            }
        }
        artists
    };
    let (file_name, by_file_name) = file_name;
    // Folders in front of a file name are no part of its name.
    let file_name = file_name.rsplit(['/', '\\']).next().unwrap_or(file_name);
    let artist_part_trusted = tag.is_some_and(|(_, by_tag)| {
        let title = words(&by_tag.base_title);
        let artist_part: Vec<Vec<String>> = by_file_name
            .credits
            .iter()
            .map(|credit| words(&credit.name))
            .collect();
        let only_label_words = artist_part.iter().any(|artist| labels_among(artist).1 == 0);
        position_of(&words(&by_file_name.base_title), &title).is_some()
            && !artist_part
                .iter()
                .any(|artist| position_of(artist, &title).is_some())
            && !only_label_words
    });
    let own: &[versions::Credit] = match artist_part_trusted {
        true => &by_file_name.credits,
        false => &[],
    };
    let mut found = label_words_in(file_name, &artists(own));
    if let Some((title, by_tag)) = tag {
        for (kind, count) in label_words_in(title, &artists(&by_tag.credits)) {
            let most = found.entry(kind).or_default();
            *most = (*most).max(count);
        }
    }
    // Intro and Outro together are Intro-Outro, as within one name.
    let both = (found.get(&MarkerKind::Intro).copied().unwrap_or(0))
        .min(found.get(&MarkerKind::Outro).copied().unwrap_or(0));
    if both > 0 {
        *found.entry(MarkerKind::IntroOutro).or_default() += both;
        for kind in [MarkerKind::Intro, MarkerKind::Outro] {
            match found.get(&kind).copied() {
                Some(count) if count > both => {
                    found.insert(kind, count - both);
                }
                _ => {
                    found.remove(&kind);
                }
            }
        }
    }
    found
}

/// Whether `name`, one of a file's two names, holds the whole title of the
/// file's other name and, among its remaining words, a label word the
/// parser didn't read as a label: "Velo Dirty 128.mp3", "Dirty Velo.mp3"
/// or "Velo (Clean) - Halden Rook.mp3" beside the tag "Velo".
///
/// The parser only reads labels in brackets at the end, after " - ", or as
/// the very last words, so to it these are two unrelated titles. This is
/// the net under that: any position, any separator.
///
/// Not counted as remaining words: the labels the parser did read in
/// `name`, and the file's artist (`artists`: its artist tag's, and the
/// artist part of `name` itself), so an artist called "Dub Kestrel"
/// raises nothing. The artist part of `name` isn't trusted when it is
/// where the title sits: a title-first file name.
fn hides_a_label_word(
    name: &ParsedName,
    own_artists: &[versions::Credit],
    artist_tag: Option<&str>,
    other_title: &str,
) -> bool {
    let title = words(other_title);
    let mut rest = words(&name.original);
    let Some(at) = position_of(&rest, &title) else {
        return false;
    };
    rest.drain(at..at + title.len());

    let own_artists: Vec<Vec<String>> = own_artists.iter().map(|c| words(&c.name)).collect();
    let title_in_artist = own_artists
        .iter()
        .any(|artist| position_of(artist, &title).is_some());
    let mut known: Vec<Vec<String>> = match title_in_artist {
        true => Vec::new(),
        false => own_artists,
    };
    for credit in artist_tag.map(versions::parse_artists).unwrap_or_default() {
        let artist = words(&credit.name);
        // An artist named by both the tag and the name is in it once.
        if !known.contains(&artist) {
            known.push(artist);
        }
    }
    known.extend(name.markers.iter().map(|marker| words(&marker.text)));
    for word in known.iter().flatten() {
        if let Some(at) = rest.iter().position(|left| left == word) {
            rest.remove(at);
        }
    }

    (1..=versions::tables::MAX_LABEL_WORDS).any(|n| {
        rest.windows(n).any(|run| {
            versions::label_kind(&run.join(" ")).is_some_and(|kind| kind != MarkerKind::Original)
        })
    })
}

/// Two files and what is known about them. Built with [`Pair::new`], so a
/// later input (the quality verdict, 1bC) can be added without touching
/// the callers.
#[derive(Debug, Clone, Copy)]
pub struct Pair<'a> {
    audio: Audio,
    a: &'a FileFacts,
    b: &'a FileFacts,
}

impl<'a> Pair<'a> {
    /// `audio` is seen from `a`: its A is `a`, its B is `b`.
    pub fn new(audio: Audio, a: &'a FileFacts, b: &'a FileFacts) -> Pair<'a> {
        Pair { audio, a, b }
    }

    /// The same pair with the two files swapped.
    pub fn swapped(&self) -> Pair<'a> {
        Pair {
            audio: self.audio.swapped(),
            a: self.b,
            b: self.a,
        }
    }
}

fn kind_name<S: Serializer>(kind: &MarkerKind, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(kind.as_str())
}

fn kind_names<S: Serializer>(kinds: &[MarkerKind], serializer: S) -> Result<S::Ok, S::Error> {
    serializer.collect_seq(kinds.iter().map(|kind| kind.as_str()))
}

/// A rework as a name states it: its kind and whose it is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Rework {
    #[serde(serialize_with = "kind_name")]
    pub kind: MarkerKind,
    /// The remixer of a remix, the handle of a bootleg edit. As written.
    pub whose: Option<String>,
}

impl From<&versions::ReworkRef> for Rework {
    fn from(rework: &versions::ReworkRef) -> Rework {
        Rework {
            kind: rework.kind,
            whose: rework.detail.clone(),
        }
    }
}

/// What two files are to each other.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Outcome {
    /// The same track: the files merge.
    Duplicate,
    /// Two cuts of one production: linked, never merged. `labels` are the
    /// cuts the names tell apart (Clean and Dirty; Extended), in
    /// [`MarkerKind`]'s order; empty when only the audio says so.
    CutLink {
        #[serde(serialize_with = "kind_names")]
        labels: Vec<MarkerKind>,
    },
    /// Two productions of one song: linked, never merged. `labels` are the
    /// kinds of rework the names tell apart (remix, flip, mashup…), in
    /// [`MarkerKind`]'s order; never empty.
    ReworkLink {
        #[serde(serialize_with = "kind_names")]
        labels: Vec<MarkerKind>,
    },
    /// Nothing to each other.
    Different,
    /// The signals disagree, or can't settle it: the user decides.
    NeedsReview,
}

/// Why [`decide`] answered as it did: a code ([`Reason::code`]) and its
/// parameters. Never English text.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(
    tag = "code",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum Reason {
    // ---- Deciding reasons: one of these comes first ----
    /// The files hold the very same audio.
    IdenticalAudio,
    /// The audio matches in full, both ways (the duplicate rule).
    SameAudio {
        coverage_a: f32,
        coverage_b: f32,
        score: f64,
    },
    /// The audio matches in full, but the names give two different cuts:
    /// the cuts only one name has.
    VersionVeto {
        #[serde(serialize_with = "kind_names")]
        only_a: Vec<MarkerKind>,
        #[serde(serialize_with = "kind_names")]
        only_b: Vec<MarkerKind>,
    },
    /// The audio matches in full, but the names say it shouldn't, or don't
    /// settle whether these are two versions.
    SameAudioNamesDisagree,
    /// `inner`'s audio sits inside the other file's.
    AudioInsideOther {
        inner: Side,
        found_a: f32,
        found_b: f32,
    },
    /// `inner`'s audio sits inside the other file's, but the names say
    /// these aren't two cuts of one production.
    AudioInsideOtherNamesDisagree {
        inner: Side,
        found_a: f32,
        found_b: f32,
    },
    /// The names are the same, but only part of the audio is shared:
    /// identical tags can hide a different recording.
    SameTagsPartialAudio { found_a: f32, found_b: f32 },
    /// Part of the audio is shared, and the names don't settle what the
    /// files are to each other.
    PartialAudioNamesUnclear { found_a: f32, found_b: f32 },
    /// The names give two different cuts: the cuts only one name has.
    NamesSayCut {
        #[serde(serialize_with = "kind_names")]
        only_a: Vec<MarkerKind>,
        #[serde(serialize_with = "kind_names")]
        only_b: Vec<MarkerKind>,
    },
    /// The names give two different reworks: the ones only one name has.
    NamesSayRework {
        only_a: Vec<Rework>,
        only_b: Vec<Rework>,
    },
    /// `mashup` names the other file's title as one of its parts, and the
    /// two share audio.
    MashupContains { mashup: Side, part: String },
    /// The audio matches in full and the table would merge, but the two
    /// files' names don't carry the same version words (the final guard):
    /// the labels one file's names carry more of than the other's.
    LabelWordsDiffer {
        #[serde(serialize_with = "kind_names")]
        only_a: Vec<MarkerKind>,
        #[serde(serialize_with = "kind_names")]
        only_b: Vec<MarkerKind>,
    },
    /// The names are the same, but the fingerprints share nothing.
    FingerprintVeto,
    /// The names are the same, and a file has no fingerprint to check
    /// them against.
    NoFingerprint,
    /// The names are the same, and the two fingerprints were never
    /// compared.
    NotCompared,
    /// The same title under different artists, with no audio shared (or
    /// none to compare): the same title can be a different song.
    SameTitleDifferentArtists,
    /// `mashup` names the other file's title as one of its parts. Only a
    /// hint: part names match unrelated tracks too.
    MashupPart { mashup: Side, part: String },
    /// The names don't settle what the files are to each other. `doubts`
    /// are the codes of the [`crate::versions::Reason`]s the names gave,
    /// in camelCase like every code here, sorted; or
    /// [`OWN_NAMES_DISAGREE`] alone.
    NamesUnclear { doubts: Vec<String> },
    /// The titles differ.
    TitlesDiffer,

    // ---- Only ever beside a deciding reason ----
    /// A duplicate whose names don't agree.
    NamesDiffer,
    /// The files credit different artists.
    ArtistsDiffer,
    /// The artists differ, but the artist of each of `sides` is the remixer
    /// the names give: a remix filed under the remixer alone. A first.
    CreditedToRemixer { sides: Vec<Side> },
}

impl Reason {
    /// The reason's code: what it serializes as, and what an i18n key is
    /// built from.
    pub fn code(&self) -> &'static str {
        match self {
            Reason::IdenticalAudio => "identicalAudio",
            Reason::SameAudio { .. } => "sameAudio",
            Reason::VersionVeto { .. } => "versionVeto",
            Reason::SameAudioNamesDisagree => "sameAudioNamesDisagree",
            Reason::AudioInsideOther { .. } => "audioInsideOther",
            Reason::AudioInsideOtherNamesDisagree { .. } => "audioInsideOtherNamesDisagree",
            Reason::SameTagsPartialAudio { .. } => "sameTagsPartialAudio",
            Reason::PartialAudioNamesUnclear { .. } => "partialAudioNamesUnclear",
            Reason::NamesSayCut { .. } => "namesSayCut",
            Reason::NamesSayRework { .. } => "namesSayRework",
            Reason::MashupContains { .. } => "mashupContains",
            Reason::LabelWordsDiffer { .. } => "labelWordsDiffer",
            Reason::FingerprintVeto => "fingerprintVeto",
            Reason::NoFingerprint => "noFingerprint",
            Reason::NotCompared => "notCompared",
            Reason::SameTitleDifferentArtists => "sameTitleDifferentArtists",
            Reason::MashupPart { .. } => "mashupPart",
            Reason::NamesUnclear { .. } => "namesUnclear",
            Reason::TitlesDiffer => "titlesDiffer",
            Reason::NamesDiffer => "namesDiffer",
            Reason::ArtistsDiffer => "artistsDiffer",
            Reason::CreditedToRemixer { .. } => "creditedToRemixer",
        }
    }

    /// The same reason with the two files swapped.
    pub fn swapped(self) -> Reason {
        match self {
            Reason::SameAudio {
                coverage_a,
                coverage_b,
                score,
            } => Reason::SameAudio {
                coverage_a: coverage_b,
                coverage_b: coverage_a,
                score,
            },
            Reason::VersionVeto { only_a, only_b } => Reason::VersionVeto {
                only_a: only_b,
                only_b: only_a,
            },
            Reason::AudioInsideOther {
                inner,
                found_a,
                found_b,
            } => Reason::AudioInsideOther {
                inner: inner.other(),
                found_a: found_b,
                found_b: found_a,
            },
            Reason::AudioInsideOtherNamesDisagree {
                inner,
                found_a,
                found_b,
            } => Reason::AudioInsideOtherNamesDisagree {
                inner: inner.other(),
                found_a: found_b,
                found_b: found_a,
            },
            Reason::SameTagsPartialAudio { found_a, found_b } => Reason::SameTagsPartialAudio {
                found_a: found_b,
                found_b: found_a,
            },
            Reason::PartialAudioNamesUnclear { found_a, found_b } => {
                Reason::PartialAudioNamesUnclear {
                    found_a: found_b,
                    found_b: found_a,
                }
            }
            Reason::NamesSayCut { only_a, only_b } => Reason::NamesSayCut {
                only_a: only_b,
                only_b: only_a,
            },
            Reason::LabelWordsDiffer { only_a, only_b } => Reason::LabelWordsDiffer {
                only_a: only_b,
                only_b: only_a,
            },
            Reason::NamesSayRework { only_a, only_b } => Reason::NamesSayRework {
                only_a: only_b,
                only_b: only_a,
            },
            Reason::MashupContains { mashup, part } => Reason::MashupContains {
                mashup: mashup.other(),
                part,
            },
            Reason::MashupPart { mashup, part } => Reason::MashupPart {
                mashup: mashup.other(),
                part,
            },
            Reason::CreditedToRemixer { sides } => {
                let mut sides: Vec<Side> = sides.into_iter().map(Side::other).collect();
                sides.sort_unstable();
                Reason::CreditedToRemixer { sides }
            }
            same @ (Reason::IdenticalAudio
            | Reason::SameAudioNamesDisagree
            | Reason::FingerprintVeto
            | Reason::NoFingerprint
            | Reason::NotCompared
            | Reason::SameTitleDifferentArtists
            | Reason::NamesUnclear { .. }
            | Reason::TitlesDiffer
            | Reason::NamesDiffer
            | Reason::ArtistsDiffer) => same,
        }
    }
}

/// [`decide`]'s answer. `reasons` is never empty, and its first is the
/// deciding one.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Decision {
    pub outcome: Outcome,
    pub reasons: Vec<Reason>,
}

impl Decision {
    /// The same decision with the two files swapped.
    pub fn swapped(self) -> Decision {
        Decision {
            outcome: self.outcome,
            reasons: self.reasons.into_iter().map(Reason::swapped).collect(),
        }
    }
}

/// The audio, sorted into the table's cases.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Shared {
    Identical,
    Full(Measured),
    Inside(Side, Measured),
    Partial(Measured),
    Nothing,
    NotCompared,
    Unknown,
}

fn shared(audio: Audio) -> Shared {
    let measured = match audio {
        Audio::Identical => return Shared::Identical,
        Audio::NotCompared => return Shared::NotCompared,
        Audio::Unknown => return Shared::Unknown,
        Audio::Compared(measured) => measured,
    };
    let full = measured.coverage_a >= DUPLICATE_COVERAGE
        && measured.coverage_b >= DUPLICATE_COVERAGE
        && measured.score <= DUPLICATE_SCORE;
    if full {
        return Shared::Full(measured);
    }
    let found_a = measured.found_a >= DUPLICATE_COVERAGE;
    let found_b = measured.found_b >= DUPLICATE_COVERAGE;
    if found_a != found_b && measured.found_score <= DUPLICATE_SCORE {
        let inner = match found_a {
            true => Side::A,
            false => Side::B,
        };
        return Shared::Inside(inner, measured);
    }
    if measured.matched_items >= MIN_SHARED_ITEMS {
        return Shared::Partial(measured);
    }
    Shared::Nothing
}

/// How the two names relate, artists aside.
#[derive(Debug, Clone, PartialEq)]
enum Relation {
    Same,
    Cut {
        only_a: Vec<MarkerKind>,
        only_b: Vec<MarkerKind>,
    },
    Rework {
        only_a: Vec<Rework>,
        only_b: Vec<Rework>,
    },
    OtherTitle,
    MashupPart {
        mashup: Side,
        part: String,
    },
    Unclear {
        doubts: Vec<String>,
    },
}

/// What the credits say about two files with the same title.
#[derive(Debug, Clone, PartialEq)]
enum Credits {
    /// The artists agree, or a file names none.
    Agree,
    /// The artists differ, but these files are credited to the remixer.
    Remixer(Vec<Side>),
    Differ,
}

/// A snake_case code in camelCase.
fn camel(code: &str) -> String {
    let mut out = String::with_capacity(code.len());
    let mut upper = false;
    for c in code.chars() {
        match c {
            '_' => upper = true,
            c if upper => {
                out.extend(c.to_uppercase());
                upper = false;
            }
            c => out.push(c),
        }
    }
    out
}

fn relation_of(names: &versions::Comparison) -> Relation {
    use versions::Reason as Said;
    match names.outcome {
        versions::Outcome::SameVersion => return Relation::Same,
        versions::Outcome::DifferentBase => return Relation::OtherTitle,
        versions::Outcome::DifferentCut => {
            for reason in &names.reasons {
                if let Said::CutDiffers { only_a, only_b } = reason {
                    return Relation::Cut {
                        only_a: only_a.clone(),
                        only_b: only_b.clone(),
                    };
                }
            }
        }
        versions::Outcome::DifferentRework => {
            for reason in &names.reasons {
                if let Said::ReworkDiffers { only_a, only_b } = reason {
                    return Relation::Rework {
                        only_a: only_a.iter().map(Rework::from).collect(),
                        only_b: only_b.iter().map(Rework::from).collect(),
                    };
                }
            }
        }
        versions::Outcome::CantTell => {
            // A mashup naming the other title as a part. Two mashups that
            // share a part aren't each other's source: that stays unclear.
            let parts: Vec<(Side, &String)> = names
                .reasons
                .iter()
                .filter_map(|reason| match reason {
                    Said::MashupPartMatch { side, part } => Some((Side::from(*side), part)),
                    _ => None,
                })
                .collect();
            if let Some(&(mashup, part)) = parts.first() {
                if parts.iter().all(|(side, _)| *side == mashup) {
                    return Relation::MashupPart {
                        mashup,
                        part: part.clone(),
                    };
                }
            }
        }
    }
    let mut doubts: Vec<String> = Vec::new();
    for reason in &names.reasons {
        let code = camel(reason.code());
        if !matches!(reason, Said::BaseTitlesMatch) && !doubts.contains(&code) {
            doubts.push(code);
        }
    }
    // Sorted, so the answer is the same whichever file comes first.
    doubts.sort_unstable();
    Relation::Unclear { doubts }
}

/// What one reading of the pair's names says.
#[derive(Debug, Clone, PartialEq)]
struct Read {
    relation: Relation,
    credits: Credits,
}

fn read(a: &Name, b: &Name) -> Read {
    use versions::Reason as Said;
    let credited = versions::compare(&a.credited, &b.credited);
    let mut remixers: Vec<Side> = credited
        .reasons
        .iter()
        .filter_map(|reason| match reason {
            Said::CreditedToRemixer { side } => Some(Side::from(*side)),
            _ => None,
        })
        .collect();
    remixers.sort_unstable();
    let (relation, credits) = if credited.reasons.contains(&Said::ArtistsDiffer) {
        // The artists make the names "can't tell"; what the names say
        // without them is the relation.
        let plain = versions::compare(&a.uncredited, &b.uncredited);
        (relation_of(&plain), Credits::Differ)
    } else if remixers.is_empty() {
        (relation_of(&credited), Credits::Agree)
    } else {
        (relation_of(&credited), Credits::Remixer(remixers))
    };
    Read { relation, credits }
}

/// Every reading of the pair's titles, the tags' first (a file with no
/// title tag stands in with its file name). Two files with no title tag
/// have one reading, their file names.
fn readings(a: &FileFacts, b: &FileFacts) -> Vec<Read> {
    let by_file_name = read(&a.file_name, &b.file_name);
    if a.tagged.is_none() && b.tagged.is_none() {
        return vec![by_file_name];
    }
    vec![read(a.first(), b.first()), by_file_name]
}

/// Where a relation stands among the readings: the lowest wins, and of two
/// readings equally low, the tags'.
///
/// Both readings carry the files' whole labels, so a cut or a rework shows
/// on any reading that puts the two titles together, and it comes first
/// either way. What is left to rank is what the titles say.
///
/// With the audio matched in full, or one file inside the other, the audio
/// says "one production", so the reading that would stop a merge comes
/// first: a mashup part or names that can't be told before "same".
///
/// Otherwise the names decide, and the reading under which they are the
/// same comes before one that says less.
///
/// "Another title" is always last: it never hides what the other reading
/// says.
fn rank(audio_led: bool, relation: &Relation) -> u8 {
    match (audio_led, relation) {
        (_, Relation::Cut { .. } | Relation::Rework { .. }) => 0,
        (true, Relation::MashupPart { .. }) => 1,
        (true, Relation::Unclear { .. }) => 2,
        (true, Relation::Same) => 3,
        (false, Relation::Same) => 1,
        (false, Relation::MashupPart { .. }) => 2,
        (false, Relation::Unclear { .. }) => 3,
        (_, Relation::OtherTitle) => 4,
    }
}

/// The labels of a link: the kinds either name has alone, in
/// [`MarkerKind`]'s order.
fn labels(kinds: impl Iterator<Item = MarkerKind>) -> Vec<MarkerKind> {
    let mut labels: Vec<MarkerKind> = kinds.collect();
    labels.sort();
    labels.dedup();
    labels
}

fn cut_labels(only_a: &[MarkerKind], only_b: &[MarkerKind]) -> Vec<MarkerKind> {
    labels(only_a.iter().chain(only_b).copied())
}

fn rework_labels(only_a: &[Rework], only_b: &[Rework]) -> Vec<MarkerKind> {
    labels(only_a.iter().chain(only_b).map(|rework| rework.kind))
}

/// What the names said, as a reason to show beside the deciding one.
fn said(relation: Relation) -> Option<Reason> {
    match relation {
        Relation::Same => None,
        Relation::Cut { only_a, only_b } => Some(Reason::NamesSayCut { only_a, only_b }),
        Relation::Rework { only_a, only_b } => Some(Reason::NamesSayRework { only_a, only_b }),
        Relation::OtherTitle => Some(Reason::TitlesDiffer),
        Relation::MashupPart { mashup, part } => Some(Reason::MashupPart { mashup, part }),
        Relation::Unclear { doubts } => Some(Reason::NamesUnclear { doubts }),
    }
}

/// What the credits said, as reasons to show beside the deciding one.
fn credit_notes(credits: &Credits) -> Vec<Reason> {
    match credits {
        Credits::Agree => Vec::new(),
        Credits::Differ => vec![Reason::ArtistsDiffer],
        Credits::Remixer(sides) => vec![Reason::CreditedToRemixer {
            sides: sides.clone(),
        }],
    }
}

/// Decides what the pair's two files are to each other. The table is in
/// the [module docs](self). The answer doesn't depend on which file comes
/// first: swapping them swaps the sides in the reasons, nothing else.
pub fn decide(pair: &Pair<'_>) -> Decision {
    let decision = by_table(pair);
    // The final guard: on a full (not identical) audio match, two files
    // merge silently only if their names carry the same version words.
    let (mine, theirs) = (&pair.a.label_words, &pair.b.label_words);
    let merges_on_full_audio =
        decision.outcome == Outcome::Duplicate && matches!(shared(pair.audio), Shared::Full(_));
    if !merges_on_full_audio || mine == theirs {
        return decision;
    }
    let more = |mine: &LabelWords, theirs: &LabelWords| -> Vec<MarkerKind> {
        mine.iter()
            .filter(|(kind, count)| **count > theirs.get(kind).copied().unwrap_or(0))
            .map(|(kind, _)| *kind)
            .collect()
    };
    let mut reasons = vec![Reason::LabelWordsDiffer {
        only_a: more(mine, theirs),
        only_b: more(theirs, mine),
    }];
    reasons.extend(decision.reasons);
    Decision {
        outcome: Outcome::NeedsReview,
        reasons,
    }
}

/// What the table says, before the final guard.
fn by_table(pair: &Pair<'_>) -> Decision {
    let shared = shared(pair.audio);
    let audio_led = matches!(
        shared,
        Shared::Identical | Shared::Full(_) | Shared::Inside(..)
    );
    // `min_by_key` keeps the first of equals: the tags' reading.
    let Read {
        mut relation,
        credits,
    } = readings(pair.a, pair.b)
        .into_iter()
        .min_by_key(|read| rank(audio_led, &read.relation))
        .expect("a pair always has a reading");
    // Under two titles no reading set the labels side by side. With the
    // audio matched they count whatever the titles: "(Clean)" beside
    // "(Dirty)" is a cut under any two names.
    let by_labels = || {
        let labels =
            versions::compare_labels(&pair.a.first().uncredited, &pair.b.first().uncredited);
        relation_of(&labels)
    };
    // One file's own names leave its version in doubt, and the other's
    // don't, or not the same way: with the audio matched, "the same" or
    // "the same labels" can't be relied on.
    let own_names_disagree = audio_led && pair.a.own_doubt != pair.b.own_doubt;
    let doubted = |said: Relation| match said {
        Relation::Same if own_names_disagree => Relation::Unclear {
            doubts: vec![OWN_NAMES_DISAGREE.to_string()],
        },
        said => said,
    };
    relation = doubted(relation);
    let by_labels = || doubted(by_labels());
    let mut titles_differ = false;
    if relation == Relation::OtherTitle {
        let labels_say = match shared {
            Shared::Full(_) => Some(by_labels()),
            // One file inside the other under another title already goes
            // to review; only a cut or a rework says more than that.
            Shared::Inside(..) => Some(by_labels())
                .filter(|said| matches!(said, Relation::Cut { .. } | Relation::Rework { .. })),
            _ => None,
        };
        if let Some(said) = labels_say.filter(|said| *said != Relation::Same) {
            relation = said;
            titles_differ = true;
        }
    }
    // The credits only say something under one title: names with another
    // title are never compared by artist.
    let artists_differ = credits == Credits::Differ;
    let notes = credit_notes(&credits);

    let (outcome, mut reasons) = match shared {
        // The very same audio can't be two versions, whatever the names
        // say: one of the files is mislabeled.
        Shared::Identical => {
            let mut reasons = vec![Reason::IdenticalAudio];
            if relation != Relation::Same {
                reasons.push(Reason::NamesDiffer);
            }
            (Outcome::Duplicate, reasons)
        }
        Shared::Full(m) => {
            let same_audio = Reason::SameAudio {
                coverage_a: m.coverage_a,
                coverage_b: m.coverage_b,
                score: m.score,
            };
            match relation {
                Relation::Same => (Outcome::Duplicate, vec![same_audio]),
                Relation::OtherTitle => (Outcome::Duplicate, vec![same_audio, Reason::NamesDiffer]),
                // The version check's veto: Clean and Dirty fingerprint
                // identically.
                Relation::Cut { only_a, only_b } => (
                    Outcome::CutLink {
                        labels: cut_labels(&only_a, &only_b),
                    },
                    vec![Reason::VersionVeto { only_a, only_b }],
                ),
                unsettled => {
                    let mut reasons = vec![Reason::SameAudioNamesDisagree];
                    reasons.extend(said(unsettled));
                    (Outcome::NeedsReview, reasons)
                }
            }
        }
        // One-sided coverage is a cut: linked, never merged.
        Shared::Inside(inner, m) => {
            let inside = Reason::AudioInsideOther {
                inner,
                found_a: m.found_a,
                found_b: m.found_b,
            };
            let disagree = Reason::AudioInsideOtherNamesDisagree {
                inner,
                found_a: m.found_a,
                found_b: m.found_b,
            };
            match relation {
                Relation::Same => (Outcome::CutLink { labels: Vec::new() }, vec![inside]),
                Relation::Cut { only_a, only_b } => (
                    Outcome::CutLink {
                        labels: cut_labels(&only_a, &only_b),
                    },
                    vec![inside, Reason::NamesSayCut { only_a, only_b }],
                ),
                Relation::Unclear { doubts } => (
                    Outcome::CutLink { labels: Vec::new() },
                    vec![inside, Reason::NamesUnclear { doubts }],
                ),
                Relation::MashupPart { mashup, part } => (
                    Outcome::ReworkLink {
                        labels: vec![MarkerKind::Mashup],
                    },
                    vec![Reason::MashupContains { mashup, part }],
                ),
                against @ (Relation::Rework { .. } | Relation::OtherTitle) => {
                    let mut reasons = vec![disagree];
                    reasons.extend(said(against));
                    (Outcome::NeedsReview, reasons)
                }
            }
        }
        Shared::Partial(m) => {
            let unclear = Reason::PartialAudioNamesUnclear {
                found_a: m.found_a,
                found_b: m.found_b,
            };
            match relation {
                Relation::OtherTitle => (Outcome::Different, vec![Reason::TitlesDiffer]),
                Relation::MashupPart { mashup, part } => (
                    Outcome::ReworkLink {
                        labels: vec![MarkerKind::Mashup],
                    },
                    vec![Reason::MashupContains { mashup, part }],
                ),
                Relation::Unclear { doubts } => (
                    Outcome::NeedsReview,
                    vec![unclear, Reason::NamesUnclear { doubts }],
                ),
                // Shared audio says "related", the artists say "another
                // song": the signals disagree.
                related if artists_differ => {
                    let mut reasons = vec![unclear];
                    reasons.extend(said(related));
                    (Outcome::NeedsReview, reasons)
                }
                // Identical tags can hide a different recording.
                Relation::Same => (
                    Outcome::NeedsReview,
                    vec![Reason::SameTagsPartialAudio {
                        found_a: m.found_a,
                        found_b: m.found_b,
                    }],
                ),
                Relation::Cut { only_a, only_b } => (
                    Outcome::CutLink {
                        labels: cut_labels(&only_a, &only_b),
                    },
                    vec![Reason::NamesSayCut { only_a, only_b }],
                ),
                Relation::Rework { only_a, only_b } => (
                    Outcome::ReworkLink {
                        labels: rework_labels(&only_a, &only_b),
                    },
                    vec![Reason::NamesSayRework { only_a, only_b }],
                ),
            }
        }
        // No audio to go on: the names decide whether it's a link, and
        // never make a duplicate.
        nothing @ (Shared::Nothing | Shared::NotCompared | Shared::Unknown) => match relation {
            Relation::OtherTitle => (Outcome::Different, vec![Reason::TitlesDiffer]),
            Relation::MashupPart { mashup, part } => (
                Outcome::Different,
                vec![Reason::MashupPart { mashup, part }],
            ),
            Relation::Unclear { doubts } => {
                (Outcome::Different, vec![Reason::NamesUnclear { doubts }])
            }
            // The same title can be a different song, even as remixes.
            _ if artists_differ => (Outcome::Different, vec![Reason::SameTitleDifferentArtists]),
            Relation::Same => match nothing {
                Shared::Unknown => (Outcome::NeedsReview, vec![Reason::NoFingerprint]),
                // Not compared is no finding: never "different".
                Shared::NotCompared => (Outcome::NeedsReview, vec![Reason::NotCompared]),
                // The fingerprint's veto on a name-based merge.
                Shared::Nothing => (Outcome::Different, vec![Reason::FingerprintVeto]),
                Shared::Identical | Shared::Full(_) | Shared::Inside(..) | Shared::Partial(_) => {
                    unreachable!("this arm only takes the cases with no audio to go on")
                }
            },
            Relation::Cut { only_a, only_b } => (
                Outcome::CutLink {
                    labels: cut_labels(&only_a, &only_b),
                },
                vec![Reason::NamesSayCut { only_a, only_b }],
            ),
            Relation::Rework { only_a, only_b } => (
                Outcome::ReworkLink {
                    labels: rework_labels(&only_a, &only_b),
                },
                vec![Reason::NamesSayRework { only_a, only_b }],
            ),
        },
    };

    // "Different under other artists" already says what the credits said.
    if reasons[0] != Reason::SameTitleDifferentArtists {
        reasons.extend(notes);
    }
    if titles_differ {
        reasons.push(Reason::TitlesDiffer);
    }
    Decision { outcome, reasons }
}
