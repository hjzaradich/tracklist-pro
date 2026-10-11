//! The local corpus (1bA-3): E3's 43 hand-labeled pairs of real files, run
//! through the app's own fingerprinting and matching pass and held to what
//! each label requires ([`super::labels::unmet`]).
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
//! - `e3/pairs.csv`: each pair's id, its two audio files and their titles;
//! - `results-private/e3-labels.csv`: each pair's label (same, version or
//!   different);
//! - `results-private/e3-fingerprints.jsonl`: which versions are cuts, and
//!   the numbers E3's own tool measured, printed beside ours.
//!
//! Every audio file is opened read-only and fingerprinted by the app's
//! decoder; the fingerprints go into a throwaway database under made-up
//! file names, and the real matching pass runs over it.
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

use std::collections::{BTreeMap, HashMap};
use std::ffi::OsString;
use std::fs::File;
use std::io::{ErrorKind, Write as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use crate::fingerprint::decode::{self, Stopped};
use crate::fingerprint::Fingerprint;
use crate::matching::{compare, item_seconds, refresh, Comparison};
use crate::versions::{self, Outcome};

use super::labels::{identical_share, same_length, shared_keys, unmet, Category, Shape};
use super::passes::db;

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
    /// Each file's title as E3 listed it: its title tag, or its file name
    /// without the extension where it has no tag.
    titles: [String; 2],
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
                titles: [field("a_title"), field("b_title")],
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

/// Fingerprints every file, a few at a time.
fn fingerprints(paths: &[&Path]) -> Vec<Result<Fingerprint, &'static str>> {
    let threads = std::thread::available_parallelism().map_or(2, |n| n.get().min(4));
    let next = AtomicUsize::new(0);
    let done: Vec<Mutex<Option<Result<Fingerprint, &'static str>>>> =
        paths.iter().map(|_| Mutex::new(None)).collect();
    std::thread::scope(|scope| {
        for _ in 0..threads {
            scope.spawn(|| loop {
                let i = next.fetch_add(1, Ordering::Relaxed);
                let Some(path) = paths.get(i) else { break };
                *done[i].lock().unwrap() = Some(fingerprint(path));
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

/// What the version check says about the pair, read two ways: from the
/// title tags (a file without one gives its file name), and from the two
/// file names.
fn names_say(pair: &Pair) -> (Outcome, Outcome) {
    let tagged = |side: usize| {
        let stem = pair.paths[side].file_stem().map(|s| s.to_string_lossy());
        match stem {
            Some(stem) if stem != pair.titles[side].as_str() => {
                versions::parse_title(&pair.titles[side])
            }
            _ => versions::parse_file_name(&file_name(&pair.paths[side])),
        }
    };
    let named = |side: usize| versions::parse_file_name(&file_name(&pair.paths[side]));
    (
        versions::compare(&tagged(0), &tagged(1)).outcome,
        versions::compare(&named(0), &named(1)).outcome,
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

#[test]
fn every_labeled_pair_from_e3_measures_as_its_label_requires() {
    let dir = match private_dir(std::env::var_os(DIR_VAR)) {
        Ok(Some(dir)) => dir,
        Ok(None) => return skipped(&format!("{DIR_VAR} is not set")),
        Err(why) => panic!("{why}"),
    };
    let pairs = load(&dir);
    assert!(!pairs.is_empty(), "{PAIRS} lists no pairs");

    // Every file once, whichever pairs it's in.
    let mut files: Vec<&Path> = pairs
        .iter()
        .flat_map(|p| p.paths.iter().map(PathBuf::as_path))
        .collect();
    files.sort_unstable();
    files.dedup();
    let prints = fingerprints(&files);
    let print_of = |path: &Path| &prints[files.binary_search(&path).expect("listed above")];

    // A set variable means "run it": a pair that can't be measured fails
    // the run, so the corpus never shrinks quietly.
    let mut unmeasured = Vec::new();
    for pair in &pairs {
        for (side, path) in ["A", "B"].iter().zip(&pair.paths) {
            if let Err(why) = print_of(path) {
                unmeasured.push(format!("pair {} file {side}: {why}", pair.id));
            }
        }
    }
    assert!(
        unmeasured.is_empty(),
        "these files couldn't be fingerprinted:\n{}",
        unmeasured.join("\n")
    );

    // The real pass, over a database holding the fingerprints under
    // made-up names.
    let db = db();
    let ids: Vec<i64> = prints
        .iter()
        .enumerate()
        .map(|(i, print)| db.add(&format!("{i}.audio"), print.as_ref().unwrap()))
        .collect();
    let id_of = |path: &Path| ids[files.binary_search(&path).expect("listed above")];
    let summary = refresh(&db.writer).unwrap();
    println!(
        "{} pairs, {} files; the pass proposed {} pairs and stored {}",
        pairs.len(),
        files.len(),
        summary.candidates,
        summary.stored
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
    let mut labeled: Vec<(i64, i64)> = Vec::new();
    let (mut sharing, mut sharing_proposed) = (0, 0);
    for pair in &pairs {
        let (a, b) = (
            print_of(&pair.paths[0]).as_ref().unwrap(),
            print_of(&pair.paths[1]).as_ref().unwrap(),
        );
        let (id_a, id_b) = (id_of(&pair.paths[0]), id_of(&pair.paths[1]));
        labeled.push((id_a.min(id_b), id_a.max(id_b)));
        // What the pass stored is what grouping will read. A pair it never
        // proposed is compared here, to show what it would have found.
        let stored: Option<Comparison> = db.of_pair(id_a, id_b);
        let proposed = stored.is_some();
        let c = stored.unwrap_or_else(|| compare(a, b).expect("one fingerprint version"));
        if Shape::of(&c) == Shape::Partial {
            sharing += 1;
            sharing_proposed += usize::from(proposed);
        }

        let mut wrong: Vec<&str> = unmet(pair.category, &c, proposed);
        let (by_tags, by_files) = names_say(pair);
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

    // Each song was picked for one pair only, so little should line up
    // across pairs. Printed for the record.
    let across: Vec<_> = db
        .all()
        .into_iter()
        .filter(|m| !labeled.contains(&(m.file_a, m.file_b)))
        .collect();
    println!(
        "across pairs: {} proposed, {} of them sharing audio, {} passing the duplicate rule",
        across.len(),
        across
            .iter()
            .filter(|m| !m.comparison.segments.is_empty())
            .count(),
        across
            .iter()
            .filter(|m| super::labels::is_duplicate(&m.comparison))
            .count(),
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
