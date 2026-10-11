//! The deciding run: asks the grouping rules ([`super::rules::decide`])
//! about pairs of files and stores what they say (`grouping_pair`). It
//! decides nothing about tracks; [`super::store::regroup`] reads the pairs
//! and does that.
//!
//! # Which files
//!
//! A file is **due** when its record (`grouping_file.decided_as`) isn't the
//! current [`super::decided_as`]: it's new, its audio, its fingerprint or
//! its names changed (the database clears the record then, whichever code
//! made the change), it came back, or the rules changed. It's **ready** when
//! it has no fingerprint, or matching has been through its fingerprint
//! (`fingerprint_matched`): before that, the stored comparisons can't be
//! told from "never compared". Each run takes the files that are both, in
//! pages, and a file is marked decided in the same transaction as the last
//! of its pairs, only if nothing a decision reads changed meanwhile. So a
//! run that stops anywhere leaves every file either decided with all its
//! pairs, or due.
//!
//! # Which pairs
//!
//! For a due file, the files it's asked about are:
//!
//! - **every file matching compared it with** (`fingerprint_match`),
//!   through the files standing for one fingerprint: files holding the very
//!   same fingerprint are asked about too;
//! - **every file holding the very same audio** (the same `audio_hash`, or
//!   the same `blake3` where there's none), up to [`MAX_TITLE_BUCKET`] of
//!   them;
//! - **every file whose title is the same** as one of its own, and, for a
//!   mashup, every file titled as one of its parts ([`Names`]). The
//!   matching pass rarely proposes a weak match, so of these, a pair the
//!   rules would answer "never compared" for, or that names a mashup's
//!   part, **is compared directly** ([`crate::matching::compare()`]) before
//!   the rules decide, and the comparison is kept where the pass keeps its
//!   own.
//!
//! Nothing is asked about every pair of the library: the work grows with
//! the files, their stored comparisons, and the files sharing a title. A
//! title more than [`MAX_TITLE_BUCKET`] files share ("Intro", "Track 01")
//! is narrowed to the files that also share an artist, and where that is
//! still too many (or the files name no artist) the pairs are skipped:
//! not asked about, not recorded, left apart, and counted in [`Decided`].
//!
//! Pairs the rules call "different" aren't stored, except inside a set of
//! duplicates that waits for them ([`super::store::undecided_pairs`]):
//! those are decided last, compared directly where nobody compared them.

use std::collections::{BTreeSet, HashMap, HashSet};

use rusqlite::{params, Connection, OptionalExtension};

use crate::db::{DbError, Writer};
use crate::fingerprint::Fingerprint;
use crate::matching::{self, Comparison};
use crate::versions::{self, CreditRole, ParsedName};

use super::plan::{Key, Outcome};
use super::rules::{self, Audio, FileFacts, Measured, Pair, Reason, Side};
use super::store::CONTENT_KEY;

/// A title shared by more files than this isn't compared pair by pair: see
/// the module docs. Also how many files holding the very same audio are
/// asked about each other.
pub const MAX_TITLE_BUCKET: usize = 50;

/// How many files are decided per transaction.
pub const PAGE: usize = 200;

/// How many files' names are read per query when the index is built.
const NAMES_PAGE: usize = 1000;

/// What one deciding run did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Decided {
    /// Files whose pairs were all decided and written.
    pub files: u64,
    /// Pairs the rules were asked about.
    pub pairs: u64,
    /// Pairs stored: every one that wasn't "different".
    pub recorded: u64,
    /// Pairs compared directly, because the matching pass never proposed
    /// them.
    pub compared_directly: u64,
    /// Titles (or one artist's share of a title) too many files hold to
    /// compare, and the pairs of files left unasked because of it.
    pub skipped_buckets: u64,
    pub skipped_pairs: u64,
    /// Pairs decided inside sets of duplicates that waited for them.
    pub completed: u64,
}

impl Decided {
    /// Whether anything was written that a grouping pass should read.
    pub fn wrote(&self) -> bool {
        self.files + self.completed > 0
    }
}

/// A title, an artist or a mashup's part as files are looked up by it.
/// `None` for one with no letter or digit: it would only gather junk.
fn lookup_key(text: &str) -> Option<String> {
    let key = versions::title_key(text);
    key.chars().any(char::is_alphanumeric).then_some(key)
}

/// What one file's names are looked up by.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Entry {
    /// Every title its title tag or its file name can be read as.
    titles: Vec<String>,
    /// The titles it names as a mashup's parts.
    parts: Vec<String>,
    artists: Vec<String>,
}

