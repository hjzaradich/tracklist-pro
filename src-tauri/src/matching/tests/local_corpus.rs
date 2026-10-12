//! The local corpus (1bA-3, 1bB-7): E3's 43 hand-labeled pairs of real
//! files, run through the app's own fingerprinting and matching pass and
//! held to what each label requires ([`super::labels::unmet`]); then
//! through the grouping rules ([`crate::grouping::rules::decide`]) and
//! held to what each label allows ([`super::labels::allowed`]).
//!
//! The pair list, the labels and the audio are the owner's and stay
//! outside git, so this runs **only where they are**: it looks for them
//! in the folder the [`DIR_VAR`] environment variable names. With the
//! variable unset it says so in one line and passes: CI never has them.
//! With it set, the run is meant: a folder that isn't there, or a file
//! that can't be read, fails. `docs/local-corpus.md` says how to run it.
//!
//! In that folder it reads, and only reads:
//!
//! - `e3/pairs.csv`: each pair's id and its two audio files;
//! - `results-private/e3-labels.csv`: each pair's label (same, version or
//!   different);
//! - `results-private/e3-fingerprints.jsonl`: which versions are cuts, and
//!   the numbers E3's own tool measured, printed beside ours.
//!
//! Every audio file is opened read-only: fingerprinted by the app's
//! decoder, hashed by the app's hasher, and its title and artist tags read
//! by the app's tag reader. The fingerprints go into a throwaway database
//! under made-up file names, and the real matching pass runs over it. All
//! of that happens once, for both tests.
//!
//! **Nothing private is ever printed:** a pair is its id, its category and
//! numbers. No title, artist, file name or path reaches the output, a
//! failure message or the database.
//!
//! # What the pass does today
//!
//! [`SHARING`] of the 43 pairs share some audio without being a full or a
//! one-sided match. The pass proposes [`SHARING_PROPOSED`] of them. The
//! others line up only weakly, with hardly an item the same bit for bit,
//! so the block index has nothing to propose them by: nothing is stored
//! for them, and the grouping rules see "never compared", not "a partial
//! match". A second recording under identical names is one of those. The
//! test asserts both numbers, so a change to the pass shows up here as a
//! changed tally.
//!
//! # The grouping rules on the real pairs
//!
//! Each pair is put to the rules as their contract says: the audio is
//! "identical" when the two files share an audio hash or a whole-file
//! hash, else what the pass stored, else a direct comparison of the two
//! fingerprints; the names are each file's title tag, artist tag and file
//! name. One line may never be crossed: **no pair labeled anything other
//! than duplicate comes out a duplicate.** Beyond that, every outcome must
//! be one its label allows; "needs review" always is. For a pair the pass
//! never proposed, the run also prints what the rules say when nobody
//! compares the two, which is what the grouping job gets unless it
//! compares them itself.

use std::collections::{BTreeMap, HashMap};
use std::ffi::OsString;
use std::fs::File;
use std::io::{ErrorKind, Write as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};

use crate::fingerprint::decode::{self, Stopped};
use crate::fingerprint::Fingerprint;
use crate::grouping::rules::{self, Audio, FileFacts, Measured};
use crate::hash::{self, FileHashes};
use crate::matching::{compare, item_seconds, refresh, Comparison, Summary};
use crate::versions::{self, Outcome};

use super::corpus_job::{same_track, stands_as};
use super::labels::{
    allowed, identical_share, is_duplicate, same_length, shared_keys, unmet, Category, Shape,
};
use super::passes::db;
use crate::grouping::tests::support::{self, RealFile};

/// Names the folder holding E3's private files: the `spikes` folder of the
/// checkout that has them.
pub const DIR_VAR: &str = "TLP_E3_DIR";

const PAIRS: &str = "e3/pairs.csv";
const LABELS: &str = "results-private/e3-labels.csv";
const GRADED: &str = "results-private/e3-fingerprints.jsonl";

/// How many of the pairs share audio without a full or a one-sided match
/// ([`Shape::Partial`]), and how many of those the pass proposes. See the
/// module docs.
const SHARING: usize = 15;
const SHARING_PROPOSED: usize = 5;

/// One labeled pair.
struct Pair {
    id: u32,
    category: Category,
    paths: [PathBuf; 2],
    /// What E3's tool measured: coverage of A, of B, and the score.
    e3: Option<(f64, f64, f64)>,
}

