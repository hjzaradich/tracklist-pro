//! The online look-up: its flow, its cache and its job (ROADMAP 2.2; 6A-S2a).
//! The services are a scripted stand-in ([`Fake`]) answering with replies
//! parsed from the MusicBrainz fixtures; nothing here goes online.

use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::job::{lookup_job, status_of, Looker, LookupState};
use super::lookup::{
    look_up, Answer, Backend, Input, On, Outcome, Why, MAX_RECORDINGS, MAX_RELEASES, MIN_SCORE,
};
use super::*;
use crate::jobs::{JobId, JobKind, JobQueue, JobStatus};
use crate::net::services::acoustid::AcoustidMatch;
use crate::net::services::musicbrainz::{self, Recording, Release};
use crate::net::services::{ClientError, LookupHit};
use crate::net::{set_opt_in, NetError, Service};
use crate::search::tests::support::{Lib, Mount};

const RECORDING: &str = include_str!("../net/services/fixtures/musicbrainz_recording.json");
const RELEASE: &str = include_str!("../net/services/fixtures/musicbrainz_release.json");

type AcoustidReply = Box<dyn Fn(&[u8]) -> Result<Vec<AcoustidMatch>, ClientError> + Send + Sync>;

type BeforeAcoustid = Box<dyn Fn(&[u8]) + Send + Sync>;

/// Scripted services. `calls` lists every request, e.g. `acoustid`,
/// `recording <id>`, `release <id>`.
pub(super) struct Fake {
    pub(super) key_missing: bool,
    pub(super) calls: Mutex<Vec<String>>,
    pub(super) acoustid: AcoustidReply,
    /// Run before each AcoustID request, with its fingerprint.
    before_acoustid: BeforeAcoustid,
    /// Recording ids MusicBrainz doesn't know.
    pub(super) unknown: Vec<String>,
    /// What a MusicBrainz recording search finds.
    pub(super) search_hits: Vec<LookupHit>,
    /// Whether a Discogs token is saved, and what a Discogs search finds.
    pub(super) token_saved: bool,
    pub(super) discogs_hits: Vec<LookupHit>,
    /// Replaces the fixture release's date.
    pub(super) release_date: Option<String>,
}

impl Fake {
    pub(super) fn new(acoustid: AcoustidReply) -> Arc<Fake> {
        Arc::new(Fake {
            key_missing: false,
            calls: Mutex::new(Vec::new()),
            acoustid,
            before_acoustid: Box::new(|_| {}),
            unknown: Vec::new(),
            search_hits: Vec::new(),
            token_saved: false,
            discogs_hits: Vec::new(),
            release_date: None,
        })
    }

    /// Every AcoustID request matches the one recording the fixture holds.
    pub(super) fn matching() -> Arc<Fake> {
        Fake::new(Box::new(|_| Ok(vec![hit(0.95, &["rec-1"])])))
    }

    pub(super) fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
}

impl Backend for Arc<Fake> {
    fn acoustid_key_missing(&self) -> bool {
        self.key_missing
    }
    fn acoustid(&self, blob: &[u8], _seconds: u32) -> Result<Vec<AcoustidMatch>, ClientError> {
        (self.before_acoustid)(blob);
        self.calls.lock().unwrap().push("acoustid".into());
        (self.acoustid)(blob)
    }
    fn recording(&self, mbid: &str) -> Result<Recording, ClientError> {
        self.calls.lock().unwrap().push(format!("recording {mbid}"));
        if self.unknown.iter().any(|u| u == mbid) {
            return Err(ClientError::NotFound(Service::MusicBrainz));
        }
        musicbrainz::parse_recording(RECORDING.as_bytes())
    }
    fn release(&self, mbid: &str) -> Result<Release, ClientError> {
        self.calls.lock().unwrap().push(format!("release {mbid}"));
        let mut release = musicbrainz::parse_release(RELEASE.as_bytes())?;
        release.id = mbid.to_owned();
        if let Some(date) = &self.release_date {
            release.date = Some(date.clone());
        }
        Ok(release)
    }
    fn search_recordings(&self, query: &str) -> Result<Vec<LookupHit>, ClientError> {
        self.calls.lock().unwrap().push(format!("search {query}"));
        Ok(self.search_hits.clone())
    }
    fn discogs_token_saved(&self) -> bool {
        self.token_saved
    }
    fn discogs(&self, query: &str) -> Result<Vec<LookupHit>, ClientError> {
        self.calls.lock().unwrap().push(format!("discogs {query}"));
        Ok(self.discogs_hits.clone())
    }
}

