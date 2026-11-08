//! Name search, Discogs, covers and the automatic Inbox look-up (ROADMAP
//! 2.2; Run 6A, S2b). The services are the scripted stand-in of
//! `online_tests`; nothing here goes online.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::arrival::arrivals_job;
use super::cover::{cover_with, is_release_id, CoverAnswer, CoverCache, MAX_BYTES, MAX_IMAGES};
use super::job::{file_statuses, Looker, LookupState};
use super::lookup::{
    look_up, Answer, Backend, Input, Live, On, Outcome, Why, DISCOGS_NAME, MAX_DISCOGS,
};
use super::names::{close, fold, names_of, Names};
use super::online_tests::Fake;
use super::*;
use crate::credentials::{Credentials, DISCOGS};
use crate::db::ReadPool;
use crate::jobs::{JobContext, JobError, JobHandler, JobKind, JobQueue, NewJob};
use crate::net::services::coverart::Artwork;
use crate::net::services::{ClientError, LookupHit};
use crate::net::{set_opt_in, Net, NetError, Service};
use crate::search::tests::support::Lib;

const ONLY_MB: On = On {
    acoustid: false,
    musicbrainz: true,
    discogs: false,
};
const ONLY_DISCOGS: On = On {
    acoustid: false,
    musicbrainz: false,
    discogs: true,
};
const SECRET: &str = "SECRET-token-for-s2b-tests-7781";

fn names() -> Names {
    Names {
        artist: "Test Artist".into(),
        title: "Test Track (Extended Mix)".into(),
    }
}

fn named() -> Input {
    Input {
        names: Some(names()),
        ..Input::default()
    }
}

fn hit(service: Service, title: &str, artist: &str) -> LookupHit {
    LookupHit {
        source: service,
        score: Some(100),
        title: Some(title.into()),
        artist: Some(artist.into()),
        album: None,
        year: None,
        isrc: None,
        genre: None,
        label: None,
        url: None,
        release_id: None,
        recording_id: Some("rec-9".into()),
        catalogue_number: None,
    }
}

fn fake_with(configure: impl FnOnce(&mut Fake)) -> Arc<Fake> {
    let mut fake = Arc::into_inner(Fake::matching()).unwrap();
    configure(&mut fake);
    Arc::new(fake)
}

fn online(service: &str, via: Option<&str>) -> IdentifySource {
    IdentifySource::Online {
        service: service.into(),
        via: via.map(str::to_owned),
    }
}

// --- What counts as the same track ---

#[test]
fn a_hit_is_close_when_title_and_artist_agree_after_folding_and_loose_when_not() {
    let want = names();
    // Case, punctuation and a bracketed version fold away.
    assert!(close(&want, "TEST TRACK!", "test artist"));
    assert!(close(&want, "Test Track", "Test Artist"));
    assert!(close(&want, "Test Track (Extended Mix)", "Test Artist"));
    assert!(close(&want, "Test-Track", "Test Artist"));
    // The artists share a name: guests and joiners split a credit.
    assert!(close(&want, "Test Track", "Test Artist feat. Someone"));
    assert!(close(&want, "Test Track", "Someone & Test Artist"));
    assert!(close(&want, "Test Track", "Someone x Test Artist"));
    // Not a whole name: "Artist" is not "Test Artist", nor is a longer name.
    assert!(!close(&want, "Test Track", "Artist"));
    assert!(!close(&want, "Test Track", "Test Artist Quartet"));
    // Both titles carry a version and they differ: another mix.
    assert!(!close(&want, "Test Track (Radio Edit)", "Test Artist"));
    // Loose: other title, other artist, a partial word.
    assert!(!close(&want, "Test Track Two", "Test Artist"));
    assert!(!close(&want, "Test", "Test Artist"));
    assert!(!close(&want, "Test Track", "Other Artist"));
    assert!(!close(&want, "Test Track", "Test Art"));
    assert!(!close(&want, "Test Track", ""));
    assert_eq!(fold("A.B. (x) [y]  C"), "a b c");
}

#[test]
fn a_file_s_names_come_from_its_tags_then_its_file_name() {
    let tagged = r#"{"id3v2":[{"key":"TIT2","value":{"type":"text","text":"Velo (Extended Mix)"}},
                     {"key":"TPE1","value":{"type":"text","text":"Kit"}}]}"#;
    let n = names_of(Some(tagged), "Music/other - name.mp3").unwrap();
    assert_eq!(
        (n.artist.as_str(), n.title.as_str()),
        ("Kit", "Velo (Extended Mix)")
    );
    let n = names_of(None, "Music/Pack/Kit - Velo.mp3").unwrap();
    assert_eq!((n.artist.as_str(), n.title.as_str()), ("Kit", "Velo"));
    // No artist anywhere: no names.
    assert_eq!(names_of(None, "Music/Velo.mp3"), None);
}

// --- Name search ---