impl Entry {
    fn new(title_tag: Option<&str>, artist_tag: Option<&str>, file_name: &str) -> Entry {
        let mut entry = Entry::default();
        let mut take = |name: &ParsedName| {
            for reading in name.readings() {
                entry.titles.extend(lookup_key(&reading.base_title));
            }
            for part in &name.mashup_parts {
                entry.parts.extend(lookup_key(part));
            }
            for credit in &name.credits {
                if credit.role == CreditRole::Main {
                    entry.artists.extend(lookup_key(&credit.name));
                }
            }
        };
        take(&versions::parse_file_name(file_name));
        if let Some(title) = title_tag.filter(|title| !title.trim().is_empty()) {
            take(&versions::parse_title(title));
        }
        for credit in artist_tag.map(versions::parse_artists).unwrap_or_default() {
            if credit.role == CreditRole::Main {
                entry.artists.extend(lookup_key(&credit.name));
            }
        }
        for keys in [&mut entry.titles, &mut entry.parts, &mut entry.artists] {
            keys.sort_unstable();
            keys.dedup();
        }
        entry
    }
}

/// The files of one over-full title, by artist.
#[derive(Debug, Default)]
struct Narrowed {
    by_artist: HashMap<String, Vec<i64>>,
    /// The files that name no artist.
    unnamed: Vec<i64>,
}

/// Every present file's titles, in memory: which files share a title, and
/// which mashups name a title as a part. Kept between runs by the job's
/// handler, and brought up to date by each run that has a file to decide,
/// before it decides the first ([`Names::catch_up`]); a run with nothing
/// due never builds it. It's only what candidates are found by, never what
/// a decision is made on, which is read from the database each time.
#[derive(Debug, Default)]
pub struct Names {
    of_file: HashMap<i64, Entry>,
    titles: HashMap<String, BTreeSet<i64>>,
    /// The mashups naming each title as a part.
    parts: HashMap<String, BTreeSet<i64>>,
    /// Over-full titles sorted by artist, worked out once per run.
    narrowed: HashMap<String, Narrowed>,
    /// What this run already counted as skipped.
    counted: HashSet<String>,
}

impl Names {
    /// How many files the index holds: none until a run had a file to
    /// decide.
    pub fn files(&self) -> usize {
        self.of_file.len()
    }

    fn remove(&mut self, file: i64) {
        let Some(entry) = self.of_file.remove(&file) else {
            return;
        };
        for (keys, index) in [
            (&entry.titles, &mut self.titles),
            (&entry.parts, &mut self.parts),
        ] {
            for key in keys {
                if let Some(files) = index.get_mut(key) {
                    files.remove(&file);
                    if files.is_empty() {
                        index.remove(key);
                    }
                }
            }
        }
    }

    fn set(&mut self, file: i64, entry: Entry) {
        self.remove(file);
        for key in &entry.titles {
            self.titles.entry(key.clone()).or_default().insert(file);
        }
        for key in &entry.parts {
            self.parts.entry(key.clone()).or_default().insert(file);
        }
        self.of_file.insert(file, entry);
    }

    /// Brings the index up to date with the file table: every present
    /// file, and the names of each file that is new to it or due.
    fn catch_up<E: From<DbError>>(
        &mut self,
        writer: &Writer,
        tick: &mut dyn FnMut(f64) -> Result<(), E>,
    ) -> Result<(), E> {
        self.narrowed.clear();
        self.counted.clear();
        let present = writer.call(|c| {
            let mut stmt = c.prepare(
                "SELECT f.id, g.decided_as IS ?1
                 FROM file f LEFT JOIN grouping_file g ON g.file_id = f.id
                 WHERE f.present = 1 ORDER BY f.id",
            )?;
            let rows = stmt.query_map([super::decided_as()], |r| Ok((r.get(0)?, r.get(1)?)))?;
            rows.collect::<rusqlite::Result<Vec<(i64, bool)>>>()
        })?;
        let now: HashSet<i64> = present.iter().map(|(file, _)| *file).collect();
        let gone: Vec<i64> = self
            .of_file
            .keys()
            .filter(|file| !now.contains(file))
            .copied()
            .collect();
        for file in gone {
            self.remove(file);
        }
        let stale: Vec<i64> = present
            .iter()
            .filter(|(file, decided)| !decided || !self.of_file.contains_key(file))
            .map(|(file, _)| *file)
            .collect();
        for page in stale.chunks(NAMES_PAGE) {
            tick(0.0)?;
            let ids = page.to_vec();
            let named = writer.call(move |c| names_of(c, &ids))?;
            for (file, names) in named {
                self.set(file, names.entry());
            }
        }
        Ok(())
    }

