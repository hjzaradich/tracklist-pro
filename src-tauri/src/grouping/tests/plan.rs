//! The pure rule: which files go where.

use std::collections::HashSet;

use crate::grouping::plan::{plan, Dest, Member};

fn member(file: i64, recording: Option<i64>, key: Option<u8>) -> Member {
    Member {
        file,
        recording,
        key: key.map(|k| vec![k; 34]),
        pinned: false,
    }
}

fn no_versions() -> HashSet<(i64, i64)> {
    HashSet::new()
}

#[test]
fn a_track_keeps_the_audio_most_of_its_files_share() {
    // Files 1 and 2 share audio 7; file 3 changed to 9.
    let members = [
        member(1, Some(1), Some(7)),
        member(2, Some(1), Some(7)),
        member(3, Some(1), Some(9)),
    ];
    let p = plan(&members, &no_versions(), &HashSet::new());
    assert_eq!(p.moves.len(), 1);
    assert_eq!((p.moves[0].file, p.moves[0].from), (3, Some(1)));
    assert!(matches!(p.moves[0].to, Dest::New(_)));
}

#[test]
fn a_tie_goes_to_the_hash_of_the_lowest_file_id_so_a_rerun_never_flips_it() {
    let members = [member(5, Some(1), Some(9)), member(4, Some(1), Some(7))];
    let p = plan(&members, &no_versions(), &HashSet::new());
    assert_eq!(p.moves.len(), 1);
    assert_eq!(p.moves[0].file, 5, "file 4's audio stays");
}

#[test]
fn unplaced_files_with_the_same_hash_share_one_new_track_and_others_get_their_own() {
    let members = [
        member(1, None, Some(7)),
        member(2, None, Some(7)),
        member(3, None, None),
        member(4, None, None),
    ];
    let p = plan(&members, &no_versions(), &HashSet::new());
    assert_eq!(p.new_recordings, 3);
    let to = |file: i64| p.moves.iter().find(|m| m.file == file).unwrap().to;
    assert_eq!(to(1), to(2));
    assert_ne!(to(3), to(4));
    assert_ne!(to(1), to(3));
}

#[test]
fn a_plan_for_settled_files_is_empty() {
    let members = [
        member(1, Some(1), Some(7)),
        member(2, Some(1), Some(7)),
        member(3, Some(2), None),
    ];
    assert_eq!(
        plan(&members, &no_versions(), &HashSet::new()),
        Default::default()
    );
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
    let mut members = Vec::new();
    for n in 0..55_000u32 {
        members.push(Member {
            file: i64::from(n),
            recording: None,
            key: Some(key(n % 50_000)),
            pinned: false,
        });
    }
    crate::grouping::plan::VISITS.with(|v| v.set(0));
    let p = plan(&members, &no_versions(), &HashSet::new());
    assert_eq!(p.moves.len(), 55_000);
    assert_eq!(p.new_recordings, 50_000);
    // Counted, not timed, so a busy machine can't fail it: a few looks per
    // file, however many distinct hashes there are.
    let visits = crate::grouping::plan::VISITS.with(|v| v.get());
    assert!(
        visits <= 8 * members.len() as u64,
        "{visits} looks at {} files",
        members.len()
    );
}

#[test]
fn a_merge_prefers_the_track_something_refers_to_then_the_lowest_id() {
    let members = [member(1, Some(1), Some(7)), member(2, Some(2), Some(7))];
    let referenced: HashSet<i64> = [2].into();
    let p = plan(&members, &no_versions(), &referenced);
    assert_eq!(p.moves.len(), 1);
    assert_eq!((p.moves[0].file, p.moves[0].to), (1, Dest::Recording(2)));
    let p = plan(&members, &no_versions(), &HashSet::new());
    assert_eq!((p.moves[0].file, p.moves[0].to), (2, Dest::Recording(1)));
}
