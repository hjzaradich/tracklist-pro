//! Reads a track's name (a title tag or a file name) and describes it:
//! the base title, the artist credits, the version markers (ROADMAP §1.5:
//! cuts and reworks), and whatever store or rip junk was around them.
//!
//! This is a pure library: no database, no files, no jobs, no IPC. It
//! **describes names and decides nothing about merging**. Grouping (1bB)
//! combines what it says with the fingerprint and the credits.
//!
//! - [`parse`] turns one name into a [`ParsedName`]. The original string is
//!   always kept, and everything taken out as junk is listed, never dropped
//!   silently.
//! - [`compare`] sets two parsed names side by side and answers with an
//!   [`Outcome`] and its [`Reason`]s, which are codes and parameters, never
//!   English text.
//!
//! # A version word can be the real title
//!
//! A title can really end in "(dirty)", and "Come Clean" isn't the clean
//! cut of "Come". One name alone can't settle that, so the parser never
//! strips blindly:
//!
//! - Words without brackets ("Paper Harbor Extended Mix") stay in
//!   [`ParsedName::base_title`] and are listed in
//!   [`ParsedName::title_markers`]: they *may* be markers.
//! - A bracketed or " - " label is read as a marker, the convention, and
//!   listed in [`ParsedName::markers`]; [`Marker::ambiguous`] is set where
//!   the form itself is doubtful (a " - " segment, a mashup `x`).
//! - A label that appears twice ("Velo (dirty) (dirty)") keeps its first
//!   copy in the title.
//!
//! [`ParsedName::readings`] lists every way the name can be split, and
//! [`compare`] picks the one the *other* name supports. Where neither name
//! settles it, the answer is [`Outcome::CantTell`].
//! A label after " - " is trusted as a marker when the other name has the
//! plain title, as a bracketed one is; only bare words are doubted there.
//!
//! # Extending
//!
//! Label spellings and junk patterns live in [`tables`], in one place. A
//! convention a user reports is a new row there plus a row in the test
//! corpus (`tests/corpus.rs`).

mod compare;
mod junk;
mod labels;
mod parse;
pub mod tables;
pub mod tokenize;

#[cfg(test)]
mod tests;

pub use compare::{compare, Comparison, Outcome, Reason, ReworkRef, Side, MAX_LABEL_GROUPS};

/// Where a name came from. A file name can hold an artist, a track number
/// and an extension; a title tag is taken as a title.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NameSource {
    /// A title tag.
    Title,
    /// A file name. Folders in front of it are set aside (and reported).
    FileName,
}

/// The two kinds of version (ROADMAP §1.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum VersionClass {
    /// Same production, different length or lyrics.
    Cut,
    /// A different production.
    Rework,
}

/// What a version marker says. Exactly the kinds of ROADMAP §1.5; a new
/// kind is the owner's decision, never the parser's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum MarkerKind {
    // Cuts
    Original,
    Extended,
    RadioEdit,
    ClubEdit,
    Short,
    Clean,
    Dirty,
    Intro,
    Outro,
    IntroOutro,
    QuickHit,
    /// A plain "(Edit)". A named edit is a bootleg.
    Edit,
    Remaster,
    // Reworks
    Remix,
    Vip,
    Flip,
    Bootleg,
    /// Rework, refix or reboot.
    Rework,
    Dub,
    Cover,
    Live,
    Alternate,
    Instrumental,
    Acapella,
    Mashup,
}

impl MarkerKind {
    /// Every kind, cuts first.
    pub const ALL: [MarkerKind; 25] = [
        MarkerKind::Original,
        MarkerKind::Extended,
        MarkerKind::RadioEdit,
        MarkerKind::ClubEdit,
        MarkerKind::Short,
        MarkerKind::Clean,
        MarkerKind::Dirty,
        MarkerKind::Intro,
        MarkerKind::Outro,
        MarkerKind::IntroOutro,
        MarkerKind::QuickHit,
        MarkerKind::Edit,
        MarkerKind::Remaster,
        MarkerKind::Remix,
        MarkerKind::Vip,
        MarkerKind::Flip,
        MarkerKind::Bootleg,
        MarkerKind::Rework,
        MarkerKind::Dub,
        MarkerKind::Cover,
        MarkerKind::Live,
        MarkerKind::Alternate,
        MarkerKind::Instrumental,
        MarkerKind::Acapella,
        MarkerKind::Mashup,
    ];