/// Splits CSV text into rows of fields: quoted fields, doubled quotes and
/// line breaks inside quotes.
fn csv(text: &str) -> Vec<Vec<String>> {
    let (mut rows, mut row, mut field) = (Vec::new(), Vec::new(), String::new());
    let mut quoted = false;
    let mut chars = text.trim_start_matches('\u{feff}').chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' if quoted && chars.peek() == Some(&'"') => {
                chars.next();
                field.push('"');
            }
            '"' => quoted = !quoted,
            ',' if !quoted => row.push(std::mem::take(&mut field)),
            '\r' if !quoted => {}
            '\n' if !quoted => {
                row.push(std::mem::take(&mut field));
                rows.push(std::mem::take(&mut row));
            }
            c => field.push(c),
        }
    }
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }
    rows
}

/// A CSV file's rows, each as its fields by column name.
fn table(text: &str) -> Vec<HashMap<String, String>> {
    let mut rows = csv(text).into_iter();
    let header = rows.next().unwrap_or_default();
    rows.filter(|row| row.iter().any(|field| !field.is_empty()))
        .map(|row| header.iter().cloned().zip(row).collect())
        .collect()
}

/// A pair's category, from its label and whether E3 marked it a cut.
fn category(label: &str, cut: bool) -> Option<Category> {
    match (label, cut) {
        ("same", _) => Some(Category::Duplicate),
        ("different", _) => Some(Category::Different),
        ("version", true) => Some(Category::Cut),
        ("version", false) => Some(Category::Rework),
        _ => None,
    }
}

/// Reads one of the three files; a failure names the file, which is no
/// secret, and nothing in it.
fn read(dir: &Path, name: &str) -> String {
    std::fs::read_to_string(dir.join(name))
        .unwrap_or_else(|e| panic!("{DIR_VAR} is set, but {name} can't be read: {}", e.kind()))
}

fn load(dir: &Path) -> Vec<Pair> {
    let labels: HashMap<String, String> = table(&read(dir, LABELS))
        .into_iter()
        .map(|mut row| {
            (
                row.remove("id").unwrap_or_default(),
                row.remove("label").unwrap_or_default(),
            )
        })
        .collect();
    let graded: HashMap<String, serde_json::Value> = read(dir, GRADED)
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let row: serde_json::Value =
                serde_json::from_str(line).unwrap_or_else(|_| panic!("{GRADED} has a bad line"));
            (row["id"].as_str().unwrap_or_default().to_string(), row)
        })
        .collect();
    let mut pairs: Vec<Pair> = table(&read(dir, PAIRS))
        .into_iter()
        .map(|row| {
            let field = |name: &str| row.get(name).cloned().unwrap_or_default();
            let id = field("id");
            let number: u32 = id
                .parse()
                .unwrap_or_else(|_| panic!("{PAIRS} has a row whose id isn't a number"));
            let label = labels
                .get(&id)
                .unwrap_or_else(|| panic!("pair {number} has no label in {LABELS}"));
            let row = graded.get(&id);
            let cut = row.and_then(|r| r["cut"].as_bool()).unwrap_or(false);
            let number_of = |name: &str| row.and_then(|r| r[name].as_f64());
            Pair {
                id: number,
                category: category(label, cut)
                    .unwrap_or_else(|| panic!("pair {number} has a label this test doesn't know")),
                paths: [field("a_path").into(), field("b_path").into()],
                e3: number_of("cover_a")
                    .zip(number_of("cover_b"))
                    .zip(number_of("score"))
                    .map(|((a, b), score)| (a, b, score)),
            }
        })
        .collect();
    pairs.sort_by_key(|p| p.id);
    pairs
}

/// Fingerprints one audio file as the app does. A failure is a short
/// reason, never the path.
fn fingerprint(path: &Path) -> Result<Fingerprint, &'static str> {
    let file = File::open(path).map_err(|e| match e.kind() {
        ErrorKind::NotFound => "audio file not found",
        _ => "audio file can't be opened",
    })?;
    decode::fingerprint(Box::new(file), &mut |_| true).map_err(|stopped| match stopped {
        Stopped::Failed(why) => why.reason(),
        Stopped::Cancelled => "cancelled",
    })
}

/// One audio file as the app sees it. Its names never leave this
/// struct: they are parsed and compared, never printed.
struct Seen {
    fingerprint: Fingerprint,
    hashes: FileHashes,
    title: Option<String>,
    artist: Option<String>,
    file_name: String,
    /// The `raw_tags` JSON the read stage would store for the file.
    raw_tags: Option<String>,
}

