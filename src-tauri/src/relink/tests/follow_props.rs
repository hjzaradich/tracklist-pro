//! What a follow promises, over any sequence of walks, moves, returns,
//! hashes, grouping passes, rankings, reads of rekordbox, relink jobs, adds,
//! removals and undos (1bE-10). Each case builds a small made-up library
//! and runs the app's own grouping pass, ranking, relink, follow step, add,
//! removal and undo over it; only the walk, the hash stage and the read of
//! rekordbox are stood in for by rows written as they would leave them.
//!
//! After every step:
//! - no file is the linked file of two Library tracks;
//! - a Library track's linked file is a file of its own track;
//! - a Library track's linked file never holds other audio than before
//!   (nothing but the follow moves a link, and it only moves to the same
//!   audio);
//! - no rekordbox entry is the source of two Library tracks in one send,
//!   also at the moment a track that owns the file an entry is matched to
//!   is added to the Library, and when it is removed again.
//!
//! And around every follow step:
//! - a link only moved from a file that is gone to a file on disk with the
//!   same `audio_hash`, current for both;
//! - the automatic follow never changes which rekordbox entry a track is
//!   sent as: a followed track that was sent as an entry is sent as that
//!   entry; one that was sent as none is sent as none, or as the one entry
//!   matched to the file it followed to, which no other track was sent as;
//!   and every other Library track is sent exactly as before;
//! - running it again moves nothing.

use std::collections::{HashMap, HashSet};

use proptest::prelude::*;

use super::follow::Drives;
use super::*;
use crate::fragile::FragileDirs;
use crate::library::LibraryTrackId;
use crate::send_values::{sent_as, SentAs};

const NO_DIRS: FragileDirs = FragileDirs {
    downloads: None,
    temp: Vec::new(),
};

/// One step. An index picks a file (or a Library track) among those there
/// are, wrapping round.
#[derive(Debug, Clone)]
enum Op {
    /// A walk finds the file gone.
    Vanish(usize),
    /// A walk finds a gone file again, holding what it held.
    Return(usize),
    /// The file is moved: gone where it was, the same audio at a new place.
    Move(usize),
    /// A copy of the file appears elsewhere: the same audio.
    Copy(usize),
    /// Another recording appears elsewhere under the file's name and
    /// length: what relink's name steps can take for it.
    Lookalike(usize),
    /// Another encode of the file appears, and grouping merges the two.
    Encode(usize),
    /// The file changes on disk; its hash isn't current until hashed again.
    Stale(usize),
    /// The hash stage runs for the file: the same audio.
    Rehash(usize),
    /// A grouping pass.
    Group,
    /// The best-file ranking.
    Roles,
    /// A read of rekordbox: every row is back, unmatched.
    Read,
    /// rekordbox holds an entry at the file's own path.
    Entry(usize),
    /// The chain after a scan: a grouping pass, then the relink job (relink,
    /// then the follow step).
    Job,
    /// The relink job alone.
    JobUngrouped,
    /// "Add to Library" for the file's track.
    Add(usize),
    /// "Remove from Library".
    Remove(usize),
    /// Undo.
    Undo,
}

fn op() -> impl Strategy<Value = Op> {
    let i = 0usize..12;
    prop_oneof![
        3 => i.clone().prop_map(Op::Vanish),
        2 => i.clone().prop_map(Op::Return),
        3 => i.clone().prop_map(Op::Move),
        2 => i.clone().prop_map(Op::Copy),
        2 => i.clone().prop_map(Op::Lookalike),
        1 => i.clone().prop_map(Op::Encode),
        1 => i.clone().prop_map(Op::Stale),
        1 => i.clone().prop_map(Op::Rehash),
        1 => Just(Op::Group),
        1 => Just(Op::Roles),
        1 => Just(Op::Read),
        1 => i.clone().prop_map(Op::Entry),
        5 => Just(Op::Job),
        1 => Just(Op::JobUngrouped),
        2 => i.clone().prop_map(Op::Add),
        1 => i.prop_map(Op::Remove),
        1 => Just(Op::Undo),
    ]
}

/// A file the model knows.
#[derive(Debug, Clone)]
struct ModelFile {
    id: i64,
    /// The byte its `audio_hash` is made of.
    audio: u8,
    /// Its path in the music folder.
    rel: String,
    duration_ms: i64,
    present: bool,
}

