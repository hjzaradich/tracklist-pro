//! 1bA-13: what fingerprint matching costs in memory and time, on made-up
//! libraries of 10,000 and 100,000 files.
//!
//! Its own test binary, so the counting allocator and the process's own
//! counters see only this. The fingerprints are made up in code (no audio,
//! no real music): six-minute tracks of about 2,900 items, nearly all of
//! them distinct, which is the most keys a track can have. Real
//! fingerprints repeat items, so real libraries sit below these numbers.
//!
//! Of every 100 tracks, three have a re-encoded copy, one has a cut, and
//! every second hundred has a copy at the duplicate rule's limit, as in
//! the index's own scale test.
//!
//! Each measurement is an ignored test. Run them one at a time, in a
//! release build, when the machine is quiet (the process's peak counters
//! never go down, so two in one process would share a peak):
//!
//! ```text
//! cargo test --release --test matching_memory -- --ignored --nocapture --exact index_alone_100k
//! cargo test --release --test matching_memory -- --ignored --nocapture --exact passes_100k
//! ```
//!
//! and the same with `10k`. `index_alone_*` builds the block index and
//! nothing else, then compacts it as a pass does. `passes_*` puts the library in a database and runs the
//! real pass: a first one, one after 100 new files, and one by a new
//! matcher (what a pass costs when the index isn't kept).
//!
//! **`a_small_library_goes_through_the_same_steps`** is the smoke test that
//! runs in CI: the same harness on 300 short tracks, checking what was
//! found rather than how long it took, so the measurement can't rot.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use rusqlite::params;
use rusty_chromaprint::FingerprintCompressor;
use tracklist_pro_lib::db::{DbError, Writer, DB_FILE_NAME};
use tracklist_pro_lib::fingerprint::{stored, Fingerprint};
use tracklist_pro_lib::matching::{BlockIndex, Matcher, Summary};
use tracklist_pro_lib::write_guard::WriteGuard;

/// Counts live heap bytes and their peak.
struct Counting;

static CURRENT: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

fn grew(by: usize) {
    let now = CURRENT.fetch_add(by, Ordering::Relaxed) + by;
    PEAK.fetch_max(now, Ordering::Relaxed);
}

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            grew(layout.size());
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) };
        CURRENT.fetch_sub(layout.size(), Ordering::Relaxed);
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let new = unsafe { System.realloc(ptr, layout, new_size) };
        if !new.is_null() {
            if new_size >= layout.size() {
                grew(new_size - layout.size());
            } else {
                CURRENT.fetch_sub(layout.size() - new_size, Ordering::Relaxed);
            }
        }
        new
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

/// The heap's live bytes and their peak while `work` ran, both counted
/// from where the heap stood before it. SQLite's own memory isn't in
/// these (it doesn't use Rust's allocator); the process's counters
/// ([`process_memory`]) hold everything.
fn heap_while<T>(work: impl FnOnce() -> T) -> (T, Heap) {
    let base = CURRENT.load(Ordering::SeqCst);
    PEAK.store(base, Ordering::SeqCst);
    let start = Instant::now();
    let out = work();
    let heap = Heap {
        took: start.elapsed(),
        peak: PEAK.load(Ordering::SeqCst).saturating_sub(base),
        kept: CURRENT.load(Ordering::SeqCst).saturating_sub(base),
    };
    (out, heap)
}

/// What one step cost.
#[derive(Debug, Clone, Copy)]
struct Heap {
    took: Duration,
    /// The most the heap held above where it started.
    peak: usize,
    /// What it still held when the step was over.
    kept: usize,
}

/// This process's peak working set and peak committed memory, in bytes.
#[cfg(windows)]
fn process_memory() -> Option<(usize, usize)> {
    use windows_sys::Win32::System::ProcessStatus::{
        K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS,
    };
    use windows_sys::Win32::System::Threading::GetCurrentProcess;

    let mut counters = PROCESS_MEMORY_COUNTERS {
        cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
        PageFaultCount: 0,
        PeakWorkingSetSize: 0,
        WorkingSetSize: 0,
        QuotaPeakPagedPoolUsage: 0,
        QuotaPagedPoolUsage: 0,
        QuotaPeakNonPagedPoolUsage: 0,
        QuotaNonPagedPoolUsage: 0,
        PagefileUsage: 0,
        PeakPagefileUsage: 0,
    };
    // SAFETY: the current process's pseudo-handle is always valid, and the
    // struct and its size are what the call expects.
    let ok = unsafe { K32GetProcessMemoryInfo(GetCurrentProcess(), &mut counters, counters.cb) };
    (ok != 0).then_some((counters.PeakWorkingSetSize, counters.PeakPagefileUsage))
}