impl Seen {
    /// The file's names as the grouping rules take them.
    fn facts(&self) -> FileFacts {
        FileFacts::new(
            self.title.as_deref(),
            self.artist.as_deref(),
            &self.file_name,
        )
    }

    /// The file's title as the version check reads it: its title tag, or
    /// its file name where it has none.
    fn tagged(&self) -> versions::ParsedName {
        match self.title.as_deref().filter(|t| !t.trim().is_empty()) {
            Some(title) => versions::parse_title(title),
            None => versions::parse_file_name(&self.file_name),
        }
    }
}

/// Fingerprints and hashes one file and reads its tags, as the app does.
/// A failure is a short reason, never the path.
fn see(path: &Path) -> Result<Seen, &'static str> {
    let fingerprint = fingerprint(path)?;
    let mut buffer = vec![0; hash::BUFFER];
    let hashes = hash::hash_path(path, &mut buffer, &mut |_| false)
        .ok()
        .flatten()
        .ok_or("audio file can't be hashed")?;
    let tags = crate::tags::read(path).map_err(|_| "tags can't be read")?;
    let raw_tags = crate::read::read_one(path, |_| Ok(true))
        .map_err(|_| "file can't be read by the read stage")?
        .raw_tags;
    Ok(Seen {
        fingerprint,
        hashes,
        title: tags.fields.title,
        artist: tags.fields.artist,
        file_name: file_name(path),
        raw_tags,
    })
}

/// Sees every file, a few at a time.
fn see_all(paths: &[&Path]) -> Vec<Result<Seen, &'static str>> {
    let threads = std::thread::available_parallelism().map_or(2, |n| n.get().min(4));
    let next = AtomicUsize::new(0);
    let done: Vec<Mutex<Option<Result<Seen, &'static str>>>> =
        paths.iter().map(|_| Mutex::new(None)).collect();
    std::thread::scope(|scope| {
        for _ in 0..threads {
            scope.spawn(|| loop {
                let i = next.fetch_add(1, Ordering::Relaxed);
                let Some(path) = paths.get(i) else { break };
                *done[i].lock().unwrap() = Some(see(path));
            });
        }
    });
    done.into_iter()
        .map(|slot| slot.into_inner().unwrap().expect("every file was tried"))
        .collect()
}

/// What two names say about each other, in a word.
fn word(outcome: Outcome) -> &'static str {
    match outcome {
        Outcome::SameVersion => "same",
        Outcome::DifferentCut => "cut",
        Outcome::DifferentRework => "rework",
        Outcome::DifferentBase => "other title",
        Outcome::CantTell => "unsure",
    }
}

/// What the names would have to say for a label.
fn names_for(category: Category) -> Outcome {
    match category {
        Category::Duplicate => Outcome::SameVersion,
        Category::Cut => Outcome::DifferentCut,
        Category::Rework => Outcome::DifferentRework,
        Category::Different => Outcome::DifferentBase,
    }
}

/// The name of the file at `path`, extension and all.
fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// What the version check says about two files, read two ways: from the
/// title tags (a file without one gives its file name), and from the two
/// file names.
fn names_say(a: &Seen, b: &Seen) -> (Outcome, Outcome) {
    let named = |file: &Seen| versions::parse_file_name(&file.file_name);
    (
        versions::compare(&a.tagged(), &b.tagged()).outcome,
        versions::compare(&named(a), &named(b)).outcome,
    )
}

/// Says why the corpus didn't run, where a test's captured output can't
/// hide it.
fn skipped(why: &str) {
    let _ = writeln!(
        std::io::stderr(),
        "local corpus skipped: {why} (docs/local-corpus.md)"
    );
}

/// The folder to read, from what [`DIR_VAR`] holds. Unset is `Ok(None)`:
/// nothing to run. Set is a deliberate run, so a folder that isn't there
/// is an error, never a quiet skip. The error doesn't repeat the path.
fn private_dir(var: Option<OsString>) -> Result<Option<PathBuf>, String> {
    let Some(dir) = var.filter(|dir| !dir.is_empty()) else {
        return Ok(None);
    };
    let dir = PathBuf::from(dir);
    if !dir.is_dir() {
        return Err(format!(
            "{DIR_VAR} is set, but the folder it names isn't there"
        ));
    }
    Ok(Some(dir))
}