struct World {
    lib: Lib,
    music: i64,
    mounted: Mounted,
    files: Vec<ModelFile>,
    /// Paths rekordbox holds an entry at.
    entries: HashSet<String>,
    /// For names and audio nothing else has.
    next: u32,
    /// Each Library track's linked audio, as last seen.
    linked_audio: HashMap<i64, Vec<u8>>,
}

impl World {
    /// Three recordings with one file each, grouped. rekordbox holds the
    /// first and the third at their paths; the first and the second are in
    /// the Library.
    fn new() -> World {
        let (lib, music, mounted) = e_music();
        let mut world = World {
            lib,
            music,
            mounted,
            files: Vec::new(),
            entries: HashSet::new(),
            next: 0,
            linked_audio: HashMap::new(),
        };
        for n in 0..3u8 {
            let rel = format!("S{n}/Song {n}.mp3");
            world.new_file(rel, n + 1, 200_000 + i64::from(n) * 20_000);
        }
        world.lib.regroup();
        world.entry(0);
        world.entry(2);
        world.lib.relink(&world.mounted);
        world.add(0);
        world.add(1);
        world.snapshot();
        world.check("the start");
        world
    }

    fn new_file(&mut self, rel: String, audio: u8, duration_ms: i64) -> usize {
        let id = self.lib.file(self.music, &rel, Some(duration_ms));
        self.lib.hashed(id, audio);
        self.files.push(ModelFile {
            id,
            audio,
            rel,
            duration_ms,
            present: true,
        });
        self.files.len() - 1
    }

    fn fresh(&mut self) -> u32 {
        self.next += 1;
        self.next
    }

    fn entry(&mut self, at: usize) {
        let file = self.files[at].clone();
        if self.entries.insert(file.rel.clone()) {
            let total = (file.duration_ms / 1000).to_string();
            self.lib.track_with(
                &loc(&format!("E:/Music/{}", file.rel)),
                Some(&total),
                &[("Name", "Kappa")],
            );
        }
    }

    fn add(&mut self, at: usize) {
        if let Some(recording) = self.lib.recording_of(self.files[at].id) {
            // Refused where the app refuses (the file isn't on disk, the
            // only match is probable): nothing changed then.
            let _ = crate::library::promote_with(&self.lib.writer, &Drives, &NO_DIRS, recording);
        }
    }

    fn library_tracks(&self) -> Vec<LibraryTrackId> {
        self.lib
            .writer
            .call(|c| {
                c.prepare("SELECT id FROM library_track ORDER BY id")?
                    .query_map([], |r| r.get(0).map(LibraryTrackId))?
                    .collect()
            })
            .unwrap()
    }

    /// What every Library track is sent as, in one send.
    fn sent(&self) -> Vec<(LibraryTrackId, SentAs)> {
        let tracks = self.library_tracks();
        let asked: Vec<(LibraryTrackId, Option<i64>)> = tracks.iter().map(|t| (*t, None)).collect();
        let sent = self
            .lib
            .writer
            .call(move |c| sent_as(c, &Drives, &asked))
            .unwrap();
        tracks.into_iter().zip(sent).collect()
    }

    /// Each Library track's linked file's audio now.
    fn audio_now(&self) -> HashMap<i64, Vec<u8>> {
        self.lib
            .writer
            .call(|c| {
                c.prepare(
                    "SELECT lt.id, f.audio_hash FROM library_track lt
                     JOIN file f ON f.id = lt.linked_file_id",
                )?
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect()
            })
            .unwrap()
    }

    fn snapshot(&mut self) {
        self.linked_audio = self.audio_now();
    }

    fn count(&self, sql: &'static str) -> i64 {
        self.lib
            .writer
            .call(move |c| c.query_row(sql, [], |r| r.get(0)))
            .unwrap()
    }

    /// The rekordbox rows matched to a file, whatever the match.
    fn entries_matched_to(&self, file: i64) -> Vec<i64> {
        self.lib
            .writer
            .call(move |c| {
                c.prepare("SELECT id FROM rekordbox_track WHERE file_id = ?1 ORDER BY id")?
                    .query_map([file], |r| r.get(0))?
                    .collect()
            })
            .unwrap()
    }