    /// The files of title `key` that `file` is compared with: all of them,
    /// or, for an over-full title, the ones sharing an artist with it.
    fn sharing_title(&mut self, file: i64, key: &str, out: &mut BTreeSet<i64>, run: &mut Decided) {
        let Some(files) = self.titles.get(key) else {
            return;
        };
        if files.len() <= MAX_TITLE_BUCKET {
            out.extend(files.iter().copied());
            return;
        }
        let of_file = &self.of_file;
        let narrowed = self.narrowed.entry(key.to_string()).or_insert_with(|| {
            let mut narrowed = Narrowed::default();
            for &other in files {
                let artists = of_file.get(&other).map(|e| e.artists.as_slice());
                match artists.unwrap_or_default() {
                    [] => narrowed.unnamed.push(other),
                    artists => {
                        for artist in artists {
                            narrowed
                                .by_artist
                                .entry(artist.clone())
                                .or_default()
                                .push(other);
                        }
                    }
                }
            }
            narrowed
        });
        let artists = of_file.get(&file).map(|e| e.artists.as_slice());
        let shares: Vec<(String, &Vec<i64>)> = match artists.unwrap_or_default() {
            [] => vec![(format!("t\0{key}\0"), &narrowed.unnamed)],
            artists => artists
                .iter()
                .filter_map(|a| Some((format!("t\0{key}\0{a}"), narrowed.by_artist.get(a)?)))
                .collect(),
        };
        for (name, share) in shares {
            if share.len() <= MAX_TITLE_BUCKET {
                out.extend(share.iter().copied());
            } else if self.counted.insert(name) {
                run.skipped_buckets += 1;
                run.skipped_pairs += (share.len() * (share.len() - 1) / 2) as u64;
            }
        }
    }

    /// The files `file`'s names lead to: the ones sharing a title with it,
    /// the mashups naming one of its titles as a part, and the files titled
    /// as one of its own parts. Never `file` itself.
    fn candidates(&mut self, file: i64, run: &mut Decided) -> BTreeSet<i64> {
        let mut out = BTreeSet::new();
        let Some(entry) = self.of_file.get(&file).cloned() else {
            return out;
        };
        for key in &entry.titles {
            self.sharing_title(file, key, &mut out, run);
        }
        // A mashup and a file titled as its part: only where neither side
        // of that title is over-full, so the pair is found from both.
        let mut across = |key: &str, mine_are_parts: bool, out: &mut BTreeSet<i64>| {
            let (Some(titled), Some(naming)) = (self.titles.get(key), self.parts.get(key)) else {
                return;
            };
            if titled.len() <= MAX_TITLE_BUCKET && naming.len() <= MAX_TITLE_BUCKET {
                out.extend(match mine_are_parts {
                    true => titled.iter().copied(),
                    false => naming.iter().copied(),
                });
            } else if self.counted.insert(format!("p\0{key}")) {
                run.skipped_buckets += 1;
                run.skipped_pairs += (titled.len() * naming.len()) as u64;
            }
        };
        for key in &entry.parts {
            across(key, true, &mut out);
        }
        for key in &entry.titles {
            across(key, false, &mut out);
        }
        out.remove(&file);
        out
    }
}

/// One file's names as stored.
struct StoredNames {
    title: Option<String>,
    artist: Option<String>,
    file_name: String,
}

impl StoredNames {
    fn new(rel_path: &str, raw_tags: Option<&str>) -> StoredNames {
        let mut names = StoredNames {
            title: None,
            artist: None,
            file_name: rel_path.rsplit('/').next().unwrap_or(rel_path).to_string(),
        };
        let tags = raw_tags.map(|raw| crate::send_values::tag_values(raw, &["Name", "Artist"]));
        for tag in tags.unwrap_or_default() {
            match tag.attribute {
                "Name" => names.title = Some(tag.value),
                "Artist" => names.artist = Some(tag.value),
                _ => {}
            }
        }
        names
    }

    fn entry(&self) -> Entry {
        Entry::new(
            self.title.as_deref(),
            self.artist.as_deref(),
            &self.file_name,
        )
    }

    fn facts(&self) -> FileFacts {
        FileFacts::new(
            self.title.as_deref(),
            self.artist.as_deref(),
            &self.file_name,
        )
    }
}

fn names_of(conn: &Connection, ids: &[i64]) -> rusqlite::Result<Vec<(i64, StoredNames)>> {
    let ids = serde_json::to_string(ids).unwrap_or_else(|_| "[]".into());
    let mut stmt = conn.prepare_cached(
        "SELECT id, rel_path, raw_tags FROM file
         WHERE id IN (SELECT value FROM json_each(?1)) AND present = 1",
    )?;
    let rows = stmt.query_map([ids], |r| {
        let (rel_path, raw_tags): (String, Option<String>) = (r.get(1)?, r.get(2)?);
        Ok((r.get(0)?, StoredNames::new(&rel_path, raw_tags.as_deref())))
    })?;
    rows.collect()
}

/// What a decision reads about one file.
struct Known {
    facts: FileFacts,
    /// The very same audio has the very same content.
    content: Option<Key>,
    has_fingerprint: bool,
    // The file's quality verdict goes here when "truncated: yes / no /
    // unknown" becomes an input (TASKS 1bC-1, with 1bC-5).
}