/// The corpus, seen and passed once, for every test here.
struct Run {
    pairs: Vec<Pair>,
    /// Every file once, whichever pairs it's in.
    files: Vec<Seen>,
    /// Each pair's two files, as places in `files`.
    of_pair: Vec<[usize; 2]>,
    /// What the pass stored for each pair; `None` where it never proposed
    /// the pair.
    stored: Vec<Option<Comparison>>,
    summary: Summary,
    /// Stored results between files of two different pairs: how many, how
    /// many of them share audio, how many pass the duplicate rule.
    across: [usize; 3],
}

impl Run {
    fn files_of(&self, pair: usize) -> (&Seen, &Seen) {
        let [a, b] = self.of_pair[pair];
        (&self.files[a], &self.files[b])
    }

    /// How the pair's two files compare, and whether the pass proposed
    /// them. What the pass stored is what grouping will read; a pair it
    /// never proposed is compared directly.
    fn compared(&self, pair: usize) -> (Comparison, bool) {
        let (a, b) = self.files_of(pair);
        match &self.stored[pair] {
            Some(stored) => (stored.clone(), true),
            None => (
                compare(&a.fingerprint, &b.fingerprint).expect("one fingerprint version"),
                false,
            ),
        }
    }
}

fn build(dir: &Path) -> Run {
    let pairs = load(dir);
    assert!(!pairs.is_empty(), "{PAIRS} lists no pairs");

    let mut paths: Vec<&Path> = pairs
        .iter()
        .flat_map(|p| p.paths.iter().map(PathBuf::as_path))
        .collect();
    paths.sort_unstable();
    paths.dedup();
    let seen = see_all(&paths);
    let place = |path: &Path| paths.binary_search(&path).expect("listed above");

    // A set variable means "run it": a pair that can't be measured fails
    // the run, so the corpus never shrinks quietly.
    let mut unmeasured = Vec::new();
    for pair in &pairs {
        for (side, path) in ["A", "B"].iter().zip(&pair.paths) {
            if let Err(why) = &seen[place(path)] {
                unmeasured.push(format!("pair {} file {side}: {why}", pair.id));
            }
        }
    }
    assert!(
        unmeasured.is_empty(),
        "these files couldn't be read:\n{}",
        unmeasured.join("\n")
    );
    let files: Vec<Seen> = seen.into_iter().map(Result::unwrap).collect();

    // The real pass, over a database holding the fingerprints under
    // made-up names.
    let db = db();
    let ids: Vec<i64> = files
        .iter()
        .enumerate()
        .map(|(i, file)| db.add(&format!("{i}.audio"), &file.fingerprint))
        .collect();
    let summary = refresh(&db.writer).unwrap();
    let of_pair: Vec<[usize; 2]> = pairs
        .iter()
        .map(|p| [place(&p.paths[0]), place(&p.paths[1])])
        .collect();
    let stored: Vec<Option<Comparison>> = of_pair
        .iter()
        .map(|&[a, b]| db.of_pair(ids[a], ids[b]))
        .collect();
    // Each song was picked for one pair only, so little should line up
    // across pairs.
    let labeled: Vec<(i64, i64)> = of_pair
        .iter()
        .map(|&[a, b]| (ids[a].min(ids[b]), ids[a].max(ids[b])))
        .collect();
    let across: Vec<_> = db
        .all()
        .into_iter()
        .filter(|m| !labeled.contains(&(m.file_a, m.file_b)))
        .collect();
    let across = [
        across.len(),
        across
            .iter()
            .filter(|m| !m.comparison.segments.is_empty())
            .count(),
        across
            .iter()
            .filter(|m| is_duplicate(&m.comparison))
            .count(),
    ];
    Run {
        pairs,
        files,
        of_pair,
        stored,
        summary,
        across,
    }
}

/// The run, made on first use. `None` when [`DIR_VAR`] is unset, which is
/// said once, on stderr.
fn run() -> Option<&'static Run> {
    static RUN: OnceLock<Option<Run>> = OnceLock::new();
    RUN.get_or_init(|| match private_dir(std::env::var_os(DIR_VAR)) {
        Ok(Some(dir)) => Some(build(&dir)),
        Ok(None) => {
            skipped(&format!("{DIR_VAR} is not set"));
            None
        }
        Err(why) => panic!("{why}"),
    })
    .as_ref()
}

