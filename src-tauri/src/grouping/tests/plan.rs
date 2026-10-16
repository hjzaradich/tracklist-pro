//! The pure rule: which files go where, and where each decided pair stands.

use std::collections::HashSet;

use crate::grouping::plan::{
    plan, Dest, Input, Link, Member, Outcome, Pair, Placed, Plan, Remembered, State, Verdict,
};

fn key(k: u8) -> Vec<u8> {
    vec![k; 34]
}

/// A file from before placements were recorded.
fn member(file: i64, recording: Option<i64>, k: Option<u8>) -> Member {
    Member {
        file,
        recording,
        key: k.map(key),
        content: k.map(key),
        present: true,
        pinned: false,
        placed: Placed::Unknown,
        decided: true,
    }
}

/// A file in a track, placed there holding what it holds now.
fn placed(file: i64, recording: i64, k: u8) -> Member {
    Member {
        placed: Placed::At(Some(key(k))),
        ..member(file, Some(recording), Some(k))
    }
}

/// A file with no track yet.
fn new(file: i64, k: u8) -> Member {
    member(file, None, Some(k))
}

/// The file holds audio `k` now, whatever it was placed with.
fn holds(m: &mut Member, k: u8) {
    m.key = Some(key(k));
    m.content = Some(key(k));
}

/// A file in a track that the hash stage can't hash: no `audio_hash`, and
/// whole-file hash `b` when it was placed and now.
fn unhashed(file: i64, recording: i64, b: u8) -> Member {
    Member {
        key: None,
        content: Some(key(b)),
        placed: Placed::At(Some(key(b))),
        ..member(file, Some(recording), None)
    }
}

fn pair(a: i64, b: i64, outcome: Outcome) -> Pair {
    Pair { a, b, outcome }
}

/// Everything a plan is made from.
#[derive(Default)]
struct World {
    members: Vec<Member>,
    pairs: Vec<Pair>,
    remembered: Vec<Remembered>,
    links: Vec<Link>,
    library: HashSet<i64>,
    referenced: HashSet<i64>,
    max_set_files: Option<usize>,
}

impl World {
    fn of(members: impl IntoIterator<Item = Member>) -> World {
        World {
            members: members.into_iter().collect(),
            ..World::default()
        }
    }

    fn pairs(mut self, pairs: impl IntoIterator<Item = Pair>) -> World {
        self.pairs = pairs.into_iter().collect();
        self.pairs.sort_by_key(|p| (p.a, p.b));
        self
    }

    fn plan(&self) -> Plan {
        plan(&Input {
            members: &self.members,
            pairs: &self.pairs,
            remembered: &self.remembered,
            links: &self.links,
            library: &self.library,
            referenced: &self.referenced,
            max_set_files: self.max_set_files.unwrap_or(50),
        })
    }

    /// Where the plan puts file `file`.
    fn dest(&self, plan: &Plan, file: i64) -> Dest {
        if let Some(m) = plan.moves.iter().find(|m| m.file == file) {
            return m.to;
        }
        let track = self.members.iter().find(|m| m.file == file).unwrap();
        Dest::Recording(
            track
                .recording
                .expect("a file that doesn't move has a track"),
        )
    }

    /// Where the plan says the pair of files `a` and `b` stands.
    fn state(&self, plan: &Plan, a: i64, b: i64) -> State {
        let at = self
            .pairs
            .iter()
            .position(|p| (p.a, p.b) == (a, b))
            .unwrap();
        plan.states[at].expect("both files are members")
    }
}

// ---- tracks from before placements were recorded -----------------------

#[test]
fn a_track_from_before_keeps_the_audio_most_of_its_files_share() {
    // Files 1 and 2 share audio 7; file 3 changed to 9.
    let world = World::of([
        member(1, Some(1), Some(7)),
        member(2, Some(1), Some(7)),
        member(3, Some(1), Some(9)),
    ]);
    let p = world.plan();
    assert_eq!(p.moves.len(), 1);
    assert_eq!((p.moves[0].file, p.moves[0].from), (3, Some(1)));
    assert!(matches!(p.moves[0].to, Dest::New(_)));
}