fn known(conn: &Connection, ids: &[i64]) -> rusqlite::Result<HashMap<i64, Known>> {
    let ids = serde_json::to_string(ids).unwrap_or_else(|_| "[]".into());
    let mut stmt = conn.prepare_cached(&format!(
        "SELECT f.id, f.rel_path, f.raw_tags, {CONTENT_KEY}, f.fingerprint IS NOT NULL
         FROM file f
         WHERE f.id IN (SELECT value FROM json_each(?1)) AND f.present = 1"
    ))?;
    let rows = stmt.query_map([ids], |r| {
        let (rel_path, raw_tags): (String, Option<String>) = (r.get(1)?, r.get(2)?);
        let content: Option<Key> = r.get(3)?;
        Ok((
            r.get(0)?,
            Known {
                facts: StoredNames::new(&rel_path, raw_tags.as_deref()).facts(),
                content: content.filter(|key| !key.is_empty()),
                has_fingerprint: r.get(4)?,
            },
        ))
    })?;
    rows.collect()
}

/// The next `limit` files that are due and ready, by id after `after`:
/// each one's id and its record's `rev`.
fn due_page(conn: &Connection, after: i64, limit: usize) -> rusqlite::Result<Vec<(i64, i64)>> {
    let mut stmt = conn.prepare_cached(
        "SELECT g.file_id, g.rev
         FROM grouping_file g JOIN file f ON f.id = g.file_id
         WHERE g.file_id > ?1 AND f.present = 1 AND g.decided_as IS NOT ?2
           AND (f.fingerprint IS NULL
                OR EXISTS (SELECT 1 FROM fingerprint_matched m
                           WHERE m.file_id = f.id AND m.version = ?3))
         ORDER BY g.file_id LIMIT ?4",
    )?;
    let rows = stmt.query_map(
        params![after, super::decided_as(), matching::VERSION, limit as i64],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    rows.collect()
}

/// Whether a deciding run has anything to do: a present file is due and
/// ready (or has no record at all yet), or a pair still says "waiting".
pub fn any_due(conn: &Connection) -> rusqlite::Result<bool> {
    conn.prepare_cached(
        "SELECT EXISTS (
             SELECT 1 FROM file f LEFT JOIN grouping_file g ON g.file_id = f.id
             WHERE f.present = 1 AND g.decided_as IS NOT ?1
               AND (f.fingerprint IS NULL
                    OR EXISTS (SELECT 1 FROM fingerprint_matched m
                               WHERE m.file_id = f.id AND m.version = ?2)))
           OR EXISTS (SELECT 1 FROM grouping_pair WHERE state = 'waiting')",
    )?
    .query_row(params![super::decided_as(), matching::VERSION], |r| {
        r.get(0)
    })
}

/// The files of the two sides of a pair, lower id first.
fn ordered(a: i64, b: i64) -> (i64, i64) {
    (a.min(b), a.max(b))
}

/// Which stored comparisons stand for which files: matching keeps one
/// fingerprint's results under the first of the files holding it.
#[derive(Default)]
struct Standing {
    /// The file standing for each file matching has been through.
    stands: HashMap<i64, i64>,
    /// The files each one stands for, itself included.
    stood_for: HashMap<i64, Vec<i64>>,
}

impl Standing {
    fn read(conn: &Connection) -> rusqlite::Result<Standing> {
        let stands = matching::store::matched(conn)?;
        let mut stood_for: HashMap<i64, Vec<i64>> = HashMap::new();
        for (&file, &stands) in &stands {
            stood_for.entry(stands).or_default().push(file);
        }
        Ok(Standing { stands, stood_for })
    }

    /// The file whose fingerprint and results stand for `file`'s: itself
    /// until matching has been through it.
    fn of(&self, file: i64) -> i64 {
        self.stands.get(&file).copied().unwrap_or(file)
    }
}

/// A full match with nothing differing: two files holding the very same
/// fingerprint.
fn same_fingerprint() -> Option<Measured> {
    Measured::new(1.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1).ok()
}

/// A pair that was decided, as it's stored.
struct Row {
    a: i64,
    b: i64,
    outcome: Outcome,
    reasons: String,
    link_label: Option<String>,
    link_source: Option<&'static str>,
    link_first: Option<&'static str>,
}

fn row(a: i64, b: i64, decision: &rules::Decision) -> Row {
    let deciding = decision.reasons.first();
    let mashup = match deciding {
        Some(Reason::MashupContains { mashup, .. }) => Some(*mashup),
        _ => None,
    };
    let (outcome, labels) = match &decision.outcome {
        rules::Outcome::Duplicate => (Outcome::Duplicate, None),
        rules::Outcome::CutLink { labels } => (Outcome::Cut, Some(labels)),
        rules::Outcome::ReworkLink { labels } => (Outcome::Rework, Some(labels)),
        rules::Outcome::NeedsReview => (Outcome::Review, None),
        rules::Outcome::Different => (Outcome::Different, None),
    };
    let link_label = labels.and_then(|labels| match mashup {
        Some(_) => Some("mashup-contains".to_string()),
        None if labels.is_empty() => None,
        None => Some(
            labels
                .iter()
                .map(|kind| kind.as_str())
                .collect::<Vec<_>>()
                .join("/"),
        ),
    });
    // What made the link: the names alone, or the audio too.
    let by_names = matches!(
        deciding,
        Some(Reason::NamesSayCut { .. } | Reason::NamesSayRework { .. })
    );
    Row {
        a,
        b,
        outcome,
        reasons: serde_json::to_string(&decision.reasons).unwrap_or_else(|_| "[]".into()),
        link_label,
        link_source: labels.map(|_| if by_names { "parser" } else { "fingerprint" }),
        link_first: mashup.map(|side| match side {
            Side::A => "a",
            Side::B => "b",
        }),
    }
}