fn hit(score: f64, recordings: &[&str]) -> AcoustidMatch {
    AcoustidMatch {
        score,
        recording_ids: recordings.iter().map(|r| (*r).to_owned()).collect(),
    }
}

const ON: On = On {
    acoustid: true,
    musicbrainz: true,
    discogs: false,
};

fn input() -> Input {
    Input {
        fingerprint: Some(vec![1, 2, 3]),
        duration_ms: Some(215_400),
        names: None,
        fingerprint_rev: 0,
    }
}

fn pairs(answer: &Answer) -> Vec<(IdentifyField, &str)> {
    answer
        .candidates
        .iter()
        .map(|c| (c.field, c.value.as_str()))
        .collect()
}

// --- The flow ---

#[test]
fn no_request_is_made_while_every_service_is_off() {
    let fake = Fake::matching();
    let off = On {
        acoustid: false,
        musicbrainz: false,
        discogs: false,
    };
    let answer = look_up(&fake, off, &input());
    assert_eq!(
        (answer.outcome, answer.why),
        (Outcome::Skipped, Some(Why::ServicesOff))
    );
    assert!(fake.calls().is_empty());
    // AcoustID off with MusicBrainz on and no name to search by: nothing to go on.
    let only_mb = On {
        acoustid: false,
        musicbrainz: true,
        discogs: false,
    };
    assert_eq!(look_up(&fake, only_mb, &input()).why, Some(Why::NoName));
    assert!(fake.calls().is_empty());
}

#[test]
fn acoustid_is_skipped_while_its_key_is_missing_and_the_outcome_says_so() {
    let fake = Arc::new(Fake {
        key_missing: true,
        ..Arc::into_inner(Fake::matching()).unwrap()
    });
    let answer = look_up(&fake, ON, &input());
    assert_eq!(
        (answer.outcome, answer.why),
        (Outcome::Skipped, Some(Why::KeyMissing))
    );
    assert!(fake.calls().is_empty());
}

#[test]
fn a_file_without_a_fingerprint_or_a_length_is_skipped_and_nothing_is_sent() {
    let fake = Fake::matching();
    for input in [
        Input {
            fingerprint: None,
            duration_ms: Some(1000),
            names: None,
            fingerprint_rev: 0,
        },
        Input {
            fingerprint: Some(vec![1]),
            duration_ms: None,
            names: None,
            fingerprint_rev: 0,
        },
        Input {
            fingerprint: Some(vec![1]),
            duration_ms: Some(0),
            names: None,
            fingerprint_rev: 0,
        },
    ] {
        let answer = look_up(&fake, ON, &input);
        assert_eq!(
            (answer.outcome, answer.why),
            (Outcome::Skipped, Some(Why::NoFingerprint))
        );
    }
    assert!(fake.calls().is_empty());
}

#[test]
fn a_score_below_the_minimum_is_dropped_and_at_most_three_recordings_are_asked_best_first() {
    assert_eq!(MIN_SCORE, 0.8);
    let fake = Fake::new(Box::new(|_| {
        Ok(vec![
            hit(0.79, &["too-low"]),
            hit(0.81, &["c"]),
            hit(0.99, &["a", "b"]),
            hit(0.9, &["d", "a"]),
        ])
    }));
    look_up(&fake, ON, &input());
    let asked: Vec<_> = fake
        .calls()
        .into_iter()
        .filter(|c| c.starts_with("recording"))
        .collect();
    assert_eq!(asked, ["recording a", "recording b", "recording d"]);
    assert_eq!(asked.len(), MAX_RECORDINGS);

    // Only low scores: AcoustID found nothing worth asking MusicBrainz about.
    let low = Fake::new(Box::new(|_| Ok(vec![hit(0.5, &["x"])])));
    let answer = look_up(&low, ON, &input());
    assert_eq!(
        (answer.outcome, answer.why),
        (Outcome::Nothing, Some(Why::NoMatch))
    );
    assert_eq!(low.calls(), ["acoustid"]);
}