#[test]
fn a_tie_goes_to_the_hash_of_the_lowest_file_id_so_a_rerun_never_flips_it() {
    let world = World::of([member(5, Some(1), Some(9)), member(4, Some(1), Some(7))]);
    let p = world.plan();
    assert_eq!(p.moves.len(), 1);
    assert_eq!(p.moves[0].file, 5, "file 4's audio stays");
}

#[test]
fn unplaced_files_with_the_same_hash_share_one_new_track_and_others_get_their_own() {
    let world = World::of([
        member(1, None, Some(7)),
        member(2, None, Some(7)),
        member(3, None, None),
        member(4, None, None),
    ]);
    let p = world.plan();
    assert_eq!(p.new_recordings, 3);
    assert_eq!(world.dest(&p, 1), world.dest(&p, 2));
    assert_ne!(world.dest(&p, 3), world.dest(&p, 4));
    assert_ne!(world.dest(&p, 1), world.dest(&p, 3));
}

#[test]
fn a_plan_for_settled_files_moves_nothing_and_has_nothing_to_record() {
    let world = World::of([placed(1, 1, 7), placed(2, 1, 7), placed(3, 2, 8)]);
    assert_eq!(world.plan(), Plan::default());
    // Files from before placements were recorded stay too; what they hold
    // is recorded as what they were placed at.
    let before = World::of([
        member(1, Some(1), Some(7)),
        member(2, Some(1), Some(7)),
        member(3, Some(2), None),
    ]);
    let p = before.plan();
    assert!(p.moves.is_empty());
    assert_eq!(p.settle, [1, 2, 3]);
}

#[test]
fn planning_scales_linearly_with_the_number_of_files() {
    // 55,000 files: 50,000 distinct audio, 5,000 of them twice. It runs on
    // the database's one writer, so a pass over every file for each
    // distinct hash (2.75 billion looks here, minutes at 100k files) can't
    // come back.
    let key = |n: u32| {
        let mut key = vec![0u8; 34];
        key[..4].copy_from_slice(&n.to_be_bytes());
        key
    };
    let world = World::of((0..55_000u32).map(|n| Member {
        file: i64::from(n),
        recording: None,
        key: Some(key(n % 50_000)),
        content: Some(key(n % 50_000)),
        present: true,
        pinned: false,
        placed: Placed::Unknown,
        decided: false,
    }));
    crate::grouping::plan::VISITS.with(|v| v.set(0));
    let p = world.plan();
    assert_eq!(p.moves.len(), 55_000);
    assert_eq!(p.new_recordings, 50_000);
    // Counted, not timed, so a busy machine can't fail it: a few looks per
    // file, however many distinct hashes there are.
    let visits = crate::grouping::plan::VISITS.with(|v| v.get());
    assert!(
        visits <= 8 * world.members.len() as u64,
        "{visits} looks at {} files",
        world.members.len()
    );
}

#[test]
fn a_merge_prefers_the_track_something_refers_to_then_the_lowest_id() {
    let mut world = World::of([placed(1, 1, 7), placed(2, 2, 7)]);
    world.referenced = [2].into();
    let p = world.plan();
    assert_eq!(p.moves.len(), 1);
    assert_eq!((p.moves[0].file, p.moves[0].to), (1, Dest::Recording(2)));
    world.referenced.clear();
    let p = world.plan();
    assert_eq!((p.moves[0].file, p.moves[0].to), (2, Dest::Recording(1)));
}

// ---- a file whose audio changed -----------------------------------------

#[test]
fn a_file_that_holds_other_audio_than_it_was_placed_at_leaves_its_track() {
    let mut world = World::of([placed(1, 1, 7), placed(2, 1, 7), placed(3, 2, 9)]);
    // File 2 was placed holding 7 and holds 9 now.
    holds(&mut world.members[1], 9);
    let p = world.plan();
    assert_eq!(p.moves.len(), 1);
    assert_eq!((p.moves[0].file, p.moves[0].to), (2, Dest::Recording(2)));
}

#[test]
fn when_every_file_of_a_track_holds_other_audio_the_lowest_stays_as_the_track() {
    let mut world = World::of([placed(4, 1, 7), placed(5, 1, 7)]);
    holds(&mut world.members[0], 8);
    holds(&mut world.members[1], 9);
    let p = world.plan();
    assert_eq!(p.moves.len(), 1);
    assert_eq!(p.moves[0].file, 5);
    assert!(matches!(p.moves[0].to, Dest::New(_)));
}