#[test]
fn every_labeled_pair_from_e3_measures_as_its_label_requires() {
    let Some(run) = run() else { return };
    let pairs = &run.pairs;
    println!(
        "{} pairs, {} files; the pass proposed {} pairs and stored {}",
        pairs.len(),
        run.files.len(),
        run.summary.candidates,
        run.summary.stored
    );
    println!(
        "coverage: of A / of B by the main alignment. found: over every alignment. \
         identical: the share of lined-up items that are the same bit for bit, and how many \
         index keys the two files share (the pass proposes from 2 up). \
         audio: full, one-sided, partial or none, as the grouping rules (1bB-1) read it. \
         names: what the version check says from the title tags, then from the file names."
    );

    let mut failures: Vec<String> = Vec::new();
    let mut tally: BTreeMap<Category, (u32, u32)> = BTreeMap::new();
    let mut names_tally: BTreeMap<Category, (u32, u32)> = BTreeMap::new();
    let (mut sharing, mut sharing_proposed) = (0, 0);
    for (i, pair) in pairs.iter().enumerate() {
        let (file_a, file_b) = run.files_of(i);
        let (a, b) = (&file_a.fingerprint, &file_b.fingerprint);
        let (c, proposed) = run.compared(i);
        if Shape::of(&c) == Shape::Partial {
            sharing += 1;
            sharing_proposed += usize::from(proposed);
        }

        let mut wrong: Vec<&str> = unmet(pair.category, &c, proposed);
        let (by_tags, by_files) = names_say(file_a, file_b);
        let wanted = names_for(pair.category);
        // Clean and Dirty pass the duplicate rule on audio, so the names
        // are all that stops a false merge: they must say "cut".
        if pair.category == Category::Cut && same_length(&c) {
            if by_tags != Outcome::DifferentCut && by_files != Outcome::DifferentCut {
                wrong.push("the names must say it's a cut");
            }
            if by_tags == Outcome::SameVersion || by_files == Outcome::SameVersion {
                wrong.push("no reading of the names may say it's the same version");
            }
        }
        let names_agree = by_tags == wanted && by_files == wanted;

        let alignments = c
            .segments
            .iter()
            .map(|s| s.alignment)
            .max()
            .map_or(0, |n| n + 1);
        let e3 = pair.e3.map_or(String::from("-"), |(a, b, score)| {
            format!("{a:.2}/{b:.2} {score:.2}")
        });
        let line = format!(
            "#{:<3}{:<10} coverage {:.2}/{:.2}  found {:.2}/{:.2}  score {:>5.2}  \
             {:>3.0}/{:<3.0} s  {:>2} segments in {} alignments  proposed {:<3}  \
             identical {:>3.0}% keys {:<3}  audio {:<9}  names {}, {}{}  E3 {}",
            pair.id,
            pair.category.as_str(),
            c.coverage_a,
            c.coverage_b,
            c.found_a(),
            c.found_b(),
            c.score,
            c.items_a as f32 * item_seconds(),
            c.items_b as f32 * item_seconds(),
            c.segments.len(),
            alignments,
            if proposed { "yes" } else { "no" },
            identical_share(&c, a, b) * 100.0,
            shared_keys(a, b),
            Shape::of(&c).as_str(),
            word(by_tags),
            word(by_files),
            if names_agree { "" } else { " (not the label)" },
            e3,
        );
        println!("{line}  {}", if wrong.is_empty() { "ok" } else { "FAIL" });
        let count = tally.entry(pair.category).or_default();
        count.1 += 1;
        if wrong.is_empty() {
            count.0 += 1;
        } else {
            failures.push(format!("{line}\n      unmet: {}", wrong.join("; ")));
        }
        let count = names_tally.entry(pair.category).or_default();
        count.1 += 1;
        count.0 += u32::from(names_agree);
    }

    let [across, sharing_across, merged_across] = run.across;
    println!(
        "across pairs: {across} proposed, {sharing_across} of them sharing audio, \
         {merged_across} passing the duplicate rule"
    );
    for category in Category::ALL {
        let (met, of) = tally.get(&category).copied().unwrap_or_default();
        let (named, _) = names_tally.get(&category).copied().unwrap_or_default();
        println!(
            "{:<10} {met}/{of} measure as the label requires; names say the label for {named}/{of}",
            category.as_str()
        );
    }
    assert!(
        failures.is_empty(),
        "{} of {} pairs don't measure as their label requires:\n{}",
        failures.len(),
        pairs.len(),
        failures.join("\n")
    );
    println!(
        "{sharing} pairs share audio without a full or one-sided match; \
         the pass proposed {sharing_proposed} of them"
    );
    assert_eq!(
        (sharing, sharing_proposed),
        (SHARING, SHARING_PROPOSED),
        "the pass proposes other weakly sharing pairs than it did: \
         correct the tally in this file and tell the grouping rules' owner"
    );
}

