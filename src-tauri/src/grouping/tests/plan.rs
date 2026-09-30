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
    let p = plan(&members, &no_versions());
    assert_eq!(p.moves.len(), 1);
    assert_eq!((p.moves[0].file, p.moves[0].from), (3, Some(1)));
    assert!(matches!(p.moves[0].to, Dest::New(_)));
}

#[test]
fn a_tie_goes_to_the_hash_of_the_lowest_file_id_so_a_rerun_never_flips_it() {
    let members = [member(5, Some(1), Some(9)), member(4, Some(1), Some(7))];
    let p = plan(&members, &no_versions());
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
    let p = plan(&members, &no_versions());
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
    assert_eq!(plan(&members, &no_versions()), Default::default());
}