#[test]
fn when_a_pinned_file_holds_other_audio_the_track_is_its_and_the_others_leave() {
    let mut world = World::of([placed(1, 1, 7), placed(2, 1, 7), placed(3, 1, 7)]);
    world.library = [1].into();
    world.members[0].pinned = true;
    holds(&mut world.members[0], 9);
    let p = world.plan();
    let moved: Vec<i64> = p.moves.iter().map(|m| m.file).collect();
    assert_eq!(moved, [2, 3], "the pinned file never moves");
    // The two that left still hold the same audio: one new track.
    assert_eq!(world.dest(&p, 2), world.dest(&p, 3));
    assert_eq!(p.new_recordings, 1);
    // Its new audio is recorded only once the others have left.
    assert!(p.settle.is_empty());
}

#[test]
fn a_file_with_no_audio_hash_leaves_its_track_when_its_whole_file_hash_changes() {
    // File 2 has no audio_hash: it joined file 1 as a duplicate, known by
    // its whole-file hash.
    let mut world =
        World::of([placed(1, 1, 7), unhashed(2, 1, 20)]).pairs([pair(1, 2, Outcome::Duplicate)]);
    assert!(world.plan().moves.is_empty());
    // Overwritten: still no audio_hash, another whole-file hash. Its pairs
    // went with its audio (the database drops them), and it's due.
    world.members[1].content = Some(key(21));
    world.members[1].decided = false;
    world.pairs.clear();
    let p = world.plan();
    assert_eq!(p.moves.len(), 1);
    assert_eq!(p.moves[0].file, 2);
    assert!(matches!(p.moves[0].to, Dest::New(_)));
}

#[test]
fn a_pinned_file_with_no_audio_hash_that_was_overwritten_keeps_its_track_to_itself() {
    let mut world = World::of([unhashed(1, 1, 20), placed(2, 1, 7)]);
    world.library = [1].into();
    world.members[0].pinned = true;
    world.members[0].content = Some(key(21));
    let p = world.plan();
    let moved: Vec<i64> = p.moves.iter().map(|m| m.file).collect();
    assert_eq!(moved, [2], "the pinned file never moves; the other leaves");
}

#[test]
fn a_pinned_file_that_lost_its_hash_keeps_the_files_beside_it() {
    let mut world = World::of([placed(1, 1, 7), placed(2, 1, 7)]);
    world.library = [1].into();
    world.members[0].pinned = true;
    world.members[0].key = None;
    world.members[0].content = None;
    let p = world.plan();
    assert!(p.moves.is_empty());
}

// ---- sets joined by duplicate pairs ---------------------------------------

#[test]
fn two_files_the_rules_call_duplicates_share_a_track() {
    let world =
        World::of([placed(1, 1, 7), placed(2, 2, 8)]).pairs([pair(1, 2, Outcome::Duplicate)]);
    let p = world.plan();
    assert_eq!(world.dest(&p, 2), Dest::Recording(1));
    assert_eq!(world.state(&p, 1, 2), State::Merged);
}

#[test]
fn a_set_merges_only_if_every_pair_in_it_is_a_duplicate() {
    // 1 and 2 are duplicates, 2 and 3 are duplicates, 1 and 3 need review.
    let world = World::of([placed(1, 1, 7), placed(2, 2, 8), placed(3, 3, 9)]).pairs([
        pair(1, 2, Outcome::Duplicate),
        pair(2, 3, Outcome::Duplicate),
        pair(1, 3, Outcome::Review),
    ]);
    let p = world.plan();
    assert!(p.moves.is_empty(), "nothing in the set merges");
    assert_eq!(p.sets_disagreeing, 1);
    assert_eq!(world.state(&p, 1, 2), State::SetDisagrees);
    assert_eq!(world.state(&p, 2, 3), State::SetDisagrees);
    assert_eq!(world.state(&p, 1, 3), State::Open);
    assert!(p.missing.is_empty());
}

#[test]
fn a_cut_a_rework_or_a_different_pair_inside_a_set_holds_it_too() {
    for odd in [Outcome::Cut, Outcome::Rework, Outcome::Different] {
        let world = World::of([placed(1, 1, 7), placed(2, 2, 8), placed(3, 3, 9)]).pairs([
            pair(1, 2, Outcome::Duplicate),
            pair(2, 3, Outcome::Duplicate),
            pair(1, 3, odd),
        ]);
        let p = world.plan();
        assert!(p.moves.is_empty(), "{odd:?}");
        assert_eq!(p.sets_disagreeing, 1, "{odd:?}");
    }
}