/// The audio as the grouping rules sort it (their module docs), for the
/// printed line only.
fn audio_class(audio: &Audio, c: &Comparison) -> &'static str {
    if matches!(audio, Audio::Identical) {
        return "identical";
    }
    let matched: usize = c.segments.iter().map(|s| s.items).sum();
    let found_score = c
        .segments
        .iter()
        .map(|s| s.score * s.items as f64)
        .sum::<f64>()
        / matched.max(1) as f64;
    let whole = |found: f32| found >= rules::DUPLICATE_COVERAGE;
    if is_duplicate(c) {
        "full"
    } else if whole(c.found_a()) != whole(c.found_b()) && found_score <= rules::DUPLICATE_SCORE {
        "one inside the other"
    } else if matched >= rules::MIN_SHARED_ITEMS {
        "partial"
    } else {
        "none"
    }
}

/// A decision in a few words and its reason codes. Codes only: a reason's
/// parameters can hold a title.
fn decided(decision: &rules::Decision) -> String {
    let kinds = |labels: &[versions::MarkerKind]| {
        labels
            .iter()
            .map(|kind| kind.as_str())
            .collect::<Vec<_>>()
            .join("+")
    };
    let outcome = match &decision.outcome {
        rules::Outcome::Duplicate => String::from("duplicate"),
        rules::Outcome::CutLink { labels } => format!("cut link [{}]", kinds(labels)),
        rules::Outcome::ReworkLink { labels } => format!("rework link [{}]", kinds(labels)),
        rules::Outcome::Different => String::from("different"),
        rules::Outcome::NeedsReview => String::from("review"),
    };
    format!("{outcome} ({})", codes(decision))
}

fn codes(decision: &rules::Decision) -> String {
    let codes: Vec<&str> = decision.reasons.iter().map(|r| r.code()).collect();
    codes.join(", ")
}

/// An outcome's kind, for the tally.
fn kind(outcome: &rules::Outcome) -> &'static str {
    match outcome {
        rules::Outcome::Duplicate => "duplicate",
        rules::Outcome::CutLink { .. } => "cut link",
        rules::Outcome::ReworkLink { .. } => "rework link",
        rules::Outcome::Different => "different",
        rules::Outcome::NeedsReview => "review",
    }
}