    /// What holds after every step.
    fn check(&self, after: &str) {
        assert_eq!(
            self.count(
                "SELECT count(*) FROM (SELECT 1 FROM library_track
                                       WHERE linked_file_id IS NOT NULL
                                       GROUP BY linked_file_id HAVING count(*) > 1)"
            ),
            0,
            "a file is linked by two Library tracks after {after}"
        );
        assert_eq!(
            self.count(
                "SELECT count(*) FROM library_track lt
                 WHERE lt.linked_file_id IS NOT NULL
                   AND NOT EXISTS (SELECT 1 FROM recording_file rf
                                   WHERE rf.file_id = lt.linked_file_id
                                     AND rf.recording_id = lt.recording_id)"
            ),
            0,
            "a linked file isn't a file of its own track after {after}"
        );
        for (track, audio) in self.audio_now() {
            if let Some(before) = self.linked_audio.get(&track) {
                assert_eq!(
                    &audio, before,
                    "Library track {track} is linked to other audio after {after}"
                );
            }
        }
        let mut entries = HashSet::new();
        for (track, sent) in self.sent() {
            if let SentAs::Entry(row) = sent {
                assert!(
                    entries.insert(row),
                    "rekordbox entry {row} is the source of two Library tracks \
                     (one of them {track:?}) after {after}"
                );
            }
        }
    }

    /// The relink job: relink, then the follow step, with what a follow
    /// promises checked around it.
    fn job(&mut self) {
        self.lib.relink(&self.mounted);
        let before: HashMap<LibraryTrackId, SentAs> = self.sent().into_iter().collect();
        let follows = self.lib.follow();
        let after: HashMap<LibraryTrackId, SentAs> = self.sent().into_iter().collect();
        let followed: HashSet<LibraryTrackId> =
            follows.followed.iter().map(|f| f.library_track).collect();
        for (track, was) in &before {
            let now = after[track];
            if followed.contains(track) {
                match was {
                    SentAs::Entry(_) => assert_eq!(
                        now, *was,
                        "the follow changed which rekordbox entry {track:?} is sent as"
                    ),
                    // Its file was gone and it was sent as no entry. If it
                    // is on one now, that is the only entry matched to the
                    // file it followed to, and nobody else's.
                    SentAs::Nothing => {
                        if let SentAs::Entry(entry) = now {
                            let to = follows
                                .followed
                                .iter()
                                .find(|f| f.library_track == *track)
                                .map(|f| f.to)
                                .unwrap();
                            assert_eq!(
                                self.entries_matched_to(to),
                                vec![entry],
                                "{track:?} followed onto a file with a second entry"
                            );
                            assert!(
                                !before.values().any(|was| *was == SentAs::Entry(entry)),
                                "{track:?} followed onto another track's entry"
                            );
                        }
                    }
                    SentAs::New => panic!("{track:?} followed though its file was on disk"),
                }
            } else {
                assert_eq!(
                    now, *was,
                    "the follow changed what {track:?}, which didn't follow, is sent as"
                );
            }
        }
        for follow in &follows.followed {
            let (from, to) = (follow.from, follow.to);
            let right: bool = self
                .lib
                .writer
                .call(move |c| {
                    c.query_row(
                        &format!(
                            "SELECT a.present = 0 AND n.present = 1
                                    AND length(a.audio_hash) > 0 AND a.audio_hash = n.audio_hash
                                    AND {} AND {}
                             FROM file a, file n WHERE a.id = ?2 AND n.id = ?3",
                            crate::relink::hash_current_of("a"),
                            crate::relink::hash_current_of("n")
                        ),
                        [crate::relink::hash_version(), from, to],
                        |r| r.get(0),
                    )
                })
                .unwrap();
            assert!(
                right,
                "{follow:?} wasn't from a gone file to the same audio on disk"
            );
        }
        assert!(
            self.lib.follow().followed.is_empty(),
            "a second follow run moved a link"
        );
    }

    fn apply(&mut self, op: &Op) {
        let len = self.files.len();
        let at = |i: usize| i % len;
        match *op {
            Op::Vanish(i) => {
                let i = at(i);
                if self.files[i].present {
                    self.lib.gone(self.files[i].id);
                    self.files[i].present = false;
                }
            }
            Op::Return(i) => {
                let i = at(i);
                if !self.files[i].present {
                    self.lib.back(self.files[i].id);
                    self.files[i].present = true;
                }
            }
            Op::Move(i) => {
                let i = at(i);
                if self.files[i].present {
                    self.lib.gone(self.files[i].id);
                    self.files[i].present = false;
                    let n = self.fresh();
                    let (audio, duration) = (self.files[i].audio, self.files[i].duration_ms);
                    self.new_file(format!("M{n}/Moved {n}.mp3"), audio, duration);
                }
            }
            Op::Copy(i) => {
                let i = at(i);
                let n = self.fresh();
                let (audio, duration) = (self.files[i].audio, self.files[i].duration_ms);
                self.new_file(format!("C{n}/Copy {n}.mp3"), audio, duration);
            }
            Op::Lookalike(i) => {
                let i = at(i);
                let n = self.fresh();
                let name = self.files[i].rel.rsplit('/').next().unwrap().to_owned();
                let duration = self.files[i].duration_ms;
                self.new_file(format!("L{n}/{name}"), 100 + n as u8, duration);
            }
            Op::Encode(i) => {
                let i = at(i);
                if self.lib.recording_of(self.files[i].id).is_some() {
                    let n = self.fresh();
                    let duration = self.files[i].duration_ms;
                    let new =
                        self.new_file(format!("E{n}/Encode {n}.flac"), 100 + n as u8, duration);
                    self.lib.title(self.files[new].id, "Kappa");
                    self.lib.joins(self.files[new].id, self.files[i].id);
                }
            }
            Op::Stale(i) => {
                let i = at(i);
                if self.files[i].present {
                    self.lib.edited(self.files[i].id);
                }
            }
            Op::Rehash(i) => {
                let i = at(i);
                if self.files[i].present {
                    self.lib.hashed(self.files[i].id, self.files[i].audio);
                }
            }
            Op::Group => self.lib.regroup(),
            Op::Roles => self.lib.roles(),
            Op::Read => self.lib.fresh_read(),
            Op::Entry(i) => self.entry(at(i)),
            Op::Job => {
                self.lib.regroup();
                self.job();
            }
            Op::JobUngrouped => self.job(),
            Op::Add(i) => {
                self.add(at(i));
                self.snapshot();
            }
            Op::Remove(i) => {
                let tracks = self.library_tracks();
                if !tracks.is_empty() {
                    let _ = crate::library::remove(&self.lib.writer, tracks[i % tracks.len()]);
                }
                self.snapshot();
            }
            Op::Undo => {
                let _ = crate::ops::undo_last_via(&self.lib.writer).unwrap();
                self.snapshot();
            }
        }
        self.check(&format!("{op:?}"));
    }
}