#[test]
fn recording_and_release_json_become_candidates_with_catalogue_number_and_year() {
    let fake = Fake::matching();
    let answer = look_up(&fake, ON, &input());
    assert_eq!(answer.outcome, Outcome::Found);
    // From the recording: the title as written, the artist credit and the
    // version it names. From each chosen release: album, year, label, number.
    let found = pairs(&answer);
    for expected in [
        (IdentifyField::Name, "Test Track (Extended Mix)"),
        (IdentifyField::Mix, "Extended Mix"),
        (IdentifyField::Album, "Test Album"),
        (IdentifyField::Year, "2001"),
        (IdentifyField::Label, "Test Records"),
        (IdentifyField::CatalogueNumber, "TST001"),
        (IdentifyField::Label, "Second Label"),
    ] {
        assert!(
            found.contains(&expected),
            "{expected:?} is missing from {found:?}"
        );
    }
    assert!(answer
        .candidates
        .iter()
        .all(|c| c.source == lookup::source()));
}

#[test]
fn the_artist_credit_keeps_its_join_phrases() {
    let answer = look_up(&Fake::matching(), ON, &input());
    assert!(pairs(&answer).contains(&(
        IdentifyField::Artist,
        "Test Artist & Second Act feat. Third Voice"
    )));
}

#[test]
fn at_most_two_releases_are_asked_official_ones_first_then_the_earliest() {
    let fake = Fake::matching();
    let answer = look_up(&fake, ON, &input());
    // The fixture has an official 2009 and 2001 release and a promotional
    // 2000 one: the promotion loses to both official ones.
    let asked: Vec<_> = fake
        .calls()
        .into_iter()
        .filter(|c| c.starts_with("release"))
        .collect();
    assert_eq!(
        asked,
        [
            "release 00000000-0000-4000-8000-0000000000b1",
            "release 00000000-0000-4000-8000-0000000000b2"
        ]
    );
    assert_eq!(asked.len(), MAX_RELEASES);
    assert_eq!(answer.releases.len(), 2);
}

#[test]
fn with_musicbrainz_off_acoustid_is_not_asked_and_the_outcome_says_so() {
    let fake = Fake::matching();
    let on = On {
        acoustid: true,
        musicbrainz: false,
        discogs: false,
    };
    let answer = look_up(&fake, on, &input());
    assert_eq!(
        (answer.outcome, answer.why),
        (Outcome::Skipped, Some(Why::MusicbrainzOff))
    );
    // AcoustID is not even asked: its recording ids are no use without MusicBrainz.
    assert!(fake.calls().is_empty());
}

#[test]
fn a_recording_musicbrainz_no_longer_has_is_passed_over_not_a_failure() {
    let fake = Arc::new(Fake {
        unknown: vec!["rec-1".into()],
        ..Arc::into_inner(Fake::matching()).unwrap()
    });
    let answer = look_up(&fake, ON, &input());
    assert_eq!(
        (answer.outcome, answer.why),
        (Outcome::Nothing, Some(Why::NoDetails))
    );
}

// --- The job ---

/// A Library track whose linked file has a fingerprint and a length.
struct Setup {
    lib: Lib,
    reads: ReadPool,
    /// (file, library track) per track.
    tracks: Vec<(i64, i64)>,
}

fn setup(count: usize) -> Setup {
    let lib = Lib::new();
    let mut tracks = Vec::new();
    for n in 0..count {
        let (recording, file) = lib.track(None, None, &format!("Artist - Title {n}.mp3"));
        let library = lib.library(recording, file);
        lib.insert(
            "UPDATE file SET fingerprint = ?1, duration_ms = 215400 WHERE id = ?2",
            (vec![n as u8 + 1; 8], file),
        );
        tracks.push((file, library));
    }
    let reads = ReadPool::open(lib.writer.guarded_path()).unwrap();
    Setup { lib, reads, tracks }
}

impl Setup {
    fn switch(&self, service: Service, on: bool) {
        set_opt_in(&self.lib.writer, service, on).unwrap();
    }

    fn both_on(&self) {
        self.switch(Service::AcoustId, true);
        self.switch(Service::MusicBrainz, true);
    }