#[cfg(not(windows))]
fn process_memory() -> Option<(usize, usize)> {
    None
}

/// splitmix64: a seeded, platform-independent random source.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

/// The items of made-up track `seed`. Each item differs from the one
/// before by one to three bits, as neighbouring items of real audio do, so
/// the stored blob is a realistic size; nearly all are distinct.
fn track(seed: u64, len: usize) -> Vec<u32> {
    let mut rng = Rng(seed.wrapping_mul(0xD6E8_FEB8_6659_FD93) ^ 0x7AC4);
    let mut item = rng.next() as u32;
    (0..len)
        .map(|_| {
            for _ in 0..=rng.below(3) {
                item ^= 1 << rng.below(32);
            }
            item
        })
        .collect()
}

/// `items` as another encoding would give them: one item in `keep` stays
/// bit-identical and the rest get 1 to 4 bits flipped.
fn reencoded(items: &[u32], seed: u64, keep: u64) -> Vec<u32> {
    let mut rng = Rng(seed ^ 0xE4C0);
    items
        .iter()
        .map(|&item| {
            if rng.below(keep) == 0 {
                return item;
            }
            let mut noisy = item;
            for _ in 0..=rng.below(4) {
                noisy ^= 1 << rng.below(32);
            }
            noisy
        })
        .collect()
}

/// How a made-up file relates to the one before it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// A track of its own.
    Original,
    /// A re-encode or a cut of the file before it: the index promises to
    /// propose the pair.
    Strong,
    /// A copy of the file before it at the duplicate rule's limit.
    Weak,
}

/// The fingerprints of a made-up library of `n` files, `len` items a
/// track, in file order: every track starts with the same "silence".
struct Library {
    n: usize,
    len: usize,
    made: usize,
    seed: u64,
    /// The copy waiting to follow the original just handed out.
    pending: Option<(Kind, Vec<u32>)>,
    silence: Vec<u32>,
}

impl Library {
    fn new(n: usize, len: usize) -> Library {
        Library {
            n,
            len,
            made: 0,
            seed: 0,
            pending: None,
            silence: track(u64::MAX, 20),
        }
    }

    fn with_silence(&self, items: &[u32]) -> Vec<u32> {
        [self.silence.as_slice(), items].concat()
    }
}

impl Iterator for Library {
    type Item = (Kind, Vec<u32>);

    fn next(&mut self) -> Option<(Kind, Vec<u32>)> {
        if self.made >= self.n {
            return None;
        }
        self.made += 1;
        if let Some(copy) = self.pending.take() {
            return Some(copy);
        }
        self.seed += 1;
        let (seed, len) = (self.seed, self.len);
        let items = track(seed, len);
        self.pending = match seed % 100 {
            1..=3 => Some((Kind::Strong, reencoded(&items, seed, 7))),
            4 => Some((
                Kind::Strong,
                reencoded(&items[len / 4..len * 3 / 4], seed, 7),
            )),
            5 if (seed / 100).is_multiple_of(2) => Some((Kind::Weak, reencoded(&items, seed, 40))),
            _ => None,
        }
        .map(|(kind, copy)| (kind, self.with_silence(&copy)));
        Some((Kind::Original, self.with_silence(&items)))
    }
}

/// The bytes `file.fingerprint` would hold for `items`.
fn blob(items: &[u32]) -> Vec<u8> {
    let config = stored::config();
    let mut blob = b"TLFP".to_vec();
    blob.push(stored::VERSION);
    blob.extend_from_slice(&FingerprintCompressor::from(&config).compress(items));
    blob
}

/// What building the index alone measured.
struct IndexReport {
    files: usize,
    /// Making the fingerprints up, without indexing them: part of `build`.
    making: Duration,
    postings: usize,
    build: Heap,
    /// Giving back the buckets' spare room afterwards, as a pass does:
    /// how long it took and what the index holds then.
    compact: Duration,
    kept: usize,
}