#[test]
fn name_search_is_accepted_only_for_a_close_match() {
    let fake = fake_with(|f| {
        f.search_hits = vec![
            hit(Service::MusicBrainz, "Test Track", "Somebody Else"),
            hit(
                Service::MusicBrainz,
                "Test Track",
                "Test Artist feat. Someone",
            ),
        ];
    });
    let answer = look_up(&fake, ONLY_MB, &named());
    assert_eq!(answer.outcome, Outcome::Found);
    let calls = fake.calls();
    assert!(calls[0].starts_with("search recording:\"Test Track\" AND artist:\"Test Artist\""));
    // The same recording and release steps as after AcoustID.
    assert_eq!(&calls[1], "recording rec-9");
    assert!(calls[2].starts_with("release "));
    assert!(!calls.iter().any(|c| c == "acoustid"));
    // "MusicBrainz", with no service leading to it.
    assert!(!answer.candidates.is_empty());
    for c in &answer.candidates {
        assert_eq!(c.source, online("MusicBrainz", None));
    }
    assert!(!answer.releases.is_empty());
}

#[test]
fn name_search_rejects_a_loose_match_and_asks_nothing_more() {
    let fake = fake_with(|f| {
        f.search_hits = vec![
            hit(Service::MusicBrainz, "Test Track Two", "Test Artist"),
            hit(Service::MusicBrainz, "Test Track", "Test Art"),
        ];
    });
    let answer = look_up(&fake, ONLY_MB, &named());
    assert_eq!(
        (answer.outcome, answer.why),
        (Outcome::Nothing, Some(Why::NoMatch))
    );
    assert_eq!(fake.calls().len(), 1, "only the search: {:?}", fake.calls());
}

#[test]
fn name_search_is_not_used_when_acoustid_found_a_match() {
    let fake = fake_with(|f| {
        f.search_hits = vec![hit(Service::MusicBrainz, "Test Track", "Test Artist")];
    });
    let on = On {
        acoustid: true,
        musicbrainz: true,
        discogs: false,
    };
    let input = Input {
        fingerprint: Some(vec![1, 2, 3]),
        duration_ms: Some(215_400),
        ..named()
    };
    let answer = look_up(&fake, on, &input);
    assert_eq!(answer.outcome, Outcome::Found);
    assert!(!fake.calls().iter().any(|c| c.starts_with("search")));
    for c in &answer.candidates {
        assert_eq!(c.source, online("MusicBrainz", Some("AcoustID")));
    }
}

#[test]
fn name_search_runs_when_acoustid_found_nothing_or_was_skipped() {
    let on = On {
        acoustid: true,
        musicbrainz: true,
        discogs: false,
    };
    let with_print = Input {
        fingerprint: Some(vec![1]),
        duration_ms: Some(1000),
        ..named()
    };
    // AcoustID answers with nothing.
    let nothing = Fake::new(Box::new(|_| Ok(Vec::new())));
    let _ = look_up(&nothing, on, &with_print);
    assert_eq!(nothing.calls()[0], "acoustid");
    assert!(nothing.calls()[1].starts_with("search "));
    // AcoustID is skipped: no fingerprint.
    let skipped = Fake::matching();
    let _ = look_up(&skipped, on, &named());
    assert!(skipped.calls()[0].starts_with("search "));
}

#[test]
fn a_file_with_no_artist_or_title_is_skipped_by_the_name_search() {
    let fake = Fake::matching();
    let answer = look_up(&fake, ONLY_MB, &Input::default());
    assert_eq!(
        (answer.outcome, answer.why),
        (Outcome::Skipped, Some(Why::NoName))
    );
    assert!(fake.calls().is_empty());
}

// --- Discogs ---

#[test]
fn discogs_is_skipped_without_a_token_and_asks_nothing() {
    let fake = Fake::matching();
    let answer = look_up(&fake, ONLY_DISCOGS, &named());
    assert_eq!(
        (answer.outcome, answer.why),
        (Outcome::Skipped, Some(Why::NoToken))
    );
    assert!(fake.calls().is_empty());
}

#[test]
fn discogs_candidates_come_from_releases_containing_the_track_whatever_the_release_is_called() {
    // A release that contains the track: its own title is the album's, not the track's.
    let mut album = hit(Service::Discogs, "Test Album", "Test Artist");
    album.label = Some("Test Label".into());
    album.year = Some(2003);
    album.catalogue_number = Some("TL-001".into());
    // By another artist (a compilation, a namesake): not taken.
    let mut other = hit(Service::Discogs, "Test Track", "Somebody Else");
    other.label = Some("Wrong Label".into());
    let mut various = hit(Service::Discogs, "Hits", "Various");
    various.label = Some("Wrong Label".into());
    // The artist with a guest, and a year that isn't four digits.
    let mut guest = hit(
        Service::Discogs,
        "Second Album",
        "Test Artist feat. Someone",
    );
    guest.year = Some(99);
    let fake = fake_with(|f| {
        f.token_saved = true;
        f.discogs_hits = vec![other, album, various, guest];
    });
    let answer = look_up(&fake, ONLY_DISCOGS, &named());
    assert_eq!(answer.outcome, Outcome::Found);
    let got: Vec<(IdentifyField, &str)> = answer
        .candidates
        .iter()
        .map(|c| (c.field, c.value.as_str()))
        .collect();
    assert_eq!(
        got,
        [
            (IdentifyField::Album, "Test Album"),
            (IdentifyField::Label, "Test Label"),
            (IdentifyField::CatalogueNumber, "TL-001"),
            (IdentifyField::Year, "2003"),
            (IdentifyField::Album, "Second Album"),
        ]
    );
    // Only Album, Label, Year and catalogue number: nothing about the track.
    for c in &answer.candidates {
        assert_eq!(c.source, online(DISCOGS_NAME, None));
        assert!(matches!(
            c.field,
            IdentifyField::Album
                | IdentifyField::Label
                | IdentifyField::Year
                | IdentifyField::CatalogueNumber
        ));
    }
    assert_eq!(MAX_DISCOGS, 3);
    // One search, by artist and by the title without its version.
    assert_eq!(fake.calls(), ["discogs Test Artist / Test Track"]);
}