    fn queue(&self, fake: &Arc<Fake>) -> JobQueue {
        JobQueue::builder(self.lib.writer.clone())
            .workers(1)
            .handler(
                JobKind::IdentifyLookup,
                Looker::new(fake.clone(), self.reads.clone()),
            )
            .start()
            .unwrap()
    }

    fn files(&self) -> Vec<i64> {
        self.tracks.iter().map(|t| t.0).collect()
    }

    /// Runs the job over every file to its end.
    fn run(&self, fake: &Arc<Fake>) -> JobStatus {
        let queue = self.queue(fake);
        let id = queue.enqueue(lookup_job(&self.files())).unwrap();
        let status = self.wait(id);
        queue.shutdown();
        status
    }

    fn wait(&self, id: JobId) -> JobStatus {
        let start = Instant::now();
        loop {
            let job = self
                .lib
                .writer
                .call(move |c| crate::jobs::store::get(c, id))
                .unwrap()
                .unwrap();
            if job.status.is_finished() {
                return job.status;
            }
            assert!(start.elapsed() < Duration::from_secs(120), "never finished");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// (outcome, reason) stored for `file`, if any.
    fn stored(&self, file: i64) -> Option<(Outcome, Option<String>)> {
        self.lib
            .writer
            .call(move |c| cache::load(c, file))
            .unwrap()
            .map(|c| (c.outcome, c.reason))
    }

    fn status(&self) -> Vec<job::IdentifyLookupStatus> {
        let ids: Vec<_> = self.tracks.iter().map(|t| LibraryTrackId(t.1)).collect();
        self.lib.writer.call(move |c| status_of(c, &ids)).unwrap()
    }
}

fn offline() -> ClientError {
    ClientError::Net(NetError::Failed {
        host: Some("api.acoustid.org".into()),
        reason: "the connection failed",
    })
}

#[test]
fn the_job_makes_no_request_while_the_services_are_off_and_stores_why() {
    let s = setup(2);
    let fake = Fake::matching();
    assert_eq!(s.run(&fake), JobStatus::Done);
    assert!(fake.calls().is_empty());
    for file in s.files() {
        assert_eq!(
            s.stored(file),
            Some((Outcome::Skipped, Some("services_off".into())))
        );
    }
}

#[test]
fn one_failed_file_does_not_stop_the_batch() {
    let s = setup(3);
    s.both_on();
    // The second file's fingerprint is [2; 8].
    let fake = Fake::new(Box::new(|blob| {
        if blob[0] == 2 {
            Err(ClientError::BadReply {
                service: Service::AcoustId,
                what: "it wasn't JSON",
            })
        } else {
            Ok(vec![hit(0.95, &["rec-1"])])
        }
    }));
    assert_eq!(s.run(&fake), JobStatus::Done);
    let files = s.files();
    assert_eq!(s.stored(files[0]).unwrap().0, Outcome::Found);
    assert_eq!(
        s.stored(files[1]),
        Some((Outcome::Failed, Some("bad_reply".into())))
    );
    assert_eq!(s.stored(files[2]).unwrap().0, Outcome::Found);
}

#[test]
fn being_offline_marks_the_remaining_files_failed_with_one_request_in_all() {
    let s = setup(3);
    s.both_on();
    let fake = Fake::new(Box::new(|_| Err(offline())));
    assert_eq!(s.run(&fake), JobStatus::Done);
    assert_eq!(fake.calls(), ["acoustid"]);
    for file in s.files() {
        assert_eq!(
            s.stored(file),
            Some((Outcome::Failed, Some("offline".into())))
        );
    }
}

#[test]
fn cached_candidates_appear_in_identify_suggestions_as_musicbrainz_via_acoustid() {
    let s = setup(1);
    s.both_on();
    assert_eq!(s.run(&Fake::matching()), JobStatus::Done);
    let track = LibraryTrackId(s.tracks[0].1);
    let suggestions = s
        .lib
        .writer
        .call(move |c| suggestions_cached(c, &Mount, track))
        .unwrap()
        .unwrap();
    let label = suggestions
        .fields
        .iter()
        .find(|f| f.field == IdentifyField::Label)
        .unwrap();
    let values: Vec<_> = label.suggestions.iter().map(|s| s.value.as_str()).collect();
    assert_eq!(values, ["Second Label", "Test Records"]);
    assert_eq!(
        label.suggestions[1].sources,
        [IdentifySource::Online {
            service: "MusicBrainz".into(),
            via: Some("AcoustID".into())
        }]
    );
    let catalogue = &suggestions.catalogue_number.suggestions;
    assert_eq!(catalogue[0].value, "TST001");
    // The plain offline call still makes and shows nothing online.
    let offline_only = s
        .lib
        .writer
        .call(move |c| suggestions_with(c, &Mount, track, Vec::new()))
        .unwrap()
        .unwrap();
    assert!(offline_only
        .fields
        .iter()
        .flat_map(|f| &f.suggestions)
        .all(|s| s.sources.iter().all(|src| src.strength() < 3)));
}

#[test]
fn a_completed_new_lookup_replaces_the_old_result_and_a_failed_one_does_not_wipe_it() {
    let s = setup(1);
    let file = s.files()[0];
    let save = |answer: Answer| {
        s.lib
            .writer
            .call(move |c| cache::save(c, file, &answer))
            .unwrap();
        s.lib
            .writer
            .call(move |c| cache::load(c, file))
            .unwrap()
            .unwrap()
    };
    let candidate = |value: &str| Candidate {
        field: IdentifyField::Label,
        value: value.into(),
        source: lookup::source(),
    };
    let found = |value: &str| Answer {
        outcome: Outcome::Found,
        why: None,
        candidates: vec![candidate(value)],
        releases: vec!["rel-1".into()],
    };

    let first = save(found("Old Label"));
    assert_eq!(first.candidates, [candidate("Old Label")]);
    assert_eq!(first.releases, ["rel-1"]);

    // A failure and a skip leave it, and note themselves beside it.
    let after_failure = save(Answer::failed(Why::Offline));
    assert_eq!(after_failure.outcome, Outcome::Found);
    assert_eq!(after_failure.candidates, [candidate("Old Label")]);
    assert_eq!(after_failure.attempt.as_ref().unwrap().0, "offline");
    let after_skip = save(Answer::skipped(Why::KeyMissing));
    assert_eq!(after_skip.candidates, [candidate("Old Label")]);
    assert_eq!(after_skip.attempt.as_ref().unwrap().0, "key_missing");

    // A completed look-up replaces it, the note with it.
    let replaced = save(found("New Label"));
    assert_eq!(replaced.candidates, [candidate("New Label")]);
    assert_eq!(replaced.attempt, None);
    let nothing = save(Answer {
        outcome: Outcome::Nothing,
        why: Some(Why::NoMatch),
        candidates: Vec::new(),
        releases: Vec::new(),
    });
    assert_eq!(
        (nothing.outcome, nothing.candidates.len()),
        (Outcome::Nothing, 0)
    );

    // A failure replaces an older failure rather than piling up notes.
    let other = setup(1);
    let file = other.files()[0];
    for answer in [
        Answer::failed(Why::Offline),
        Answer::skipped(Why::KeyMissing),
    ] {
        other
            .lib
            .writer
            .call(move |c| cache::save(c, file, &answer))
            .unwrap();
    }
    assert_eq!(
        other.stored(file),
        Some((Outcome::Skipped, Some("key_missing".into())))
    );
}

/// A fake that stops at the first file's AcoustID request until released.
fn held() -> (Arc<Fake>, Receiver<()>, Sender<()>) {
    let (reached_tx, reached) = channel();
    let (release, release_rx) = channel::<()>();
    let (reached_tx, release_rx) = (Mutex::new(reached_tx), Mutex::new(release_rx));
    let first = Mutex::new(true);
    let mut fake = Arc::into_inner(Fake::matching()).unwrap();
    fake.before_acoustid = Box::new(move |_| {
        if std::mem::take(&mut *first.lock().unwrap()) {
            reached_tx.lock().unwrap().send(()).unwrap();
            release_rx.lock().unwrap().recv().unwrap();
        }
    });
    (Arc::new(fake), reached, release)
}

#[test]
fn cancelling_stops_between_files_and_keeps_the_one_finished() {
    let s = setup(3);
    s.both_on();
    let (fake, reached, release) = held();
    let queue = s.queue(&fake);
    let id = queue.enqueue(lookup_job(&s.files())).unwrap();
    reached.recv().unwrap();
    queue.cancel(id).unwrap();
    release.send(()).unwrap();
    assert_eq!(s.wait(id), JobStatus::Cancelled);
    queue.shutdown();
    let files = s.files();
    assert_eq!(
        s.stored(files[0]).unwrap().0,
        Outcome::Found,
        "the first was finished"
    );
    assert_eq!(s.stored(files[1]), None);
    assert_eq!(s.stored(files[2]), None);
    assert_eq!(fake.calls().iter().filter(|c| *c == "acoustid").count(), 1);
}

#[test]
fn the_status_says_never_running_and_found_and_a_later_skipped_attempt_beside_it() {
    let s = setup(3);
    s.both_on();
    let never = s.status();
    assert!(never
        .iter()
        .all(|st| st.state == LookupState::Never && st.when.is_none()));

    let (fake, reached, release) = held();
    let queue = s.queue(&fake);
    let id = queue.enqueue(lookup_job(&s.files())).unwrap();
    reached.recv().unwrap();
    // Every file is in the running job, the first is being asked.
    assert!(s.status().iter().all(|st| st.state == LookupState::Running));
    release.send(()).unwrap();
    assert_eq!(s.wait(id), JobStatus::Done);
    queue.shutdown();
    let done = s.status();
    assert!(done
        .iter()
        .all(|st| st.state == LookupState::Found && st.when.is_some()));
    assert!(done[0].found >= 6, "{} details", done[0].found);

    // Asked again with MusicBrainz off: the answer kept stays, the attempt is noted.
    s.switch(Service::MusicBrainz, false);
    assert_eq!(s.run(&Fake::matching()), JobStatus::Done);
    let again = s.status();
    assert_eq!(again[0].state, LookupState::Found);
    assert_eq!(
        again[0].later_attempt.as_ref().unwrap().reason,
        "musicbrainz_off"
    );
    // A track that isn't in the Library has never been looked up.
    let status = s
        .lib
        .writer
        .call(|c| status_of(c, &[LibraryTrackId(9_999)]))
        .unwrap();
    assert_eq!(status[0].state, LookupState::Never);
}

#[test]
fn a_waiting_job_shows_its_files_as_queued() {
    let s = setup(2);
    s.both_on();
    let (fake, reached, release) = held();
    let queue = s.queue(&fake);
    let first = queue.enqueue(lookup_job(&[s.files()[0]])).unwrap();
    reached.recv().unwrap();
    // The one worker is busy with the first file; the second waits.
    let second = queue.enqueue(lookup_job(&[s.files()[1]])).unwrap();
    let states: Vec<_> = s.status().iter().map(|st| st.state).collect();
    assert_eq!(states, [LookupState::Running, LookupState::Queued]);
    release.send(()).unwrap();
    assert_eq!(s.wait(first), JobStatus::Done);
    assert_eq!(s.wait(second), JobStatus::Done);
    queue.shutdown();
}

#[test]
fn a_request_for_tracks_queues_one_job_for_their_linked_files_and_none_for_no_files() {
    let s = setup(2);
    let (fake, reached, release) = held();
    let queue = s.queue(&fake);
    s.both_on();
    let busy = queue.enqueue(lookup_job(&[s.files()[0]])).unwrap();
    reached.recv().unwrap();
    let ids: Vec<_> = s.tracks.iter().map(|t| LibraryTrackId(t.1)).collect();
    let files = s
        .reads
        .read(|c| job::linked_files(c, &[ids[1], ids[0], ids[1]]))
        .unwrap();
    // Each file once, in the order asked.
    assert_eq!(files, [s.files()[1], s.files()[0]]);
    let id = job::enqueue(&queue, files).unwrap().unwrap();
    let record = s
        .lib
        .writer
        .call(move |c| crate::jobs::store::get(c, id))
        .unwrap()
        .unwrap();
    assert_eq!(record.kind, Some(JobKind::IdentifyLookup));
    assert_eq!(record.priority, job::PRIORITY);
    assert!(record.priority < crate::jobs::Priority::NORMAL);
    assert!(record.priority > crate::jobs::Priority::BACKGROUND);
    assert_eq!(job::enqueue(&queue, Vec::new()).unwrap(), None);
    release.send(()).unwrap();
    assert_eq!(s.wait(busy), JobStatus::Done);
    assert_eq!(s.wait(id), JobStatus::Done);
    queue.shutdown();
}
