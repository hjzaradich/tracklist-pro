//! "In a crate" and "In no crate" stay right when crate folders (1cA-9/10)
//! delete crates with their entries, move crates, and when Undo brings them
//! back: the track table's `IN_CRATE` follows the entries through the
//! triggers of migration 0032, whichever way the entries come and go.

use super::*;
use crate::crates::{create_folder_on, create_in, delete_on, move_on};

fn in_crate(f: &Fixture) -> Vec<i64> {
    filter_ids(f, TableSource::Library, vec![TableFilter::InCrate])
}

fn in_no_crate(f: &Fixture) -> Vec<i64> {
    filter_ids(f, TableSource::Library, vec![TableFilter::InNoCrate])
}

fn crate_id(
    f: &Fixture,
    make: impl FnOnce(&mut rusqlite::Connection) -> i64 + Send + 'static,
) -> CrateId {
    CrateId(f.lib.writer.call(move |c| Ok(make(c))).unwrap())
}

fn put(f: &Fixture, crate_id: CrateId, recording: i64) {
    f.lib.insert(
        "INSERT INTO crate_entry (crate_id, library_track_id, kind)
         SELECT ?1, id, 'member' FROM library_track WHERE recording_id = ?2",
        (crate_id.0, recording),
    );
}

#[test]
fn deleting_a_folder_with_its_crates_and_undoing_it_keeps_in_a_crate_right() {
    let f = fixture();
    let [r1, r2, _, r4, ..] = f.recs;
    let folder = crate_id(&f, |c| {
        create_folder_on(c, None, "Gigs").unwrap().unwrap().0
    });
    let inside = crate_id(&f, move |c| {
        create_in(c, Some(CrateId(folder.0)), "Friday")
            .unwrap()
            .unwrap()
            .0
    });
    let outside = crate_id(&f, |c| create_in(c, None, "Warm up").unwrap().unwrap().0);
    // r1 is only in the crate in the folder, r2 in both, r4 only outside.
    put(&f, inside, r1);
    put(&f, inside, r2);
    put(&f, outside, r2);
    put(&f, outside, r4);
    assert_eq!(in_crate(&f), [r1, r2, r4]);
    assert!(in_no_crate(&f).is_empty());

    // Moving the crate out of the folder changes no track's crates.
    f.lib
        .writer
        .call(move |c| Ok(move_on(c, inside, None)))
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(in_crate(&f), [r1, r2, r4]);
    f.lib
        .writer
        .call(move |c| Ok(move_on(c, inside, Some(folder))))
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(in_crate(&f), [r1, r2, r4]);

    // The folder goes with the crate in it and its entries: r1 is in no crate.
    f.lib
        .writer
        .call(move |c| Ok(delete_on(c, folder)))
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(in_crate(&f), [r2, r4]);
    assert_eq!(in_no_crate(&f), [r1]);

    // Undo brings the crate, the folder and the entries back.
    f.lib
        .writer
        .call(|c| Ok(crate::ops::undo_last(c)))
        .unwrap()
        .unwrap();
    assert_eq!(in_crate(&f), [r1, r2, r4]);
    assert!(in_no_crate(&f).is_empty());
}