// --- Covers ---

fn reads_for(lib: &Lib) -> ReadPool {
    ReadPool::open(lib.writer.guarded_path()).unwrap()
}

fn art(len: usize) -> Artwork {
    Artwork {
        bytes: vec![7; len],
        mime: "image/jpeg".into(),
    }
}

const RELEASE: &str = "00000000-0000-4000-8000-0000000000b1";

#[test]
fn a_cover_is_refused_for_an_id_that_is_not_a_uuid_and_nothing_is_asked() {
    let lib = Lib::new();
    let reads = reads_for(&lib);
    set_opt_in(&lib.writer, Service::CoverArt, true).unwrap();
    let asked = AtomicUsize::new(0);
    for bad in [
        "",
        "not-a-uuid",
        "../../etc",
        "00000000-0000-4000-8000-0000000000b1/../x",
        "00000000-0000-4000-8000-0000000000bg",
        "0000000000004000800000000000b1",
    ] {
        assert!(!is_release_id(bad));
        let answer = cover_with(&reads, &CoverCache::new(), bad, |_| {
            asked.fetch_add(1, Ordering::SeqCst);
            Ok(None)
        })
        .unwrap();
        assert_eq!(
            answer,
            CoverAnswer::Refused {
                reason: "bad_id".into()
            }
        );
    }
    assert_eq!(asked.load(Ordering::SeqCst), 0);
    assert!(is_release_id(RELEASE));
}

#[test]
fn a_cover_is_refused_while_cover_art_is_off_even_for_one_kept_in_memory() {
    let lib = Lib::new();
    let reads = reads_for(&lib);
    let cache = CoverCache::new();
    let asked = AtomicUsize::new(0);
    let fetch = |_: &str| {
        asked.fetch_add(1, Ordering::SeqCst);
        Ok(Some(art(4)))
    };
    let off = CoverAnswer::Refused {
        reason: "cover_art_off".into(),
    };
    assert_eq!(cover_with(&reads, &cache, RELEASE, fetch).unwrap(), off);
    assert_eq!(asked.load(Ordering::SeqCst), 0);

    set_opt_in(&lib.writer, Service::CoverArt, true).unwrap();
    let shown = cover_with(&reads, &cache, RELEASE, fetch).unwrap();
    assert_eq!(
        shown,
        CoverAnswer::Image {
            mime: "image/jpeg".into(),
            data: "BwcHBw==".into()
        }
    );
    // The second ask is answered from memory.
    cover_with(&reads, &cache, RELEASE, fetch).unwrap();
    assert_eq!(asked.load(Ordering::SeqCst), 1);

    set_opt_in(&lib.writer, Service::CoverArt, false).unwrap();
    assert_eq!(cover_with(&reads, &cache, RELEASE, fetch).unwrap(), off);
}

#[test]
fn a_release_with_no_cover_and_a_failed_request_are_answers_not_errors() {
    let lib = Lib::new();
    let reads = reads_for(&lib);
    set_opt_in(&lib.writer, Service::CoverArt, true).unwrap();
    let cache = CoverCache::new();
    assert_eq!(
        cover_with(&reads, &cache, RELEASE, |_| Ok(None)).unwrap(),
        CoverAnswer::NoCover
    );
    let offline = cover_with(&reads, &cache, RELEASE, |_| {
        Err(ClientError::Net(NetError::Failed {
            host: Some("coverartarchive.org".into()),
            reason: "the connection failed",
        }))
    })
    .unwrap();
    assert_eq!(
        offline,
        CoverAnswer::Failed {
            reason: "offline".into()
        }
    );
    assert!(cache.is_empty());
}

#[test]
fn the_cover_cache_is_bounded_by_count_and_size_and_drops_the_oldest_first() {
    let cache = CoverCache::new();
    for n in 0..MAX_IMAGES + 10 {
        cache.put(&format!("id-{n}"), art(1));
    }
    assert_eq!(cache.len(), MAX_IMAGES);
    assert!(cache.get("id-0").is_none() && cache.get("id-9").is_none());
    assert!(cache.get("id-10").is_some() && cache.get(&format!("id-{}", MAX_IMAGES + 9)).is_some());

    let big = CoverCache::new();
    let eight = 8 * 1024 * 1024;
    for n in 0..3 {
        big.put(&format!("big-{n}"), art(eight));
    }
    assert!(big.bytes() <= MAX_BYTES);
    assert_eq!(big.len(), 2);
    assert!(big.get("big-0").is_none() && big.get("big-2").is_some());
    // One image over the limit on its own is not kept, and costs nothing.
    big.put("huge", art(MAX_BYTES + 1));
    assert!(big.get("huge").is_none());
    assert_eq!(big.len(), 2);
}