/// Builds the block index over a library of `n` files and nothing else.
fn index_alone(n: usize, len: usize) -> (IndexReport, BlockIndex) {
    let start = Instant::now();
    let items: usize = Library::new(n, len).map(|(_, items)| items.len()).sum();
    assert!(items >= n * len / 2);
    let making = start.elapsed();
    let (mut index, build) = heap_while(|| {
        let mut index = BlockIndex::new();
        for (entry, (_, items)) in Library::new(n, len).enumerate() {
            index.insert(entry as u32, &items);
        }
        index
    });
    let before = CURRENT.load(Ordering::SeqCst);
    let start = Instant::now();
    index.compact();
    let compact = start.elapsed();
    let given_back = before.saturating_sub(CURRENT.load(Ordering::SeqCst));
    let report = IndexReport {
        files: n,
        making,
        postings: index.postings(),
        build,
        compact,
        kept: build.kept.saturating_sub(given_back),
    };
    (report, index)
}

/// A migrated database in a temp folder with one music folder.
struct Db {
    writer: Writer,
    files: i64,
    _dir: tempfile::TempDir,
}

impl Db {
    fn new() -> Db {
        let dir = tempfile::Builder::new()
            .prefix("tlp-matching-")
            .tempdir()
            .unwrap();
        let guard = WriteGuard::app_data(dir.path()).unwrap();
        let path = guard.check(&dir.path().join(DB_FILE_NAME)).unwrap();
        let writer = Writer::open(&path).unwrap();
        writer
            .call(|c| {
                c.execute_batch(
                    "INSERT INTO volume (identity, kind) VALUES ('serial=NTFS-1A2B3C4D', 'external');
                     INSERT INTO music_folder (volume_id, rel_path, rel_path_key) VALUES (1, 'M', 'M');",
                )
            })
            .unwrap();
        Db {
            writer,
            files: 0,
            _dir: dir,
        }
    }

    /// Adds a present file for each fingerprint. Returns their ids.
    fn add(&mut self, fingerprints: impl Iterator<Item = Vec<u32>>) -> Vec<i64> {
        let mut ids = Vec::new();
        let mut fingerprints = fingerprints.peekable();
        while fingerprints.peek().is_some() {
            let first = self.files;
            let blobs: Vec<Vec<u8>> = fingerprints.by_ref().take(1000).map(|i| blob(&i)).collect();
            self.files += blobs.len() as i64;
            let added = self
                .writer
                .call(move |c| {
                    let tx = c.transaction()?;
                    let mut ids = Vec::new();
                    for (i, blob) in blobs.iter().enumerate() {
                        let name = format!("{:07}.flac", first + i as i64);
                        tx.execute(
                            "INSERT INTO file (music_folder_id, rel_path, rel_path_key, size, mtime, fingerprint)
                             VALUES (1, ?1, ?1, 1000, 1000, ?2)",
                            params![name, blob],
                        )?;
                        ids.push(tx.last_insert_rowid());
                    }
                    tx.commit()?;
                    Ok(ids)
                })
                .unwrap();
            ids.extend(added);
        }
        ids
    }

    /// Whether files `a` and `b` have a stored result.
    fn compared(&self, a: i64, b: i64) -> bool {
        self.writer
            .call(move |c| {
                c.query_row(
                    "SELECT COUNT(*) FROM fingerprint_match WHERE file_a = ?1 AND file_b = ?2",
                    [a.min(b), a.max(b)],
                    |r| r.get::<_, i64>(0),
                )
            })
            .unwrap()
            == 1
    }

    fn rows(&self) -> i64 {
        self.writer
            .call(|c| c.query_row("SELECT COUNT(*) FROM fingerprint_match", [], |r| r.get(0)))
            .unwrap()
    }
}

/// One pass, and how long its first part took: reading every fingerprint
/// and bringing the index up to date. The rest is finding candidates and
/// comparing them.
fn pass(matcher: &Matcher, db: &Db) -> (Summary, Heap, Duration) {
    // The pass reports once per page of fingerprints it reads, once more
    // when there are none left, and then once per fingerprint it works on.
    let reading = db.files as usize / PAGE + 2;
    let start = Instant::now();
    let (mut reports, mut read) = (0, Duration::ZERO);
    let (summary, heap) = heap_while(|| {
        matcher
            .pass::<DbError>(&db.writer, &mut |_| {
                reports += 1;
                if reports == reading {
                    read = start.elapsed();
                }
                Ok(())
            })
            .unwrap()
    });
    (summary, heap, read)
}