#[test]
fn a_pair_nobody_decided_makes_the_set_wait_and_is_listed_to_be_decided() {
    let world = World::of([placed(1, 1, 7), placed(2, 2, 8), placed(3, 3, 9)]).pairs([
        pair(1, 2, Outcome::Duplicate),
        pair(2, 3, Outcome::Duplicate),
    ]);
    let p = world.plan();
    assert!(p.moves.is_empty());
    assert_eq!((p.sets_waiting, p.sets_disagreeing), (1, 0));
    assert_eq!(p.missing, [(1, 3)]);
    assert_eq!(world.state(&p, 1, 2), State::Waiting);
    // Once it's decided a duplicate, all three merge.
    let world = world.pairs([
        pair(1, 2, Outcome::Duplicate),
        pair(2, 3, Outcome::Duplicate),
        pair(1, 3, Outcome::Duplicate),
    ]);
    let p = world.plan();
    assert_eq!(world.dest(&p, 2), Dest::Recording(1));
    assert_eq!(world.dest(&p, 3), Dest::Recording(1));
}

#[test]
fn files_with_the_very_same_audio_need_no_decided_pair_inside_a_set() {
    // 1 and 2 hold the same audio; 3 is a duplicate of both by the rules.
    let world = World::of([placed(1, 1, 7), placed(2, 2, 7), placed(3, 3, 9)]).pairs([
        pair(1, 3, Outcome::Duplicate),
        pair(2, 3, Outcome::Duplicate),
    ]);
    let p = world.plan();
    assert_eq!(world.dest(&p, 2), Dest::Recording(1));
    assert_eq!(world.dest(&p, 3), Dest::Recording(1));
}

#[test]
fn files_with_the_very_same_audio_stay_together_when_their_set_is_held() {
    // 1 and 2 hold the same audio; 3 is a duplicate of 1 and needs review
    // against 2. The set is held, but 1 and 2 are one track all the same.
    let world = World::of([new(1, 7), new(2, 7), new(3, 9)])
        .pairs([pair(1, 3, Outcome::Duplicate), pair(2, 3, Outcome::Review)]);
    let mut world = world;
    for m in &mut world.members {
        m.decided = true;
    }
    let p = world.plan();
    assert_eq!(world.dest(&p, 1), world.dest(&p, 2));
    assert_ne!(world.dest(&p, 1), world.dest(&p, 3));
    assert_eq!(p.sets_disagreeing, 1);
}

#[test]
fn a_set_with_more_files_than_the_cap_is_held_as_if_it_disagreed() {
    let files = [placed(1, 1, 1), placed(2, 2, 2), placed(3, 3, 3)];
    let all = [
        pair(1, 2, Outcome::Duplicate),
        pair(1, 3, Outcome::Duplicate),
        pair(2, 3, Outcome::Duplicate),
    ];
    let mut world = World::of(files).pairs(all);
    // At the cap it merges.
    world.max_set_files = Some(3);
    assert_eq!(world.plan().moves.len(), 2);
    // One over, nothing does.
    world.max_set_files = Some(2);
    let p = world.plan();
    assert!(p.moves.is_empty());
    assert_eq!(p.sets_disagreeing, 1);
    assert_eq!(world.state(&p, 1, 2), State::SetDisagrees);
    assert!(
        p.missing.is_empty(),
        "nothing is asked about a set that big"
    );
}

#[test]
fn any_number_of_files_with_the_very_same_audio_share_a_track_whatever_the_cap() {
    let mut world = World::of((1..=200).map(|file| new(file, 7)));
    world.max_set_files = Some(2);
    let p = world.plan();
    assert_eq!(p.new_recordings, 1);
    assert_eq!(p.moves.len(), 200);
}

#[test]
fn the_pairs_of_a_file_that_is_due_are_not_relied_on() {
    let mut world =
        World::of([placed(1, 1, 7), placed(2, 2, 8)]).pairs([pair(1, 2, Outcome::Duplicate)]);
    world.members[1].decided = false;
    let p = world.plan();
    assert!(p.moves.is_empty());
    assert_eq!(world.state(&p, 1, 2), State::Waiting);
    assert!(
        p.missing.is_empty(),
        "it's decided with the file, not on top"
    );
}