/// A direct comparison waiting to be stored: the two files standing for
/// the fingerprints, the lower id first, the blobs compared and the result.
struct Compared {
    low: i64,
    blob_low: Vec<u8>,
    high: i64,
    blob_high: Vec<u8>,
    comparison: Comparison,
}

/// What is known about the audio of the pairs being decided.
#[derive(Default)]
struct AudioKnown {
    /// Stored comparisons, by the two files standing for the fingerprints,
    /// the lower id first (its A).
    stored: HashMap<(i64, i64), Comparison>,
    /// Pairs looked up and not found: nobody compared them.
    never: HashSet<(i64, i64)>,
}

impl AudioKnown {
    /// The audio of files `a` and `b`, seen from `a`.
    fn of(&self, standing: &Standing, a: (i64, &Known), b: (i64, &Known)) -> Audio {
        if a.1.content.is_some() && a.1.content == b.1.content {
            return Audio::Identical;
        }
        if !(a.1.has_fingerprint && b.1.has_fingerprint) {
            return Audio::Unknown;
        }
        let (sa, sb) = (standing.of(a.0), standing.of(b.0));
        if sa == sb {
            return same_fingerprint().map_or(Audio::NotCompared, Audio::Compared);
        }
        let Some(stored) = self.stored.get(&ordered(sa, sb)) else {
            return Audio::NotCompared;
        };
        // A stored result that isn't a measurement says nothing.
        match Measured::try_from(stored) {
            Ok(measured) if sa < sb => Audio::Compared(measured),
            Ok(measured) => Audio::Compared(measured.swapped()),
            Err(_) => Audio::NotCompared,
        }
    }
}

/// Reads the stored comparisons of each of `pairs` (files standing for
/// fingerprints, the lower first) that isn't known yet.
fn look_up(writer: &Writer, audio: &mut AudioKnown, pairs: Vec<(i64, i64)>) -> Result<(), DbError> {
    let unknown: Vec<(i64, i64)> = pairs
        .into_iter()
        .filter(|pair| !audio.stored.contains_key(pair) && !audio.never.contains(pair))
        .collect();
    if unknown.is_empty() {
        return Ok(());
    }
    let found = writer.call(move |c| {
        let mut found = Vec::with_capacity(unknown.len());
        for (low, high) in unknown {
            found.push(((low, high), matching::store::of_pair(c, low, high)?));
        }
        Ok(found)
    })?;
    for (pair, comparison) in found {
        match comparison {
            Some(comparison) => {
                audio.stored.insert(pair, comparison);
            }
            None => {
                audio.never.insert(pair);
            }
        }
    }
    Ok(())
}

/// Compares directly each of `pairs` (files standing for fingerprints, the
/// lower first): reads the two fingerprints and lines them up, off the
/// writer. A pair whose fingerprints can't be read or compared stays
/// unknown.
fn compare_directly(
    writer: &Writer,
    audio: &mut AudioKnown,
    pairs: BTreeSet<(i64, i64)>,
    run: &mut Decided,
) -> Result<Vec<Compared>, DbError> {
    if pairs.is_empty() {
        return Ok(Vec::new());
    }
    let mut ids: Vec<i64> = pairs.iter().flat_map(|&(low, high)| [low, high]).collect();
    ids.sort_unstable();
    ids.dedup();
    let blobs: HashMap<i64, Vec<u8>> = writer
        .call(move |c| matching::store::fingerprints_of(c, &ids))?
        .into_iter()
        .map(|(file, blob, _)| (file, blob))
        .collect();
    let mut prints: HashMap<i64, Option<Fingerprint>> = HashMap::new();
    let mut compared = Vec::new();
    for (low, high) in pairs {
        let (Some(blob_low), Some(blob_high)) = (blobs.get(&low), blobs.get(&high)) else {
            continue;
        };
        for (file, blob) in [(low, blob_low), (high, blob_high)] {
            prints
                .entry(file)
                .or_insert_with(|| Fingerprint::from_blob(blob).ok());
        }
        let (Some(Some(a)), Some(Some(b))) = (prints.get(&low), prints.get(&high)) else {
            continue;
        };
        // The lower file id is A, as the matching pass stores it.
        let Ok(comparison) = matching::compare(a, b) else {
            continue;
        };
        run.compared_directly += 1;
        audio.never.remove(&(low, high));
        audio.stored.insert((low, high), comparison.clone());
        compared.push(Compared {
            low,
            blob_low: blob_low.clone(),
            high,
            blob_high: blob_high.clone(),
            comparison,
        });
    }
    Ok(compared)
}