#[test]
fn covers_are_never_written_to_disk() {
    // The cover module has no way to touch the file system or a database:
    // its images live in memory only.
    let source = include_str!("cover.rs");
    for forbidden in [
        "std::fs",
        "fs::",
        "File::",
        "OpenOptions",
        "tempfile",
        "create_dir",
        ".write(",
        "write_all",
        "rusqlite::params",
        "INSERT",
        "UPDATE ",
    ] {
        assert!(!source.contains(forbidden), "cover.rs mentions {forbidden}");
    }
}

// --- Inbox files looked up on arrival ---

struct Noop;

impl JobHandler for Noop {
    fn run(&self, _: &JobContext) -> Result<(), JobError> {
        Ok(())
    }
}

/// An Inbox folder with one file that has held still: the next hash job's
/// intake makes it an arrival.
fn inbox_with_one_arrival() -> (Lib, i64) {
    let lib = Lib::new();
    let (_, file) = lib.track(None, None, "Test Artist - Test Track.mp3");
    lib.exec(
        "UPDATE music_folder SET role = 'inbox' WHERE id = 1;
         INSERT INTO inbox_batch (music_folder_id, kind) VALUES (1, 'already_there');
         INSERT INTO file_stage (file_id, stage, version, size, mtime, status)
         VALUES (1, 'hash', 1, NULL, NULL, 'done');",
    );
    (lib, file)
}

/// Runs a hash job ended by intake, with the look-up job registered, and
/// answers the targets of the look-up jobs that were queued.
fn arrivals_queued(on: &[Service]) -> Vec<String> {
    let (lib, file) = inbox_with_one_arrival();
    assert_eq!(file, 1);
    for &service in on {
        set_opt_in(&lib.writer, service, true).unwrap();
    }
    let reads = reads_for(&lib);
    let queue = JobQueue::builder(lib.writer.clone())
        .workers(1)
        .handler(JobKind::Hash, crate::inbox::intake::after_hash(Noop))
        .handler(
            JobKind::IdentifyLookup,
            Looker::new(Fake::matching(), reads),
        )
        .start()
        .unwrap();
    queue.enqueue(NewJob::new(JobKind::Hash)).unwrap();
    let start = Instant::now();
    loop {
        let open: i64 = lib
            .writer
            .call(|c| {
                c.query_row(
                    "SELECT count(*) FROM job WHERE status IN ('queued', 'running')",
                    [],
                    |r| r.get(0),
                )
            })
            .unwrap();
        let hashed: i64 = lib
            .writer
            .call(|c| {
                c.query_row(
                    "SELECT count(*) FROM job WHERE kind = 'hash' AND status = 'done'",
                    [],
                    |r| r.get(0),
                )
            })
            .unwrap();
        if open == 0 && hashed == 1 {
            break;
        }
        assert!(start.elapsed() < Duration::from_secs(60), "never finished");
        std::thread::sleep(Duration::from_millis(5));
    }
    queue.shutdown();
    lib.writer
        .call(|c| {
            let mut stmt =
                c.prepare("SELECT target FROM job WHERE kind = 'identify_lookup' ORDER BY id")?;
            let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
            rows.collect()
        })
        .unwrap()
}

#[test]
fn an_arriving_inbox_file_is_queued_only_while_acoustid_or_musicbrainz_is_on() {
    for on in [
        vec![Service::AcoustId],
        vec![Service::MusicBrainz],
        vec![Service::AcoustId, Service::MusicBrainz, Service::Discogs],
    ] {
        let queued = arrivals_queued(&on);
        assert_eq!(queued.len(), 1, "{on:?}");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&queued[0]).unwrap(),
            serde_json::json!({"file_ids": [1], "automatic": true}),
            "{on:?}"
        );
    }
}

#[test]
fn nothing_is_queued_for_an_arriving_inbox_file_while_every_service_is_off() {
    assert!(arrivals_queued(&[]).is_empty());
    // Discogs and Cover Art alone don't identify a file by themselves.
    assert!(arrivals_queued(&[Service::Discogs, Service::CoverArt]).is_empty());
}

#[test]
fn an_arrival_batch_is_one_job_in_the_order_given_and_none_for_no_files() {
    let (lib, _) = inbox_with_one_arrival();
    set_opt_in(&lib.writer, Service::MusicBrainz, true).unwrap();
    let job = lib.writer.call(|c| arrivals_job(c, &[3, 1, 2])).unwrap();
    let job = job.expect("a job");
    assert_eq!(
        job.target,
        Some(serde_json::json!({"file_ids": [3, 1, 2], "automatic": true}))
    );
    assert_eq!(job.priority, super::job::PRIORITY);
    assert!(lib.writer.call(|c| arrivals_job(c, &[])).unwrap().is_none());
}