    /// Whether this kind is a cut or a rework: a cut if its spellings are
    /// in [`tables::CUT_LABELS`], a rework otherwise. The tables are the
    /// one place that says so.
    pub fn class(self) -> VersionClass {
        match tables::CUT_LABELS.iter().any(|(kind, _)| *kind == self) {
            true => VersionClass::Cut,
            false => VersionClass::Rework,
        }
    }

    /// A stable name, for storage and for building i18n keys.
    pub fn as_str(self) -> &'static str {
        match self {
            MarkerKind::Original => "original",
            MarkerKind::Extended => "extended",
            MarkerKind::RadioEdit => "radio_edit",
            MarkerKind::ClubEdit => "club_edit",
            MarkerKind::Short => "short",
            MarkerKind::Clean => "clean",
            MarkerKind::Dirty => "dirty",
            MarkerKind::Intro => "intro",
            MarkerKind::Outro => "outro",
            MarkerKind::IntroOutro => "intro_outro",
            MarkerKind::QuickHit => "quick_hit",
            MarkerKind::Edit => "edit",
            MarkerKind::Remaster => "remaster",
            MarkerKind::Remix => "remix",
            MarkerKind::Vip => "vip",
            MarkerKind::Flip => "flip",
            MarkerKind::Bootleg => "bootleg",
            MarkerKind::Rework => "rework",
            MarkerKind::Dub => "dub",
            MarkerKind::Cover => "cover",
            MarkerKind::Live => "live",
            MarkerKind::Alternate => "alternate",
            MarkerKind::Instrumental => "instrumental",
            MarkerKind::Acapella => "acapella",
            MarkerKind::Mashup => "mashup",
        }
    }
}

/// How a marker was written in the name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MarkerForm {
    /// In brackets of any kind: "(Extended Mix)", "[Vey Sun Flip]".
    Bracketed,
    /// After a " - ": "Paper Harbor - Extended Mix".
    DashSegment,
    /// Plain words at the end of the title: "Paper Harbor Extended Mix".
    Bare,
    /// A mashup's "x" or "vs" between two titles.
    Separator,
}

/// One version marker found in a name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Marker {
    pub kind: MarkerKind,
    /// Who or what the marker names: the remixer of a remix, the place of a
    /// live recording, the handle of a bootleg edit. As written.
    pub detail: Option<String>,
    /// The marker as written in the name, brackets included.
    pub text: String,
    pub form: MarkerForm,
    /// True when this might be part of the title instead of a marker.
    pub ambiguous: bool,
}

/// An artist's part in a credit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CreditRole {
    /// Named in front of the title ("Artist - Title").
    Main,
    /// Named after "feat." / "ft." / "featuring".
    Featured,
}

/// One artist named in the name. A remixer isn't listed here: it's the
/// `detail` of its marker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Credit {
    pub name: String,
    pub role: CreditRole,
}

/// What kind of junk a piece of text was.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum JunkKind {
    /// An HTML entity, decoded: `&#40;` is "(".
    HtmlEntity,
    /// `%20` standing in for a space, in a file name.
    UrlEscape,
    /// A store's numeric ID in front of the name: `10000001_`.
    StoreId,
    /// A site's name: `RipSite.example - `, `[www.ripsite.example]`.
    SiteName,
    /// A rip or upload tag: `[ example-ripper ]`, `(320)`, `(Free Download)`.
    RipTag,
    /// A track number in front of the name: `01 `, `1-03 `, `A1 - `.
    TrackNumber,
    /// Underscores standing in for spaces.
    Underscores,
    /// A file name's audio extension.
    FileExtension,
    /// Folders in front of a file name.
    FolderPath,
    /// A running time in brackets: `(3:45)`. Length decides nothing.
    Duration,
}