/// Stores direct comparisons where the matching pass keeps its own, each
/// only while both files still hold the fingerprints that were compared.
fn store_compared(conn: &Connection, compared: &[Compared]) -> rusqlite::Result<()> {
    for found in compared {
        let a = matching::store::Side {
            file: found.low,
            blob: &found.blob_low,
        };
        let b = matching::store::Side {
            file: found.high,
            blob: &found.blob_high,
        };
        matching::store::put(conn, a, b, &found.comparison)?;
    }
    Ok(())
}

fn store_rows(conn: &Connection, rows: &[Row]) -> rusqlite::Result<()> {
    // A pair that was there keeps where it stands until the next grouping
    // pass says otherwise.
    let mut put = conn.prepare_cached(
        "INSERT INTO grouping_pair
             (file_a, file_b, outcome, reasons, link_label, link_source, link_first)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT (file_a, file_b) DO UPDATE
             SET outcome = excluded.outcome, reasons = excluded.reasons,
                 link_label = excluded.link_label, link_source = excluded.link_source,
                 link_first = excluded.link_first, decided_at = excluded.decided_at
             WHERE outcome IS NOT excluded.outcome OR reasons IS NOT excluded.reasons
                OR link_label IS NOT excluded.link_label
                OR link_source IS NOT excluded.link_source
                OR link_first IS NOT excluded.link_first",
    )?;
    for row in rows {
        put.execute(params![
            row.a,
            row.b,
            row.outcome.as_str(),
            row.reasons,
            row.link_label,
            row.link_source,
            row.link_first,
        ])?;
    }
    Ok(())
}

/// Asks the rules about files `a` and `b` (`a` the lower id) with what is
/// known of their audio.
fn ask(
    standing: &Standing,
    audio: &AudioKnown,
    known: &HashMap<i64, Known>,
    a: i64,
    b: i64,
) -> Option<(Audio, rules::Decision)> {
    let (ka, kb) = (known.get(&a)?, known.get(&b)?);
    let heard = audio.of(standing, (a, ka), (b, kb));
    let decision = rules::decide(&Pair::new(heard, &ka.facts, &kb.facts));
    Some((heard, decision))
}

/// The pair's names are the same, or one is a mashup naming the other as a
/// part: what a weak match the pass never proposed changes the answer for
/// (the rules' contract with their caller).
fn names_ask_for_a_comparison(decision: &rules::Decision) -> bool {
    matches!(
        decision.reasons.first(),
        Some(Reason::NotCompared | Reason::MashupPart { .. })
    )
}

/// One deciding run over every file that is due and ready, then over the
/// pairs waiting sets still miss. `tick` is called often with how far along
/// it is; `stop_here` at each point where the run can stop without losing
/// work (between pages), once this run has written something. An error
/// from either stops the run there; what was written stays.
pub fn run<E: From<DbError>>(
    writer: &Writer,
    names: &mut Names,
    page: usize,
    tick: &mut dyn FnMut(f64) -> Result<(), E>,
    stop_here: &mut dyn FnMut() -> Result<(), E>,
) -> Result<Decided, E> {
    let mut run = Decided::default();
    let page = page.max(1);
    tick(0.0)?;
    let standing = writer.call(|c| Standing::read(c))?;
    let total = writer
        .call(|c| {
            c.query_row(
                "SELECT count(*) FROM grouping_file WHERE decided_as IS NOT ?1",
                [super::decided_as()],
                |r| r.get::<_, i64>(0),
            )
        })?
        .max(1) as f64;

    let mut after = 0;
    let mut seen = 0u64;
    let mut caught_up = false;
    loop {
        tick((seen as f64 / total).min(0.95))?;
        if run.files > 0 {
            stop_here()?;
        }
        let due = writer.call(move |c| due_page(c, after, page))?;
        let Some(&(last, _)) = due.last() else {
            break;
        };
        after = last;
        seen += due.len() as u64;
        // The index of names is only needed to decide a file: a run with
        // none to decide never builds it (after a restart that is every
        // present file's names read again).
        if !caught_up {
            names.catch_up(writer, tick)?;
            caught_up = true;
        }
        decide_page(writer, names, &standing, &due, &mut run)?;
    }

    // The pairs sets of duplicates still wait for: every one is decided,
    // compared directly where nobody compared the two.
    let missing = writer.call(|c| super::store::undecided_pairs(c))?;
    for pairs in missing.chunks(page) {
        tick(0.97)?;
        if run.wrote() {
            stop_here()?;
        }
        complete(writer, &standing, pairs, &mut run)?;
    }
    Ok(run)
}