#[test]
fn an_inbox_file_s_status_has_the_same_states_as_a_track_s() {
    let (lib, file) = inbox_with_one_arrival();
    let state = |file: i64| {
        lib.writer
            .call(move |c| file_statuses(c, &[file]))
            .unwrap()
            .remove(0)
    };
    assert_eq!(state(file).state, LookupState::Never);
    assert_eq!(state(999).state, LookupState::Never);
    let answer = Answer::failed(Why::Offline);
    lib.writer
        .call(move |c| cache::save(c, file, &answer))
        .unwrap();
    let s = state(file);
    assert_eq!(
        (s.file_id, s.state, s.reason.as_deref()),
        (file, LookupState::Failed, Some("offline"))
    );
    let answer = Answer::skipped(Why::NoToken);
    lib.writer
        .call(move |c| cache::save(c, file, &answer))
        .unwrap();
    assert_eq!(state(file).state, LookupState::Skipped);
}

// --- The token ---

#[test]
fn the_discogs_token_never_appears_in_any_reply_or_error() {
    let lib = Lib::new();
    let credentials = Credentials::in_memory();
    credentials.set(DISCOGS, SECRET).unwrap();
    let live = Live::new(Net::new(reads_for(&lib)), credentials.clone());
    assert!(live.discogs_token_saved());
    // Discogs is off at the gate: refused before anything is sent.
    let error = live.discogs("Test Artist", "Test Track").unwrap_err();
    let shown = format!("{error} {error:?}");
    assert!(!shown.contains(SECRET), "{shown}");
    let answer = look_up(&live, ONLY_DISCOGS, &named());
    assert!(!format!("{answer:?}").contains(SECRET));
    // A token that can't be a header value sends nothing, and says nothing of it.
    credentials.set(DISCOGS, "bad\ntoken-SECRET").unwrap();
    let error = live.discogs("q", "q").unwrap_err();
    let shown = format!("{error} {error:?}");
    assert!(!shown.contains("SECRET"), "{shown}");
    // The store itself never prints an entry.
    assert!(!format!("{credentials:?}").contains("SECRET"));
    // Nothing stored for a file carries it either.
    let (lib, file) = inbox_with_one_arrival();
    let saved = Answer {
        outcome: Outcome::Found,
        why: None,
        candidates: look_up(
            &fake_with(|f| {
                f.token_saved = true;
                f.discogs_hits = vec![hit(Service::Discogs, "Test Track", "Test Artist")];
            }),
            ONLY_DISCOGS,
            &named(),
        )
        .candidates,
        releases: Vec::new(),
        path: "discogs".into(),
    };
    lib.writer
        .call(move |c| cache::save(c, file, &saved))
        .unwrap();
    let row: String = lib
        .writer
        .call(|c| {
            c.query_row(
                "SELECT candidates || coalesce(reason, '') FROM identify_lookup",
                [],
                |r| r.get(0),
            )
        })
        .unwrap();
    assert!(!row.contains(SECRET));
}

// --- Follow-ups from the review of the look-up job ---

#[test]
fn a_year_outside_1000_to_9999_is_never_offered_from_musicbrainz() {
    let year_of = |date: &str| {
        let fake = fake_with(|f| f.release_date = Some(date.to_owned()));
        let on = On {
            acoustid: true,
            musicbrainz: true,
            discogs: false,
        };
        let input = Input {
            fingerprint: Some(vec![1]),
            duration_ms: Some(1000),
            ..Input::default()
        };
        look_up(&fake, on, &input)
            .candidates
            .iter()
            .find(|c| c.field == IdentifyField::Year)
            .map(|c| c.value.clone())
    };
    assert_eq!(year_of("0000-05-06"), None);
    assert_eq!(year_of("0999"), None);
    assert_eq!(year_of("19x5"), None);
    assert_eq!(year_of("1000"), Some("1000".into()));
    assert_eq!(year_of("2003-05-06"), Some("2003".into()));
    assert_eq!(year_of("9999"), Some("9999".into()));
}

#[test]
fn a_year_outside_1000_to_9999_is_never_offered_from_discogs() {
    for (year, shown) in [(0, false), (999, false), (1000, true), (10_000, false)] {
        let mut h = hit(Service::Discogs, "Test Track", "Test Artist");
        h.year = Some(year);
        let fake = fake_with(|f| {
            f.token_saved = true;
            f.discogs_hits = vec![h];
        });
        let answer = look_up(&fake, ONLY_DISCOGS, &named());
        let has = answer
            .candidates
            .iter()
            .any(|c| c.field == IdentifyField::Year);
        assert_eq!(has, shown, "{year}");
    }
}

/// A file with a fingerprint, and what `cache` says of it.
fn fingerprinted(lib: &Lib, file: i64, bytes: &'static [u8]) {
    lib.writer
        .call(move |c| {
            c.execute(
                "UPDATE file SET fingerprint = ?1 WHERE id = ?2",
                (bytes, file),
            )
        })
        .unwrap();
}

fn found_answer() -> Answer {
    let fake = fake_with(|f| {
        f.token_saved = true;
        f.discogs_hits = vec![hit(Service::Discogs, "Test Track", "Test Artist")];
    });
    look_up(&fake, ONLY_DISCOGS, &named())
}