#[test]
fn a_due_files_duplicate_pair_keeps_its_set_waiting_and_merges_nothing() {
    // Every pair is a duplicate, but file 3 is due: what is stored about it
    // was decided on names or audio it no longer has.
    let mut world = World::of([placed(1, 1, 7), placed(2, 2, 8), placed(3, 3, 9)]).pairs([
        pair(1, 2, Outcome::Duplicate),
        pair(1, 3, Outcome::Duplicate),
        pair(2, 3, Outcome::Duplicate),
    ]);
    world.members[2].decided = false;
    let p = world.plan();
    assert!(p.moves.is_empty(), "not even files 1 and 2 merge");
    assert_eq!(p.sets_waiting, 1);
    assert!(p.missing.is_empty(), "nothing to decide but the file");
    for (a, b) in [(1, 2), (1, 3), (2, 3)] {
        assert_eq!(world.state(&p, a, b), State::Waiting);
    }
    // Decided again, with the same answers: one track.
    world.members[2].decided = true;
    assert_eq!(world.plan().moves.len(), 2);
}

#[test]
fn a_held_set_stays_held_while_one_of_its_files_is_due() {
    // 1-2 and 2-3 are duplicates, 1-3 is for review: nothing merges. A tag
    // rewrite then makes file 3 due.
    let mut world = World::of([placed(1, 1, 7), placed(2, 2, 8), placed(3, 3, 9)]).pairs([
        pair(1, 2, Outcome::Duplicate),
        pair(1, 3, Outcome::Review),
        pair(2, 3, Outcome::Duplicate),
    ]);
    let held = world.plan();
    world.members[2].decided = false;
    let p = world.plan();
    assert!(
        p.moves.is_empty(),
        "files 1 and 2 don't merge behind its back"
    );
    assert_eq!(
        p.states, held.states,
        "and every pair stands where it stood"
    );
    assert_eq!(world.state(&p, 1, 2), State::SetDisagrees);
    assert_eq!(world.state(&p, 2, 3), State::SetDisagrees);
    assert_eq!(world.state(&p, 1, 3), State::Open);
    assert_eq!((p.sets_disagreeing, p.sets_waiting), (1, 0));
}

// ---- a file that isn't on disk ------------------------------------------------

#[test]
fn a_file_that_is_not_on_disk_holds_no_newcomer_out_of_its_track() {
    // Track 1 holds files 1 and 2; file 2 was deleted from disk. File 3
    // arrives, a duplicate of file 1. Nothing can be decided between 2 and
    // 3, ever.
    let mut world = World::of([placed(1, 1, 7), placed(2, 1, 8), new(3, 9)]).pairs([
        pair(1, 2, Outcome::Duplicate),
        pair(1, 3, Outcome::Duplicate),
    ]);
    world.members[1].present = false;
    let p = world.plan();
    assert_eq!(p.moves.len(), 1);
    assert_eq!(world.dest(&p, 3), Dest::Recording(1), "the newcomer joins");
    assert_eq!(
        world.dest(&p, 2),
        Dest::Recording(1),
        "the missing file stays"
    );
    assert_eq!(world.state(&p, 1, 3), State::Merged);
    assert!(p.missing.is_empty());
    assert_eq!(p.sets_waiting, 0);
}

#[test]
fn a_pair_with_a_file_that_is_not_on_disk_joins_no_set_and_waits_for_the_file() {
    // Files 1 and 2 were called duplicates; file 1 is gone from disk.
    let mut world =
        World::of([placed(1, 1, 7), placed(2, 2, 8)]).pairs([pair(1, 2, Outcome::Duplicate)]);
    world.members[0].present = false;
    let p = world.plan();
    assert!(p.moves.is_empty());
    assert_eq!(world.state(&p, 1, 2), State::Waiting);
    assert_eq!(
        (p.sets_waiting, p.sets_disagreeing, p.missing.len()),
        (0, 0, 0),
        "no set, nothing owed"
    );
    // A pair of its that isn't a duplicate holds nothing either.
    let mut world = World::of([placed(1, 1, 7), placed(2, 2, 8), placed(3, 3, 9)]).pairs([
        pair(1, 2, Outcome::Review),
        pair(1, 3, Outcome::Duplicate),
        pair(2, 3, Outcome::Duplicate),
    ]);
    world.members[0].present = false;
    let p = world.plan();
    assert_eq!(
        world.dest(&p, 2),
        world.dest(&p, 3),
        "the two on disk merge"
    );
    assert_eq!(world.dest(&p, 1), Dest::Recording(1));
}