#[test]
fn the_grouping_rules_merge_no_pair_labeled_other_than_duplicate_and_keep_every_pair_within_what_its_label_allows(
) {
    let Some(run) = run() else { return };
    println!(
        "audio: as the grouping rules sort it. proposed: whether the pass compared the pair \
         (if not, it is compared directly here). Then the rules' outcome and its reason codes. \
         never compared: what the rules say for the same pair when nobody compares it."
    );
    let mut merged_wrongly: Vec<String> = Vec::new();
    let mut not_allowed: Vec<String> = Vec::new();
    let mut tally: BTreeMap<(Category, &str), u32> = BTreeMap::new();
    let mut duplicates_in_review: BTreeMap<String, Vec<u32>> = BTreeMap::new();
    for (i, pair) in run.pairs.iter().enumerate() {
        let (a, b) = run.files_of(i);
        let (c, proposed) = run.compared(i);
        let same_audio_hash = matches!((a.hashes.audio, b.hashes.audio), (Ok(x), Ok(y)) if x == y);
        let audio = if same_audio_hash || a.hashes.blake3 == b.hashes.blake3 {
            Audio::Identical
        } else {
            Audio::Compared(Measured::try_from(&c).expect("a comparison's own numbers"))
        };
        let (facts_a, facts_b) = (a.facts(), b.facts());
        let decision = rules::decide(&rules::Pair::new(audio, &facts_a, &facts_b));
        // What the grouping job gets for a pair the pass never proposed,
        // unless it compares the two itself.
        let uncompared = (!proposed && audio != Audio::Identical)
            .then(|| rules::decide(&rules::Pair::new(Audio::NotCompared, &facts_a, &facts_b)));

        let line = format!(
            "#{:<3}{:<10} audio {:<20} proposed {:<3}  {}{}",
            pair.id,
            pair.category.as_str(),
            audio_class(&audio, &c),
            if proposed { "yes" } else { "no" },
            decided(&decision),
            uncompared.as_ref().map_or(String::new(), |d| format!(
                "  never compared: {}",
                decided(d)
            )),
        );
        println!("{line}");
        *tally
            .entry((pair.category, kind(&decision.outcome)))
            .or_default() += 1;
        if pair.category == Category::Duplicate && decision.outcome == rules::Outcome::NeedsReview {
            duplicates_in_review
                .entry(codes(&decision))
                .or_default()
                .push(pair.id);
        }
        // The hard line, on both ways of asking.
        let merges = |d: &rules::Decision| d.outcome == rules::Outcome::Duplicate;
        if pair.category != Category::Duplicate
            && (merges(&decision) || uncompared.as_ref().is_some_and(merges))
        {
            merged_wrongly.push(line.clone());
        }
        if !allowed(pair.category, &decision.outcome) {
            not_allowed.push(line);
        }
    }

    for category in Category::ALL {
        let counts: Vec<String> = tally
            .iter()
            .filter(|((of, _), _)| *of == category)
            .map(|((_, outcome), n)| format!("{n} {outcome}"))
            .collect();
        println!("{:<10} {}", category.as_str(), counts.join(", "));
    }
    let merged = tally
        .get(&(Category::Duplicate, "duplicate"))
        .copied()
        .unwrap_or(0);
    let in_review: usize = duplicates_in_review.values().map(Vec::len).sum();
    println!("duplicates: {merged} merge without review, {in_review} to review");
    for (codes, ids) in &duplicates_in_review {
        println!("  to review for ({codes}): pairs {ids:?}");
    }
    assert!(
        merged_wrongly.is_empty(),
        "pairs not labeled duplicate that the rules merge:\n{}",
        merged_wrongly.join("\n")
    );
    assert!(
        not_allowed.is_empty(),
        "pairs whose outcome their label doesn't allow:\n{}",
        not_allowed.join("\n")
    );
}

/// Whether two files hold the very same audio: one audio hash or one
/// whole-file hash.
fn same_audio(a: &Seen, b: &Seen) -> bool {
    matches!((a.hashes.audio, b.hashes.audio), (Ok(x), Ok(y)) if x == y)
        || a.hashes.blake3 == b.hashes.blake3
}

/// The pair whose two files must never share a track: a "version" pair
/// that shares much audio and was relabeled after listening (E3's summary,
/// `spikes/results/e3-summary.md`).
const KEPT_APART: u32 = 42;