#[test]
fn an_answer_for_audio_that_has_changed_since_is_not_found_and_needs_a_new_look_up() {
    let (lib, file) = inbox_with_one_arrival();
    fingerprinted(&lib, file, &[1, 2, 3]);
    let answer = found_answer();
    lib.writer
        .call(move |c| cache::save(c, file, &answer))
        .unwrap();
    let state = |lib: &Lib| {
        lib.writer
            .call(move |c| file_statuses(c, &[file]))
            .unwrap()
            .remove(0)
    };
    assert_eq!(state(&lib).state, LookupState::Found);
    let kept = lib
        .writer
        .call(move |c| cache::load(c, file))
        .unwrap()
        .unwrap();
    assert!(!kept.stale);

    // The file is replaced in place: another fingerprint.
    fingerprinted(&lib, file, &[9, 9, 9]);
    let kept = lib
        .writer
        .call(move |c| cache::load(c, file))
        .unwrap()
        .unwrap();
    assert!(kept.stale);
    let s = state(&lib);
    assert_eq!((s.state, s.found), (LookupState::Never, 0));
    let releases = lib
        .writer
        .call(move |c| super::cover::releases_of_file(c, file))
        .unwrap();
    assert!(releases.is_empty());

    // A new answer replaces it outright, even one that only skipped: the
    // old details are not kept beside it as a "good older answer".
    let skipped = Answer::skipped(Why::NoToken);
    lib.writer
        .call(move |c| cache::save(c, file, &skipped))
        .unwrap();
    let s = state(&lib);
    assert_eq!(
        (s.state, s.found, s.later_attempt),
        (LookupState::Skipped, 0, None)
    );
}

#[test]
fn an_answer_found_before_the_file_had_a_fingerprint_stays_good_once_it_has_one() {
    let (lib, file) = inbox_with_one_arrival();
    let answer = found_answer();
    lib.writer
        .call(move |c| cache::save(c, file, &answer))
        .unwrap();
    fingerprinted(&lib, file, &[1, 2, 3]);
    let kept = lib
        .writer
        .call(move |c| cache::load(c, file))
        .unwrap()
        .unwrap();
    assert!(!kept.stale);
}

#[test]
fn an_answer_is_kept_with_the_fingerprint_the_look_up_read_not_the_one_after() {
    let (lib, file) = inbox_with_one_arrival();
    fingerprinted(&lib, file, &[1, 2, 3]);
    let read = lib
        .writer
        .call(move |c| {
            c.query_row(
                "SELECT fingerprint_rev FROM file WHERE id = ?1",
                [file],
                |r| r.get::<_, i64>(0),
            )
        })
        .unwrap();
    // The audio changes while the look-up is under way.
    fingerprinted(&lib, file, &[9, 9, 9]);
    let answer = found_answer();
    lib.writer
        .call(move |c| cache::save_at(c, file, &answer, read))
        .unwrap();
    let kept = lib
        .writer
        .call(move |c| cache::load(c, file))
        .unwrap()
        .unwrap();
    assert!(kept.stale);
}

#[test]
fn with_musicbrainz_off_acoustid_is_not_asked_even_with_a_fingerprint() {
    let fake = Fake::matching();
    let on = On {
        acoustid: true,
        musicbrainz: false,
        discogs: false,
    };
    let input = Input {
        fingerprint: Some(vec![1]),
        duration_ms: Some(1000),
        ..named()
    };
    let answer = look_up(&fake, on, &input);
    assert_eq!(
        (answer.outcome, answer.why),
        (Outcome::Skipped, Some(Why::MusicbrainzOff))
    );
    assert!(fake.calls().is_empty());
}

#[test]
fn only_a_gate_failure_that_means_no_internet_counts_as_offline() {
    let failed = |reason| NetError::Failed { host: None, reason };
    assert!(failed("the connection failed").is_offline());
    assert!(failed("the host wasn't found").is_offline());
    assert!(failed("it timed out").is_offline());
    assert!(!failed("the secure connection failed").is_offline());
    assert!(!NetError::NotOptedIn(Service::Discogs).is_offline());
    // The look-up marks the file failed as offline, not as a plain failure.
    let answer = look_up(
        &Fake::new(Box::new(|_| {
            Err(ClientError::Net(NetError::Failed {
                host: None,
                reason: "it timed out",
            }))
        })),
        On {
            acoustid: true,
            musicbrainz: true,
            discogs: false,
        },
        &Input {
            fingerprint: Some(vec![1]),
            duration_ms: Some(1000),
            ..Input::default()
        },
    );
    assert_eq!(
        (answer.outcome, answer.why),
        (Outcome::Failed, Some(Why::Offline))
    );
}

#[test]
fn recording_ids_of_tracks_not_loaded_anywhere_are_all_queued_with_their_files() {
    let lib = Lib::new();
    let mut recordings = Vec::new();
    let mut files = Vec::new();
    for n in 0..40 {
        let (recording, file) = lib.track(None, None, &format!("Artist - Title {n}.mp3"));
        lib.library(recording, file);
        recordings.push(recording);
        files.push(file);
    }
    // A recording in no Library, one unknown, and a repeat.
    let (outside, _) = lib.track(None, None, "Nobody - Outside.mp3");
    let mut asked = recordings.clone();
    asked.extend([outside, 9_999, recordings[0]]);
    let found = lib
        .writer
        .call(move |c| super::job::linked_files_of_recordings(c, &asked))
        .unwrap();
    assert_eq!(found, files);
    // They go to the job as one list.
    assert_eq!(
        super::job::lookup_job(&found).target,
        Some(serde_json::json!({ "file_ids": files }))
    );
}

