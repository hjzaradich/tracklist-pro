//! The rekordbox XML parser on a large collection (1aA-8).
//!
//! Ignored by default: it generates a 50,000-track export (every attribute
//! rekordbox writes, a beat grid, cues, and a playlist tree) and reports how
//! long the parse takes and how much memory it peaks at. Run it with
//!
//! ```text
//! cargo test --release --test rekordbox_xml_scale -- --ignored --nocapture
//! ```
//!
//! Peak memory is measured by counting this test binary's allocations, so
//! it's the parser's own heap use, not the whole process's. Everything is
//! synthetic.
//!
//! `local_export_counts` reads a real export if one is named in the
//! `TLP_REKORDBOX_XML` environment variable, and prints counts only.

use std::alloc::{GlobalAlloc, Layout, System};
use std::fmt::Write as _;
use std::io::Write as _;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use tracklist_pro_lib::rekordbox::{EntryTarget, RekordboxXml};

struct Counting;

static CURRENT: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

// SAFETY: forwards every call to the system allocator unchanged; it only
// keeps two counters beside it.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            let now = CURRENT.fetch_add(layout.size(), Ordering::Relaxed) + layout.size();
            PEAK.fetch_max(now, Ordering::Relaxed);
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
                let grow = new_size - layout.size();
                let now = CURRENT.fetch_add(grow, Ordering::Relaxed) + grow;
                PEAK.fetch_max(now, Ordering::Relaxed);
            } else {
                CURRENT.fetch_sub(layout.size() - new_size, Ordering::Relaxed);
            }
        }
        new
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

const TRACKS: usize = 50_000;

/// A synthetic export shaped like rekordbox 7's.
fn generate(tracks: usize) -> String {
    let mut xml = String::with_capacity(tracks * 1_400);
    xml.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<DJ_PLAYLISTS Version=\"1.0.0\">\n");
    xml.push_str("  <PRODUCT Name=\"rekordbox\" Version=\"7.2.19\" Company=\"AlphaTheta\"/>\n");
    writeln!(xml, "  <COLLECTION Entries=\"{tracks}\">").unwrap();
    for i in 1..=tracks {
        let folder = i % 400;
        let key = ["8A", "9A", "Fm", "Db", "1m"][i % 5];
        writeln!(
            xml,
            "    <TRACK TrackID=\"{id}\" Name=\"Synthetic Title {i} (Extended Mix)\" \
             Artist=\"Synthetic Artist {a} &amp; Friend\" Composer=\"\" Album=\"Synthetic Album {f}\" \
             Grouping=\"\" Genre=\"Genre {g}\" Kind=\"MP3 File\" Size=\"{size}\" TotalTime=\"{t}\" \
             DiscNumber=\"0\" TrackNumber=\"{n}\" Year=\"20{y:02}\" AverageBpm=\"{bpm}.00\" \
             DateAdded=\"2026-09-{d:02}\" BitRate=\"320\" SampleRate=\"44100\" \
             Comments=\"/* My Tag */ synthetic comment {i}\" PlayCount=\"{p}\" Rating=\"{r}\" \
             Location=\"file://localhost/C:/Users/someone/Music/Folder%20{f}/Synthetic%20Artist%20{a}%20-%20Title%20{i}%20#{i}%20(Extended%20Mix).mp3\" \
             Remixer=\"\" Tonality=\"{key}\" Label=\"Label {g}\" Mix=\"Extended Mix\">\n",
            id = 100_000_000 + i,
            a = i % 3_000,
            f = folder,
            g = i % 40,
            size = 8_000_000 + i,
            t = 180 + i % 240,
            n = i % 12 + 1,
            y = i % 25,
            bpm = 120 + i % 60,
            d = i % 28 + 1,
            p = i % 30,
            r = (i % 6) * 51,
        )
        .unwrap();
        writeln!(
            xml,
            "      <TEMPO Inizio=\"0.025\" Bpm=\"{}.00\" Metro=\"4/4\" Battito=\"1\"/>",
            120 + i % 60
        )
        .unwrap();
        for c in 0..4 {
            writeln!(
                xml,
                "      <POSITION_MARK Name=\"\" Type=\"0\" Start=\"{}.500\" Num=\"{c}\" Red=\"40\" Green=\"226\" Blue=\"20\"/>",
                30 + c * 16
            )
            .unwrap();
        }
        xml.push_str("      <POSITION_MARK Name=\"\" Type=\"0\" Start=\"30.500\" Num=\"-1\"/>\n    </TRACK>\n");
    }
    xml.push_str(
        "  </COLLECTION>\n  <PLAYLISTS>\n    <NODE Type=\"0\" Name=\"ROOT\" Count=\"1\">\n",
    );
    let lists = 200;
    writeln!(
        xml,
        "      <NODE Type=\"0\" Name=\"Gigs\" Count=\"{lists}\">"
    )
    .unwrap();
    for l in 0..lists {
        writeln!(
            xml,
            "        <NODE Name=\"Playlist {l}\" Type=\"1\" KeyType=\"0\" Entries=\"100\">"
        )
        .unwrap();
        for e in 0..100 {
            let id = 100_000_000 + (l * 97 + e * 13) % tracks + 1;
            writeln!(xml, "          <TRACK Key=\"{id}\"/>").unwrap();
        }
        xml.push_str("        </NODE>\n");
    }
    xml.push_str("      </NODE>\n    </NODE>\n  </PLAYLISTS>\n</DJ_PLAYLISTS>\n");
    xml
}