/// How many fingerprints the pass reads per query (`matching::job`).
const PAGE: usize = 500;

/// How many files arrive between the first pass and the later one.
const NEW_FILES: usize = 100;

/// What the passes over one library measured.
struct PassReport {
    files: usize,
    /// Pairs planted in the library that the index promises to find, as
    /// file ids, and those at the rule's limit.
    strong: Vec<(i64, i64)>,
    weak: Vec<(i64, i64)>,
    first: (Summary, Heap, Duration),
    /// After [`NEW_FILES`] more files: half of them new tracks, half
    /// re-encodes of tracks already there (`new_copies`).
    later: (Summary, Heap, Duration),
    new_copies: Vec<(i64, i64)>,
    /// The same library by a matcher that kept nothing: every fingerprint
    /// is read into a new index, and every pair already has its result.
    restarted: (Summary, Heap, Duration),
    rows: i64,
    db: Db,
}

/// Puts a library of `n` files in a database and runs the passes.
fn passes(n: usize, len: usize) -> PassReport {
    let mut db = Db::new();
    let mut kinds = Vec::new();
    let library = Library::new(n, len).map(|(kind, items)| {
        kinds.push(kind);
        items
    });
    let ids = db.add(library);
    let planted = |wanted: Kind| -> Vec<(i64, i64)> {
        (1..ids.len())
            .filter(|&i| kinds[i] == wanted)
            .map(|i| (ids[i - 1], ids[i]))
            .collect()
    };
    let (strong, weak) = (planted(Kind::Strong), planted(Kind::Weak));

    let matcher = Matcher::new();
    let first = pass(&matcher, &db);

    // Half new tracks, half re-encodes of the library's first tracks
    // (seeds 10, 20, …: tracks with no copy yet).
    let silence = track(u64::MAX, 20);
    let with_silence = |items: &[u32]| [silence.as_slice(), items].concat();
    let copied: Vec<u64> = (1..=NEW_FILES as u64 / 2).map(|i| i * 10).collect();
    let new = (0..NEW_FILES as u64).map(|i| match copied.get(i as usize) {
        Some(&seed) => with_silence(&reencoded(&track(seed, len), seed ^ 0x51, 7)),
        None => with_silence(&track(u64::MAX / 2 + i, len)),
    });
    let new_ids = db.add(new);
    // Track `seed` is the file made when the library's seed counter stood
    // at `seed`: find it by counting originals.
    let originals: Vec<i64> = (0..ids.len())
        .filter(|&i| kinds[i] == Kind::Original)
        .map(|i| ids[i])
        .collect();
    let new_copies: Vec<(i64, i64)> = copied
        .iter()
        .zip(&new_ids)
        .filter_map(|(&seed, &copy)| Some((*originals.get(seed as usize - 1)?, copy)))
        .collect();
    let later = pass(&matcher, &db);
    drop(matcher);

    let restarted = pass(&Matcher::new(), &db);
    PassReport {
        files: n,
        strong,
        weak,
        first,
        later,
        new_copies,
        restarted,
        rows: db.rows(),
        db,
    }
}

fn mib(bytes: usize) -> String {
    format!("{:7.1} MiB", bytes as f64 / (1024.0 * 1024.0))
}

fn secs(d: Duration) -> String {
    format!("{:8.2} s", d.as_secs_f64())
}

fn print_process() {
    if let Some((working_set, committed)) = process_memory() {
        println!(
            "  the process, everything in it: peak working set {}, peak committed {}",
            mib(working_set),
            mib(committed)
        );
    }
    println!("  release build: {}", !cfg!(debug_assertions));
    println!();
}