#[test]
fn an_answer_with_no_recorded_revision_is_not_shown_as_found_and_is_never_requeued_by_the_migration(
) {
    let (lib, file) = inbox_with_one_arrival();
    fingerprinted(&lib, file, &[1, 2, 3]);
    let answer = found_answer();
    lib.writer
        .call(move |c| cache::save(c, file, &answer))
        .unwrap();
    let state = |lib: &Lib| {
        lib.writer
            .call(move |c| file_statuses(c, &[file]))
            .unwrap()
            .remove(0)
    };
    assert_eq!(state(&lib).state, LookupState::Found);
    // A row from before migration 0059: no revision recorded.
    lib.exec("UPDATE identify_lookup SET fingerprint_rev = NULL");
    let kept = lib
        .writer
        .call(move |c| cache::load(c, file))
        .unwrap()
        .unwrap();
    assert!(kept.stale);
    assert_eq!(state(&lib).state, LookupState::Never);
    let releases = lib
        .writer
        .call(move |c| super::cover::releases_of_file(c, file))
        .unwrap();
    assert!(releases.is_empty());
    // The migration queued nothing: asking again is up to the user.
    let jobs: i64 = lib
        .writer
        .call(|c| c.query_row("SELECT count(*) FROM job", [], |r| r.get(0)))
        .unwrap();
    assert_eq!(jobs, 0);
}

// --- Fix round 1 ---

#[test]
fn credits_split_into_names_and_a_longer_name_is_not_the_shorter_one() {
    use super::names::{artists_match, credit_names};
    assert_eq!(credit_names("Kit feat. Y"), ["kit", "y"]);
    assert_eq!(
        credit_names("A & B, C ft. D featuring E x F and G vs H").len(),
        8
    );
    assert_eq!(credit_names("Eric B. & Rakim"), ["eric b", "rakim"]);
    // A joiner at an end is part of a name.
    assert_eq!(credit_names("Malcolm X"), ["malcolm x"]);
    assert_eq!(credit_names("Artist (2)"), ["artist"]);
    assert!(artists_match("Kit feat. Y", "Kit"));
    assert!(artists_match("kit", "Y & KIT"));
    assert!(!artists_match("Kit", "Kit Sebastian"));
    assert!(!artists_match("Prince", "Prince Royce"));
    assert!(!artists_match("", ""));
}

fn mixes(answer: &Answer) -> Vec<String> {
    answer
        .candidates
        .iter()
        .filter(|c| c.field == IdentifyField::Mix)
        .map(|c| c.value.clone())
        .collect()
}

#[test]
fn a_name_search_hit_gives_a_mix_only_when_its_version_is_ours() {
    // The recording the search leads to is the fixture's "... (Extended Mix)".
    let fake = fake_with(|f| {
        f.search_hits = vec![hit(Service::MusicBrainz, "Test Track", "Test Artist")];
    });
    let same = look_up(&fake, ONLY_MB, &named());
    assert_eq!(mixes(&same), ["Extended Mix"]);
    // Our file says another mix: the recording's version is not offered.
    let other = Input {
        names: Some(Names {
            artist: "Test Artist".into(),
            title: "Test Track (Radio Edit)".into(),
        }),
        ..Input::default()
    };
    let answer = look_up(&fake, ONLY_MB, &other);
    assert_eq!(answer.outcome, Outcome::Found);
    assert!(mixes(&answer).is_empty());
    // And none when ours has no version.
    let bare = Input {
        names: Some(Names {
            artist: "Test Artist".into(),
            title: "Test Track".into(),
        }),
        ..Input::default()
    };
    assert!(mixes(&look_up(&fake, ONLY_MB, &bare)).is_empty());
    // AcoustID, which tells versions apart, still gives it.
    let with_print = Input {
        fingerprint: Some(vec![1]),
        duration_ms: Some(1000),
        ..bare
    };
    let on = On {
        acoustid: true,
        musicbrainz: true,
        discogs: false,
    };
    assert_eq!(
        mixes(&look_up(&Fake::matching(), on, &with_print)),
        ["Extended Mix"]
    );
}

#[test]
fn a_token_that_went_missing_reports_skipped_no_token_not_failed() {
    let fake = fake_with(|f| {
        f.token_saved = true;
        f.discogs_no_token = true;
    });
    let answer = look_up(&fake, ONLY_DISCOGS, &named());
    assert_eq!(
        (answer.outcome, answer.why),
        (Outcome::Skipped, Some(Why::NoToken))
    );
}