/// A piece of the name set aside as junk, as it was written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Junk {
    pub kind: JunkKind,
    pub text: String,
}

/// A name, described. See the module docs for how doubt is shown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedName {
    /// The string exactly as it was given.
    pub original: String,
    pub source: NameSource,
    /// The title without its version markers, junk and credits. Characters
    /// are normalized (NFC, plain quotes and brackets); nothing is
    /// lowercased.
    pub base_title: String,
    pub credits: Vec<Credit>,
    /// The version markers, in the order written.
    pub markers: Vec<Marker>,
    /// Label words left in `base_title` because they may be the real title.
    pub title_markers: Vec<Marker>,
    /// For a mashup ("A x B"), the titles it names. Each is only a
    /// *possible* source title.
    pub mashup_parts: Vec<String>,
    /// Trailing brackets this module doesn't understand, as written. They
    /// are left out of `base_title`; two names that differ in one compare
    /// as "can't tell".
    pub unrecognized: Vec<String>,
    /// Everything set aside as junk.
    pub junk: Vec<Junk>,
    /// The title with every qualifier taken off.
    core: String,
    /// Label groups in the order written; the first `folded` of them are
    /// part of `base_title`.
    qualifiers: Vec<Qualifier>,
    folded: usize,
    /// The mashup marker a bare "x" or "vs" stands for.
    separator: Option<Marker>,
}

/// One bracket, " - " segment or run of bare words holding labels.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Qualifier {
    as_written: String,
    markers: Vec<Marker>,
}

/// One way to split a name into a title and its markers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reading {
    pub base_title: String,
    pub markers: Vec<Marker>,
    /// Text that [`ParsedName::base_title`] keeps in the title, read here
    /// as a marker.
    pub promoted: Vec<String>,
    /// Text that [`ParsedName::markers`] holds as a marker, read here as
    /// part of the title.
    pub demoted: Vec<String>,
}

impl Reading {
    /// How far this reading is from the name's own.
    pub fn distance(&self) -> usize {
        self.promoted.len() + self.demoted.len()
    }
}

impl ParsedName {
    /// Every way to read the name, the parser's own reading first. In each,
    /// the labels up to some point belong to the title and the rest are
    /// markers.
    pub fn readings(&self) -> Vec<Reading> {
        let mut readings = Vec::with_capacity(self.qualifiers.len() + 1);
        for split in 0..=self.qualifiers.len() {
            let base_title = self.base_with(split);
            if base_title.is_empty() && split != self.folded {
                continue;
            }
            let texts = |range: std::ops::Range<usize>| -> Vec<String> {
                self.qualifiers[range]
                    .iter()
                    .map(|q| q.as_written.clone())
                    .collect()
            };
            let (promoted, demoted) = if split < self.folded {
                (texts(split..self.folded), Vec::new())
            } else {
                (Vec::new(), texts(self.folded..split))
            };
            let mut markers: Vec<Marker> = self.qualifiers[split..]
                .iter()
                .flat_map(|q| q.markers.iter().cloned())
                .collect();
            markers.extend(self.separator.iter().cloned());
            readings.push(Reading {
                base_title,
                markers,
                promoted,
                demoted,
            });
        }
        readings.sort_by_key(Reading::distance);
        readings
    }

    /// The core title plus the first `split` qualifiers as written.
    fn base_with(&self, split: usize) -> String {
        let mut base = self.core.clone();
        for qualifier in &self.qualifiers[..split] {
            if !base.is_empty() {
                base.push(' ');
            }
            base.push_str(&qualifier.as_written);
        }
        base
    }
}

/// Describes one name. Never panics, whatever the input.
pub fn parse(input: &str, source: NameSource) -> ParsedName {
    parse::parse(input, source)
}

/// [`parse`] for a title tag.
pub fn parse_title(input: &str) -> ParsedName {
    parse(input, NameSource::Title)
}

/// [`parse`] for a file name.
pub fn parse_file_name(input: &str) -> ParsedName {
    parse(input, NameSource::FileName)
}