#[test]
#[ignore = "slow: generates and parses a 50,000-track export; run with --ignored"]
fn a_50k_track_export_parses_and_reports_time_and_peak_memory() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("large.xml");
    {
        let xml = generate(TRACKS);
        let mut file = std::fs::File::create(&path).unwrap();
        file.write_all(xml.as_bytes()).unwrap();
    }
    let file_size = std::fs::metadata(&path).unwrap().len();

    // The floor: quick-xml alone, reading every element and attribute and
    // doing nothing with them. The parser's time over this is its own work;
    // the ratio holds up when the machine is busy, the seconds don't.
    let started = Instant::now();
    let elements = bare_read(&path);
    let floor = started.elapsed();
    assert!(elements > TRACKS * 7);

    let baseline = CURRENT.load(Ordering::Relaxed);
    PEAK.store(baseline, Ordering::Relaxed);
    let started = Instant::now();
    let read = RekordboxXml::read_file(&path).unwrap();
    let elapsed = started.elapsed();
    let peak = PEAK.load(Ordering::Relaxed) - baseline;
    let held = CURRENT.load(Ordering::Relaxed) - baseline;

    assert_eq!(read.tracks.len(), TRACKS);
    assert!(
        read.warnings.is_empty(),
        "{:?}",
        &read.warnings[..5.min(read.warnings.len())]
    );
    assert!(read
        .tracks
        .iter()
        .all(|t| t.key.is_some() && t.cues.len() == 5 && t.tempos.len() == 1));
    let lists = read.playlists.playlists();
    assert_eq!(lists.len(), 200);
    assert!(lists.iter().all(|(_, p)| p
        .entries
        .iter()
        .all(|e| matches!(e.target, EntryTarget::Track(_)))));

    let mib = |b: usize| b as f64 / (1024.0 * 1024.0);
    println!(
        "rekordbox XML: {TRACKS} tracks, {:.1} MiB file, parsed in {:.2} s \
         ({:.0} tracks/s; a bare quick-xml read takes {:.2} s, {:.1}x less); peak heap {:.1} MiB, result holds {:.1} MiB",
        file_size as f64 / (1024.0 * 1024.0),
        elapsed.as_secs_f64(),
        TRACKS as f64 / elapsed.as_secs_f64(),
        floor.as_secs_f64(),
        elapsed.as_secs_f64() / floor.as_secs_f64(),
        mib(peak),
        mib(held),
    );
    drop(read);
}

/// Reads every element and its raw attribute values with quick-xml and nothing
/// else; returns how many elements there were.
fn bare_read(path: &std::path::Path) -> usize {
    use quick_xml::events::Event;
    let file = std::io::BufReader::with_capacity(1 << 16, std::fs::File::open(path).unwrap());
    let mut reader = quick_xml::Reader::from_reader(file);
    let mut buf = Vec::new();
    let (mut elements, mut bytes) = (0, 0);
    loop {
        match reader.read_event_into(&mut buf).unwrap() {
            Event::Start(e) | Event::Empty(e) => {
                elements += 1;
                for attr in e.attributes() {
                    bytes += attr.unwrap().value.len();
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    assert!(bytes > 0);
    elements
}

/// Local only: counts from a real export, never names.
#[test]
#[ignore = "local only: needs TLP_REKORDBOX_XML pointing at a real export"]
fn local_export_counts() {
    let Some(path) = std::env::var_os("TLP_REKORDBOX_XML") else {
        println!("TLP_REKORDBOX_XML not set; nothing to read");
        return;
    };
    let started = Instant::now();
    let read = RekordboxXml::read_file(std::path::Path::new(&path)).unwrap();
    let elapsed = started.elapsed();
    let playlists = read.playlists.playlists();
    let entries: usize = playlists.iter().map(|(_, p)| p.entries.len()).sum();
    let streaming = read.tracks.iter().filter(|t| t.is_streaming()).count();
    let bad_locations = read.tracks.iter().filter(|t| t.location.is_err()).count();
    let with_key = read.tracks.iter().filter(|t| t.key.is_some()).count();
    let cues: usize = read.tracks.iter().map(|t| t.cues.len()).sum();
    let tempos: usize = read.tracks.iter().map(|t| t.tempos.len()).sum();
    println!("parsed in {:.3} s", elapsed.as_secs_f64());
    println!(
        "tracks kept {} (streaming {streaming}, undecodable Location {bad_locations}, with key {with_key}), cues {cues}, grid entries {tempos}",
        read.tracks.len()
    );
    println!("skips {:?}", read.skips);
    println!(
        "playlists {}, entries {entries}, entries without a track {}",
        playlists.len(),
        read.entries_without_track
    );
    let mut kinds = std::collections::BTreeMap::<String, usize>::new();
    for w in &read.warnings {
        let kind = format!("{:?}", w.problem);
        let kind = kind.split([' ', '{', '(']).next().unwrap_or("").to_owned();
        *kinds.entry(kind).or_default() += 1;
    }
    println!("warnings by kind {kinds:?}");
}