proptest! {
    // Each case builds a database, so fewer cases than the default 256.
    // A failure isn't written to a regressions file: the case is printed.
    #![proptest_config(ProptestConfig {
        cases: 32,
        failure_persistence: None,
        ..ProptestConfig::default()
    })]

    #[test]
    fn no_sequence_of_scans_moves_returns_and_follows_breaks_what_a_follow_promises(
        scripted in 0..3u8,
        ops in proptest::collection::vec(op(), 1..28),
    ) {
        let mut world = World::new();
        if scripted == 1 {
            // Start where an entry stands in for a gone file while relink
            // has matched it to a file of another track: the moment the
            // tracks around it are added and removed is what follows.
            for op in [Op::Lookalike(0), Op::Vanish(0), Op::Job] {
                world.apply(&op);
            }
        }
        if scripted == 2 {
            // Start with a file moved away and back again: the first track
            // (which rekordbox holds an entry for) follows there and back,
            // and is sent as the entry it was sent as at the start.
            let first = world.library_tracks()[0];
            let sent_as_now = |world: &World| {
                world.sent().into_iter().find(|(t, _)| *t == first).map(|(_, s)| s)
            };
            let at_the_start = sent_as_now(&world);
            prop_assert!(matches!(at_the_start, Some(SentAs::Entry(_))));
            world.apply(&Op::Move(0));
            world.apply(&Op::Job);
            let moved = world.files.len() - 1;
            prop_assert_eq!(world.lib.linked_file(first), world.files[moved].id);
            prop_assert_eq!(sent_as_now(&world), at_the_start);
            for op in [Op::Vanish(moved), Op::Return(0), Op::Job] {
                world.apply(&op);
            }
            prop_assert_eq!(world.lib.linked_file(first), world.files[0].id);
            prop_assert_eq!(sent_as_now(&world), at_the_start);
        }
        for op in &ops {
            world.apply(op);
        }
        // Whatever state it ends in, the chain settles it.
        world.apply(&Op::Job);
    }
}