fn print_index(report: &IndexReport) {
    println!();
    println!(
        "matching_memory: the block index alone, {} files",
        report.files
    );
    println!(
        "  build      {} (of which making the fingerprints up {})   heap peak {}   kept {}   {} postings ({:.1} bytes each kept)",
        secs(report.build.took),
        secs(report.making),
        mib(report.build.peak),
        mib(report.build.kept),
        report.postings,
        report.build.kept as f64 / report.postings.max(1) as f64
    );
    println!(
        "  compact    {}   kept {} ({:.1} bytes a posting)",
        secs(report.compact),
        mib(report.kept),
        report.kept as f64 / report.postings.max(1) as f64
    );
    print_process();
}

fn print_passes(report: &PassReport) {
    println!();
    println!(
        "matching_memory: passes over {} files in a database",
        report.files
    );
    for (name, (summary, heap, read)) in [
        ("first pass", &report.first),
        ("later pass", &report.later),
        ("new matcher", &report.restarted),
    ] {
        println!(
            "  {name:<11}{} (reading and indexing {})   heap peak {}   kept {}   {} changed, {} candidates, {} compared, {} stored",
            secs(heap.took),
            secs(*read),
            mib(heap.peak),
            mib(heap.kept),
            summary.changed,
            summary.candidates,
            summary.compared,
            summary.stored
        );
    }
    let found = |pairs: &[(i64, i64)]| {
        pairs
            .iter()
            .filter(|&&(a, b)| report.db.compared(a, b))
            .count()
    };
    println!(
        "  found: {} of {} planted pairs, {} of {} at the rule's limit, {} of {} copies among the new files; {} rows",
        found(&report.strong),
        report.strong.len(),
        found(&report.weak),
        report.weak.len(),
        found(&report.new_copies),
        report.new_copies.len(),
        report.rows
    );
    print_process();
}

/// Six minutes of audio in fingerprint items.
const SIX_MINUTES: usize = 2_900;

#[test]
#[ignore = "a measurement: run alone, in a release build; see the module docs"]
fn index_alone_10k() {
    print_index(&index_alone(10_000, SIX_MINUTES).0);
}

#[test]
#[ignore = "a measurement: run alone, in a release build; see the module docs"]
fn index_alone_100k() {
    print_index(&index_alone(100_000, SIX_MINUTES).0);
}

#[test]
#[ignore = "a measurement: run alone, in a release build; see the module docs"]
fn passes_10k() {
    print_passes(&passes(10_000, SIX_MINUTES));
}

#[test]
#[ignore = "a measurement: run alone, in a release build; see the module docs"]
fn passes_100k() {
    print_passes(&passes(100_000, SIX_MINUTES));
}

/// The smoke test: the harness on a small library, checking what it finds.
#[test]
fn a_small_library_goes_through_the_same_steps() {
    const FILES: usize = 300;
    const ITEMS: usize = 600;

    // The made-up blobs are ones the app reads back as the same items.
    let items = track(1, ITEMS);
    assert_eq!(
        Fingerprint::from_blob(&blob(&items)).unwrap().items(),
        items
    );

    let (report, index) = index_alone(FILES, ITEMS);
    print_index(&report);
    assert_eq!(index.len(), FILES);
    assert!(report.postings > 0);

    let report = print_and_return(passes(FILES, ITEMS));
    let db = &report.db;
    assert!(!report.strong.is_empty() && !report.new_copies.is_empty());
    for &(a, b) in report.strong.iter().chain(&report.new_copies) {
        assert!(db.compared(a, b), "files {a} and {b} were never compared");
    }
    let (first, later, restarted) = (report.first.0, report.later.0, report.restarted.0);
    assert_eq!(first.files, FILES as u64);
    assert_eq!(first.changed, FILES as u64);
    assert!(first.compared >= report.strong.len() as u64);
    // The later pass works on the new files alone.
    assert_eq!(later.files, (FILES + NEW_FILES) as u64);
    assert_eq!(later.changed, NEW_FILES as u64);
    assert!(later.compared >= report.new_copies.len() as u64);
    assert!(later.compared <= NEW_FILES as u64);
    // A new matcher reads everything again and finds nothing left to do.
    assert_eq!(restarted.changed, (FILES + NEW_FILES) as u64);
    assert_eq!((restarted.compared, restarted.stored), (0, 0));
    assert_eq!(report.rows, (first.stored + later.stored) as i64);
}

fn print_and_return(report: PassReport) -> PassReport {
    print_passes(&report);
    report
}