// ---- merges are kept --------------------------------------------------------

#[test]
fn a_newcomer_never_splits_a_track_it_only_stays_apart() {
    // 1 and 2 are one track. 3 arrives: a duplicate of 1, a review against
    // 2.
    let world = World::of([placed(1, 1, 7), placed(2, 1, 8), new(3, 9)]).pairs([
        pair(1, 2, Outcome::Duplicate),
        pair(1, 3, Outcome::Duplicate),
        pair(2, 3, Outcome::Review),
    ]);
    let mut world = world;
    world.members[2].decided = true;
    let p = world.plan();
    assert_eq!(p.moves.len(), 1, "only the newcomer is placed");
    assert!(matches!(world.dest(&p, 3), Dest::New(_)));
    assert_eq!(world.state(&p, 1, 2), State::Merged);
    assert_eq!(world.state(&p, 1, 3), State::SetDisagrees);
    assert_eq!(world.state(&p, 2, 3), State::Open);
}

#[test]
fn a_pair_inside_a_track_that_is_no_longer_a_duplicate_is_marked_and_the_track_stays_whole() {
    // A retag: the rules now send the pair to review.
    for now in [
        Outcome::Review,
        Outcome::Cut,
        Outcome::Rework,
        Outcome::Different,
    ] {
        let world = World::of([placed(1, 1, 7), placed(2, 1, 8)]).pairs([pair(1, 2, now)]);
        let p = world.plan();
        assert!(p.moves.is_empty(), "{now:?}");
        assert_eq!(world.state(&p, 1, 2), State::InOneTrack, "{now:?}");
        assert!(p.wanted_links.is_empty(), "no link from a track to itself");
    }
}

// ---- the Library --------------------------------------------------------------

#[test]
fn two_tracks_that_are_both_in_the_library_never_merge_whatever_the_pair_says() {
    for (k1, k2) in [(7, 7), (7, 8)] {
        let mut world =
            World::of([placed(1, 1, k1), placed(2, 2, k2)]).pairs([pair(1, 2, Outcome::Duplicate)]);
        world.library = [1, 2].into();
        world.members[0].pinned = true;
        world.members[1].pinned = true;
        let p = world.plan();
        assert!(p.moves.is_empty());
        assert_eq!(p.sets_both_in_library, 1);
        assert_eq!(world.state(&p, 1, 2), State::BothInLibrary);
    }
}

#[test]
fn a_file_outside_the_library_joins_the_library_track_not_the_other_way_round() {
    let mut world =
        World::of([placed(1, 1, 7), placed(2, 2, 8)]).pairs([pair(1, 2, Outcome::Duplicate)]);
    world.library = [2].into();
    world.members[1].pinned = true;
    let p = world.plan();
    assert_eq!(p.moves.len(), 1);
    assert_eq!((p.moves[0].file, p.moves[0].to), (1, Dest::Recording(2)));
}

#[test]
fn a_third_copy_of_two_library_tracks_joins_neither() {
    let mut world = World::of([placed(1, 1, 7), placed(2, 2, 7), new(3, 7)]);
    world.library = [1, 2].into();
    let p = world.plan();
    assert_eq!(p.moves.len(), 1);
    assert!(matches!(world.dest(&p, 3), Dest::New(_)));
}

// ---- what the user decided ---------------------------------------------------

fn remembered(a: u8, b: u8, verdict: Verdict) -> Remembered {
    Remembered {
        key_a: key(a.min(b)),
        key_b: key(a.max(b)),
        verdict,
    }
}