/// Decides every pair of one page of due files and writes them, marking
/// each file decided if nothing about it changed since it was read.
fn decide_page(
    writer: &Writer,
    names: &mut Names,
    standing: &Standing,
    due: &[(i64, i64)],
    run: &mut Decided,
) -> Result<(), DbError> {
    // What matching stored for each due file's fingerprint.
    let mut stood: Vec<i64> = due.iter().map(|&(file, _)| standing.of(file)).collect();
    stood.sort_unstable();
    stood.dedup();
    let mut audio = AudioKnown::default();
    let stored = writer.call(move |c| {
        let mut all = Vec::new();
        for file in stood {
            all.extend(matching::store::of_file(c, file)?);
        }
        Ok(all)
    })?;
    for found in stored {
        audio
            .stored
            .insert((found.file_a, found.file_b), found.comparison);
    }
    let mut compared_with: HashMap<i64, Vec<i64>> = HashMap::new();
    for &(a, b) in audio.stored.keys() {
        compared_with.entry(a).or_default().push(b);
        compared_with.entry(b).or_default().push(a);
    }

    // Each due file's partners: by name, by stored comparison, by the very
    // same fingerprint. Every partner is a present file.
    let mut pairs: BTreeSet<(i64, i64)> = BTreeSet::new();
    for &(file, _) in due {
        let mut partners = names.candidates(file, run);
        let stands = standing.of(file);
        let same_print = standing.stood_for.get(&stands).into_iter().flatten();
        partners.extend(same_print.copied());
        for other in compared_with.get(&stands).into_iter().flatten() {
            partners.extend(standing.stood_for.get(other).into_iter().flatten().copied());
            partners.insert(*other);
        }
        for partner in partners {
            if partner != file && names.of_file.contains_key(&partner) {
                pairs.insert(ordered(file, partner));
            }
        }
    }
    let mut ids: Vec<i64> = pairs.iter().flat_map(|&(a, b)| [a, b]).collect();
    ids.extend(due.iter().map(|&(file, _)| file));
    ids.sort_unstable();
    ids.dedup();
    let mut known = writer.call(move |c| known(c, &ids))?;

    // The very same audio: each due file with up to a bucket of the files
    // holding it.
    let contents: Vec<Key> = due
        .iter()
        .filter_map(|(file, _)| known.get(file)?.content.clone())
        .collect();
    let twins = writer.call(move |c| identical(c, &contents))?;
    let mut extra: Vec<i64> = Vec::new();
    for &(file, _) in due {
        let Some(content) = known.get(&file).and_then(|k| k.content.as_ref()) else {
            continue;
        };
        for &twin in twins.get(content).into_iter().flatten() {
            if twin != file {
                pairs.insert(ordered(file, twin));
                extra.push(twin);
            }
        }
    }
    extra.retain(|file| !known.contains_key(file));
    if !extra.is_empty() {
        known.extend(writer.call(move |c| self::known(c, &extra))?);
    }

    // What nobody stored is looked up, then asked about; the pairs whose
    // names call for it are compared directly and asked about again.
    let stands_of = |&(a, b): &(i64, i64)| ordered(standing.of(a), standing.of(b));
    look_up(
        writer,
        &mut audio,
        pairs
            .iter()
            .map(stands_of)
            .filter(|(a, b)| a != b)
            .collect(),
    )?;
    let mut decisions: Vec<(i64, i64, rules::Decision)> = Vec::new();
    let mut to_compare: BTreeSet<(i64, i64)> = BTreeSet::new();
    let mut again: Vec<(i64, i64)> = Vec::new();
    for &(a, b) in &pairs {
        let Some((heard, decision)) = ask(standing, &audio, &known, a, b) else {
            continue;
        };
        run.pairs += 1;
        if heard == Audio::NotCompared && names_ask_for_a_comparison(&decision) {
            to_compare.insert(stands_of(&(a, b)));
            again.push((a, b));
        } else {
            decisions.push((a, b, decision));
        }
    }
    let compared = compare_directly(writer, &mut audio, to_compare, run)?;
    for (a, b) in again {
        if let Some((_, decision)) = ask(standing, &audio, &known, a, b) {
            decisions.push((a, b, decision));
        }
    }

    let rows: Vec<Row> = decisions
        .iter()
        .filter(|(_, _, decision)| decision.outcome != rules::Outcome::Different)
        .map(|(a, b, decision)| row(*a, *b, decision))
        .collect();
    let kept: HashSet<(i64, i64)> = rows.iter().map(|row| (row.a, row.b)).collect();
    let due = due.to_vec();
    let (files, recorded) = writer.call(move |conn| {
        let tx = conn.transaction()?;
        store_compared(&tx, &compared)?;
        // The pairs a due file had and has no more go; the rest are
        // written over.
        {
            let mut had = tx.prepare_cached(
                "SELECT file_a, file_b FROM grouping_pair WHERE file_a = ?1 OR file_b = ?1",
            )?;
            let mut drop =
                tx.prepare_cached("DELETE FROM grouping_pair WHERE file_a = ?1 AND file_b = ?2")?;
            for &(file, _) in &due {
                let before: Vec<(i64, i64)> = had
                    .query_map([file], |r| Ok((r.get(0)?, r.get(1)?)))?
                    .collect::<rusqlite::Result<_>>()?;
                for pair in before {
                    if !kept.contains(&pair) {
                        drop.execute([pair.0, pair.1])?;
                    }
                }
            }
        }
        store_rows(&tx, &rows)?;
        // Decided, with the last of its pairs: unless something a decision
        // reads changed since the file was listed.
        let mut files = 0;
        {
            let mut mark = tx.prepare_cached(
                "UPDATE grouping_file SET decided_as = ?1 WHERE file_id = ?2 AND rev = ?3",
            )?;
            for &(file, rev) in &due {
                files += mark.execute(params![super::decided_as(), file, rev])? as u64;
            }
        }
        tx.commit()?;
        Ok((files, rows.len() as u64))
    })?;
    run.files += files;
    run.recorded += recorded;
    Ok(())
}