#[test]
fn the_path_says_which_services_were_actually_asked() {
    let on_all = On {
        acoustid: true,
        musicbrainz: true,
        discogs: true,
    };
    let print = Input {
        fingerprint: Some(vec![1]),
        duration_ms: Some(1000),
        ..Input::default()
    };
    // Name search only.
    let fake = fake_with(|f| {
        f.search_hits = vec![hit(Service::MusicBrainz, "Test Track", "Test Artist")];
    });
    assert_eq!(look_up(&fake, ONLY_MB, &named()).path, "musicbrainz-name");
    // AcoustID, then MusicBrainz by its recording.
    let no_discogs = On {
        discogs: false,
        ..on_all
    };
    assert_eq!(
        look_up(&Fake::matching(), no_discogs, &print).path,
        "acoustid+musicbrainz"
    );
    // Discogs only.
    let fake = fake_with(|f| f.token_saved = true);
    assert_eq!(look_up(&fake, ONLY_DISCOGS, &named()).path, "discogs");
    // AcoustID found nothing, the name search ran, and Discogs too.
    let nothing = Fake::new(Box::new(|_| Ok(Vec::new())));
    let fake = Arc::new(Fake {
        token_saved: true,
        ..Arc::into_inner(nothing).unwrap()
    });
    let both = Input {
        names: Some(names()),
        ..print
    };
    assert_eq!(
        look_up(&fake, on_all, &both).path,
        "acoustid+musicbrainz-name+discogs"
    );
    // Nothing asked: none.
    let none = look_up(&Fake::matching(), ONLY_DISCOGS, &Input::default());
    assert_eq!(none.path, "none");
}

#[test]
fn the_stored_path_is_the_one_used_not_a_fixed_one() {
    let (lib, file) = inbox_with_one_arrival();
    let fake = fake_with(|f| {
        f.token_saved = true;
        f.discogs_hits = vec![hit(Service::Discogs, "Test Album", "Test Artist")];
    });
    let answer = look_up(&fake, ONLY_DISCOGS, &named());
    lib.writer
        .call(move |c| cache::save(c, file, &answer))
        .unwrap();
    let path: String = lib
        .writer
        .call(|c| c.query_row("SELECT path FROM identify_lookup", [], |r| r.get(0)))
        .unwrap();
    assert_eq!(path, "discogs");
}

#[test]
fn the_discogs_token_is_read_from_the_store_once_per_batch() {
    let lib = Lib::new();
    let credentials = Credentials::in_memory();
    credentials.set(DISCOGS, SECRET).unwrap();
    let live = Live::new(Net::new(reads_for(&lib)), credentials.clone());
    assert!(live.discogs_token_saved());
    // Gone from the store: the batch in progress still has what it read.
    credentials.clear(DISCOGS).unwrap();
    assert!(live.discogs_token_saved());
    // The next batch reads the store again.
    live.new_batch();
    assert!(!live.discogs_token_saved());
    credentials.set(DISCOGS, SECRET).unwrap();
    assert!(!live.discogs_token_saved());
    live.new_batch();
    assert!(live.discogs_token_saved());
}

mod automatic {
    use super::super::job::{automatic_lookup_job, lookup_job};
    use super::super::online_tests::{setup, Setup};
    use super::*;
    use crate::jobs::JobStatus;

    fn run_job(s: &Setup, fake: &Arc<Fake>, job: NewJob) {
        let queue = s.queue(fake);
        let id = queue.enqueue(job).unwrap();
        assert_eq!(s.wait(id), JobStatus::Done);
        queue.shutdown();
    }

    #[test]
    fn an_automatic_job_asks_nothing_once_acoustid_and_musicbrainz_are_off_but_a_manual_one_may() {
        let s = setup(2);
        // Switched on when the files arrived, off by the time the job runs;
        // Discogs is on and has a token.
        s.switch(Service::Discogs, true);
        let fake = fake_with(|f| {
            f.token_saved = true;
            f.discogs_hits = vec![hit(Service::Discogs, "Title 0", "Artist")];
        });
        run_job(&s, &fake, automatic_lookup_job(&s.files()));
        assert!(fake.calls().is_empty(), "{:?}", fake.calls());
        for file in s.files() {
            assert_eq!(
                s.stored(file),
                Some((Outcome::Skipped, Some("services_off".into())))
            );
        }
        // The same files by request, with only Discogs on: allowed.
        run_job(&s, &fake, lookup_job(&s.files()));
        assert_eq!(fake.calls().len(), 2);
        assert!(fake.calls().iter().all(|c| c.starts_with("discogs ")));
    }

    #[test]
    fn an_automatic_job_still_asks_while_one_of_the_two_is_on() {
        let s = setup(1);
        s.switch(Service::MusicBrainz, true);
        let fake = fake_with(|_| {});
        run_job(&s, &fake, automatic_lookup_job(&s.files()));
        assert!(!fake.calls().is_empty());
    }

    fn status_of_file(s: &Setup) -> super::super::job::InboxLookupStatus {
        let file = s.files()[0];
        s.lib
            .writer
            .call(move |c| file_statuses(c, &[file]))
            .unwrap()
            .remove(0)
    }

    #[test]
    fn an_inbox_file_s_status_names_the_best_artist_title_and_mix_with_their_sources() {
        let s = setup(1);
        s.both_on();
        s.switch(Service::Discogs, true);
        let fake = Fake::matching();
        run_job(&s, &fake, lookup_job(&s.files()));
        let found = status_of_file(&s).identified.expect("identified");
        assert_eq!(
            found.artist.as_deref(),
            Some("Test Artist & Second Act feat. Third Voice")
        );
        assert_eq!(found.title.as_deref(), Some("Test Track (Extended Mix)"));
        assert_eq!(found.mix.as_deref(), Some("Extended Mix"));
        assert_eq!(found.sources, [online("MusicBrainz", Some("AcoustID"))]);
        // Nothing looked up yet: null.
        assert_eq!(status_of_file(&setup(1)).identified, None);
    }
}