#[test]
fn the_grouping_job_over_every_real_file_merges_no_pair_labeled_other_than_duplicate_and_keeps_pair_42_apart(
) {
    let Some(run) = run() else { return };
    // PRIVACY: `rel_path` below holds real file names and `raw_tags` real
    // tags, inside a temporary database. Nothing in this test prints a
    // `rel_path`, a tag or a reason's fields, calls `Db::dump`, or puts one
    // in an assertion message: output is ids, labels, outcomes, states and
    // reason codes.
    let db = support::db();
    let ids: Vec<i64> = run
        .files
        .iter()
        .enumerate()
        .map(|(i, file)| {
            db.add_real(RealFile {
                rel_path: &format!("{i}/{}", file.file_name),
                audio: file.hashes.audio.ok().map(|audio| audio.to_bytes()),
                blake3: file.hashes.blake3,
                raw_tags: file.raw_tags.as_deref(),
                fingerprint: &file.fingerprint,
            })
        })
        .collect();
    // The steps of the app's chain: the quick grouping pass, the matching
    // pass, the deciding job.
    let (decided, _) = db.decide();
    println!(
        "{} files in the database; the deciding job asked the rules about {} pairs, compared {} \
         directly, held {} sets waiting",
        ids.len(),
        decided.pairs,
        decided.compared_directly,
        db.count("SELECT count(*) FROM grouping_pair WHERE state = 'waiting'"),
    );

    let (mut duplicates, mut duplicates_merged) = (0, 0);
    let (mut others, mut false_merges) = (0, Vec::<String>::new());
    let (mut held, mut waiting) = (Vec::<u32>::new(), Vec::<u32>::new());
    let mut kept_apart = None;
    for (i, pair) in run.pairs.iter().enumerate() {
        let [a, b] = run.of_pair[i].map(|file| ids[file]);
        let shared = same_track(&db, a, b);
        let stands = stands_as(&db, a, b);
        println!(
            "#{:<3}{:<10} {:<10} {stands}",
            pair.id,
            pair.category.as_str(),
            if shared { "one track" } else { "apart" },
        );
        match db.pair(a, b).map(|(_, state)| state).as_deref() {
            Some("set_disagrees") => held.push(pair.id),
            Some("waiting") => waiting.push(pair.id),
            _ => {}
        }
        if pair.id == KEPT_APART {
            kept_apart = Some(shared);
        }
        if pair.category == Category::Duplicate {
            duplicates += 1;
            duplicates_merged += usize::from(shared);
        } else {
            others += 1;
            if shared {
                false_merges.push(format!(
                    "pair {} ({}), files {a} and {b}: {stands}",
                    pair.id,
                    pair.category.as_str()
                ));
            }
        }
    }

    // Two files of different pairs in one track, unless they are the same
    // audio. The labels say nothing about these, so they are listed, not
    // asserted.
    let mut across: Vec<String> = Vec::new();
    for (p, first) in run.pairs.iter().enumerate() {
        for (q, second) in run.pairs.iter().enumerate().skip(p + 1) {
            for &x in &run.of_pair[p] {
                for &y in &run.of_pair[q] {
                    if x != y
                        && same_track(&db, ids[x], ids[y])
                        && !same_audio(&run.files[x], &run.files[y])
                    {
                        across.push(format!(
                            "pairs {} and {}, files {} and {}: {}",
                            first.id,
                            second.id,
                            ids[x],
                            ids[y],
                            stands_as(&db, ids[x], ids[y])
                        ));
                    }
                }
            }
        }
    }

    println!(
        "false merges {}/{} pairs not labeled duplicate ({:.1}%)",
        false_merges.len(),
        others,
        100.0 * false_merges.len() as f64 / others.max(1) as f64
    );
    println!("duplicates in one track: {duplicates_merged}/{duplicates}");
    println!("pair {KEPT_APART} in one track: {kept_apart:?}");
    println!("sets held, set_disagrees: pairs {held:?}");
    println!("waiting: pairs {waiting:?}");
    println!(
        "across pairs, files in one track without the same audio: {}",
        across.len()
    );
    for line in &across {
        println!("  {line}");
    }

    assert!(
        false_merges.is_empty(),
        "{} of {others} pairs not labeled duplicate share a track (the criterion is under 1%, \
         which with this many pairs is none):\n{}",
        false_merges.len(),
        false_merges.join("\n")
    );
    assert_eq!(
        kept_apart,
        Some(false),
        "pair {KEPT_APART} must be in the corpus, and its files in two tracks"
    );
}

#[test]
fn the_pair_list_reader_keeps_commas_quotes_and_line_breaks_inside_quoted_fields() {
    let rows =
        table("id,a_title,b_title\r\n1,\"Moth, Pt. 2\",\"The \"\"Moth\"\"\nLive\"\r\n2,plain,\n");
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["a_title"], "Moth, Pt. 2");
    assert_eq!(rows[0]["b_title"], "The \"Moth\"\nLive");
    assert_eq!(
        (rows[1]["id"].as_str(), rows[1]["b_title"].as_str()),
        ("2", "")
    );
    // A byte-order mark in front (a spreadsheet's export) isn't part of the
    // first column's name.
    assert_eq!(table("\u{feff}id,label\n7,same\n")[0]["id"], "7");
}

#[test]
fn an_unset_variable_skips_the_run_and_a_set_one_naming_a_missing_folder_fails_it() {
    assert_eq!(private_dir(None), Ok(None));
    let here = tempfile::tempdir().unwrap();
    assert_eq!(
        private_dir(Some(here.path().into())),
        Ok(Some(here.path().to_path_buf()))
    );
    let gone = here.path().join("not-there");
    let refused = private_dir(Some(gone.clone().into())).unwrap_err();
    assert!(refused.contains(DIR_VAR), "{refused}");
    // The message never repeats the folder: a path can hold a user name.
    assert!(!refused.contains("not-there"), "{refused}");
}

#[test]
fn a_label_becomes_a_category_and_only_a_version_can_be_a_cut() {
    assert_eq!(category("same", false), Some(Category::Duplicate));
    assert_eq!(category("same", true), Some(Category::Duplicate));
    assert_eq!(category("different", false), Some(Category::Different));
    assert_eq!(category("version", true), Some(Category::Cut));
    assert_eq!(category("version", false), Some(Category::Rework));
    assert_eq!(category("maybe", false), None);
}