#[test]
fn tracks_the_user_decided_apart_never_merge_and_their_pairs_are_settled() {
    // Two tracks of two files each. The user decided on files 1 and 3;
    // every pair between the two tracks is settled by it.
    let mut world = World::of([
        placed(1, 1, 1),
        placed(2, 1, 2),
        placed(3, 2, 3),
        placed(4, 2, 4),
    ])
    .pairs([
        pair(1, 2, Outcome::Duplicate),
        pair(1, 3, Outcome::Duplicate),
        pair(1, 4, Outcome::Duplicate),
        pair(2, 3, Outcome::Review),
        pair(2, 4, Outcome::Duplicate),
        pair(3, 4, Outcome::Duplicate),
    ]);
    world.remembered = vec![remembered(1, 3, Verdict::Apart)];
    let p = world.plan();
    assert!(p.moves.is_empty());
    for (a, b) in [(1, 3), (1, 4), (2, 3), (2, 4)] {
        assert_eq!(world.state(&p, a, b), State::Decided, "{a} and {b}");
    }
    assert_eq!(world.state(&p, 1, 2), State::Merged);
}

#[test]
fn tracks_the_user_called_the_same_merge_whatever_the_rules_say() {
    let mut world =
        World::of([placed(1, 1, 1), placed(2, 2, 2)]).pairs([pair(1, 2, Outcome::Review)]);
    world.remembered = vec![remembered(1, 2, Verdict::Same)];
    let p = world.plan();
    assert_eq!(world.dest(&p, 2), Dest::Recording(1));
    // With no pair stored at all, too.
    world.pairs.clear();
    assert_eq!(world.plan().moves.len(), 1);
}

#[test]
fn the_same_never_merges_two_library_tracks() {
    let mut world =
        World::of([placed(1, 1, 1), placed(2, 2, 2)]).pairs([pair(1, 2, Outcome::Duplicate)]);
    world.library = [1, 2].into();
    world.remembered = vec![remembered(1, 2, Verdict::Same)];
    let p = world.plan();
    assert!(p.moves.is_empty());
    assert_eq!(world.state(&p, 1, 2), State::Decided);
}

#[test]
fn a_decision_about_one_piece_of_audio_is_between_the_library_tracks_holding_it() {
    // "Keep both" for two Library tracks with the very same audio. The
    // decision isn't about a third copy outside the Library: it joins
    // neither, and its pairs are settled with the set (owner decision,
    // 2026-10-12, 1bE-9).
    let mut world = World::of([placed(1, 1, 7), placed(2, 2, 7), placed(3, 3, 7)]).pairs([
        pair(1, 2, Outcome::Duplicate),
        pair(1, 3, Outcome::Duplicate),
        pair(2, 3, Outcome::Duplicate),
    ]);
    world.library = [1, 2].into();
    world.remembered = vec![remembered(7, 7, Verdict::Apart)];
    let p = world.plan();
    assert!(p.moves.is_empty());
    assert_eq!(world.state(&p, 1, 2), State::Decided);
    assert_eq!(world.state(&p, 1, 3), State::Decided);
    assert_eq!(world.state(&p, 2, 3), State::Decided);
}

#[test]
fn a_copy_outside_the_library_is_held_until_its_library_tracks_are_kept_both() {
    // Library tracks 1 and 2 hold other audio than each other; 3 is a copy
    // outside. Before "Keep both" the copy's pairs wait in Review with the
    // set; after it they are settled and nothing moves.
    let mut world = World::of([placed(1, 1, 1), placed(2, 2, 2), placed(3, 3, 3)]).pairs([
        pair(1, 2, Outcome::Duplicate),
        pair(1, 3, Outcome::Duplicate),
        pair(2, 3, Outcome::Duplicate),
    ]);
    world.library = [1, 2].into();
    let p = world.plan();
    assert_eq!(world.state(&p, 1, 2), State::BothInLibrary);
    assert_eq!(world.state(&p, 1, 3), State::SetDisagrees);
    world.remembered = vec![remembered(1, 2, Verdict::Apart)];
    let p = world.plan();
    assert!(p.moves.is_empty());
    for (a, b) in [(1, 2), (1, 3), (2, 3)] {
        assert_eq!(world.state(&p, a, b), State::Decided, "{a} and {b}");
    }
    // A decision about the copy alone settles nothing between the Library
    // tracks.
    world.remembered = vec![remembered(1, 3, Verdict::Apart)];
    let p = world.plan();
    assert_eq!(world.state(&p, 1, 2), State::BothInLibrary);
    assert_eq!(world.state(&p, 2, 3), State::SetDisagrees);
}