/// The present files holding each of `contents`, the lowest
/// [`MAX_TITLE_BUCKET`] ids of each.
fn identical(conn: &Connection, contents: &[Key]) -> rusqlite::Result<HashMap<Key, Vec<i64>>> {
    let mut by_hash = conn.prepare_cached(
        "SELECT id FROM file WHERE audio_hash = ?1 AND present = 1 ORDER BY id LIMIT ?2",
    )?;
    let mut by_blake3 = conn.prepare_cached(
        "SELECT id FROM file
         WHERE blake3 = ?1 AND (audio_hash IS NULL OR audio_hash = x'') AND present = 1
         ORDER BY id LIMIT ?2",
    )?;
    let mut out: HashMap<Key, Vec<i64>> = HashMap::new();
    for content in contents {
        if out.contains_key(content) {
            continue;
        }
        let limit = MAX_TITLE_BUCKET as i64 + 1;
        let mut files: Vec<i64> = by_hash
            .query_map(params![content, limit], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        if files.is_empty() {
            files = by_blake3
                .query_map(params![content, limit], |r| r.get(0))?
                .collect::<rusqlite::Result<_>>()?;
        }
        out.insert(content.clone(), files);
    }
    Ok(out)
}

/// Decides `pairs`, which sets of duplicates wait for, and writes every
/// answer, "different" included: a pair nobody compared is compared first.
fn complete(
    writer: &Writer,
    standing: &Standing,
    pairs: &[(i64, i64)],
    run: &mut Decided,
) -> Result<(), DbError> {
    let mut ids: Vec<i64> = pairs.iter().flat_map(|&(a, b)| [a, b]).collect();
    ids.sort_unstable();
    ids.dedup();
    let known = writer.call(move |c| known(c, &ids))?;
    let stands_of = |&(a, b): &(i64, i64)| ordered(standing.of(a), standing.of(b));
    let mut audio = AudioKnown::default();
    look_up(
        writer,
        &mut audio,
        pairs
            .iter()
            .map(stands_of)
            .filter(|(a, b)| a != b)
            .collect(),
    )?;
    let to_compare: BTreeSet<(i64, i64)> = pairs
        .iter()
        .filter(|(a, b)| {
            ask(standing, &audio, &known, *a, *b)
                .is_some_and(|(heard, _)| heard == Audio::NotCompared)
        })
        .map(stands_of)
        .collect();
    let compared = compare_directly(writer, &mut audio, to_compare, run)?;
    let rows: Vec<Row> = pairs
        .iter()
        .filter_map(|&(a, b)| {
            let (_, decision) = ask(standing, &audio, &known, a, b)?;
            Some(row(a, b, &decision))
        })
        .collect();
    run.pairs += rows.len() as u64;
    run.completed += rows.len() as u64;
    writer.call(move |conn| {
        let tx = conn.transaction()?;
        store_compared(&tx, &compared)?;
        store_rows(&tx, &rows)?;
        tx.commit()
    })
}

/// File `file`'s decided pairs: the other file, the outcome and where the
/// pair stands, for tests and for whoever reads them next (Review, 1bD).
pub fn pairs_of(conn: &Connection, file: i64) -> rusqlite::Result<Vec<(i64, String, String)>> {
    let mut stmt = conn.prepare_cached(
        "SELECT CASE WHEN file_a = ?1 THEN file_b ELSE file_a END, outcome, state
         FROM grouping_pair WHERE file_a = ?1 OR file_b = ?1 ORDER BY 1",
    )?;
    let rows = stmt.query_map([file], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
    rows.collect()
}

/// The reasons stored for the pair of files `a` and `b`, as JSON.
pub fn reasons_of(conn: &Connection, a: i64, b: i64) -> rusqlite::Result<Option<String>> {
    let (low, high) = ordered(a, b);
    conn.query_row(
        "SELECT reasons FROM grouping_pair WHERE file_a = ?1 AND file_b = ?2",
        [low, high],
        |r| r.get(0),
    )
    .optional()
}