// ---- version links --------------------------------------------------------------

#[test]
fn a_link_the_user_made_settles_its_two_tracks_and_gets_no_second_link() {
    let mut world =
        World::of([placed(1, 1, 1), placed(2, 2, 2)]).pairs([pair(1, 2, Outcome::Duplicate)]);
    world.links = vec![Link {
        a: 2,
        b: 1,
        settled: true,
    }];
    let p = world.plan();
    assert!(p.moves.is_empty());
    assert_eq!(world.state(&p, 1, 2), State::Decided);
    assert!(p.wanted_links.is_empty() && p.unsupported_links.is_empty());
    // A review or a cut between them isn't asked about again either.
    for outcome in [Outcome::Review, Outcome::Cut] {
        let world = World {
            pairs: vec![pair(1, 2, outcome)],
            links: world.links.clone(),
            members: world.members.clone(),
            ..World::default()
        };
        let p = world.plan();
        assert_eq!(world.state(&p, 1, 2), State::Decided);
        assert!(p.wanted_links.is_empty());
    }
}

#[test]
fn a_cut_or_a_rework_asks_for_one_link_between_its_two_tracks() {
    let world = World::of([placed(1, 1, 1), placed(2, 1, 2), placed(3, 2, 3)])
        .pairs([pair(1, 3, Outcome::Cut), pair(2, 3, Outcome::Rework)]);
    let p = world.plan();
    assert_eq!(p.wanted_links.len(), 1, "one link per pair of tracks");
    let wanted = p.wanted_links[0];
    assert_eq!(
        (wanted.a, wanted.b),
        (Dest::Recording(1), Dest::Recording(2))
    );
    assert_eq!(world.pairs[wanted.pair].outcome, Outcome::Cut, "the first");
    assert_eq!(world.state(&p, 1, 3), State::Linked);
    assert_eq!(world.state(&p, 2, 3), State::Linked);
}

#[test]
fn the_jobs_own_link_stands_only_while_a_pair_asks_for_it() {
    let mut world = World::of([placed(1, 1, 7), placed(2, 2, 7)]).pairs([pair(1, 2, Outcome::Cut)]);
    world.links = vec![Link {
        a: 1,
        b: 2,
        settled: false,
    }];
    // Asked for: it holds the two tracks apart, the same audio or not.
    let p = world.plan();
    assert!(p.moves.is_empty() && p.unsupported_links.is_empty());
    // No pair asks for it any more: it goes, and the tracks merge.
    world.pairs.clear();
    let p = world.plan();
    assert_eq!(p.unsupported_links, [(1, 2)]);
    assert_eq!(p.moves.len(), 1);
}

#[test]
fn the_jobs_own_link_gives_way_at_once_when_the_user_calls_the_two_tracks_the_same() {
    let mut world = World::of([placed(1, 1, 7), placed(2, 2, 8)]).pairs([pair(1, 2, Outcome::Cut)]);
    world.links = vec![Link {
        a: 1,
        b: 2,
        settled: false,
    }];
    world.remembered = vec![remembered(7, 8, Verdict::Same)];
    // One plan: the link goes and the file moves.
    let p = world.plan();
    assert_eq!(p.unsupported_links, [(1, 2)]);
    assert_eq!(p.moves.len(), 1);
    assert_eq!((p.left_alone, p.sets_disagreeing), (0, 0));
    assert_eq!(world.state(&p, 1, 2), State::InOneTrack);
    assert!(p.wanted_links.is_empty());
    // A link the user made stands, whatever they said later.
    world.links[0].settled = true;
    let p = world.plan();
    assert!(p.moves.is_empty() && p.unsupported_links.is_empty());
}

#[test]
fn a_file_is_never_moved_between_linked_versions() {
    // File 2 sits in track 1 and holds track 2's audio now; the two tracks
    // are linked versions.
    let mut world = World::of([placed(1, 1, 7), placed(2, 1, 7), placed(3, 2, 9)]);
    holds(&mut world.members[1], 9);
    world.links = vec![Link {
        a: 1,
        b: 2,
        settled: true,
    }];
    let p = world.plan();
    assert!(p.moves.is_empty());
    assert_eq!(p.left_alone, 1);
    // And it's tried again next time: nothing is recorded for it.
    assert!(!p.settle.contains(&2));
}
