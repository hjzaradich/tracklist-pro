//! The mapping from energy to stars, and the setting. What a send, the merge
//! and a resolve do with it is tested with the conflict engine
//! (`conflicts/tests/energy_stars.rs`).

use super::*;

fn db() -> (tempfile::TempDir, Writer) {
    let dir = tempfile::tempdir().unwrap();
    let writer = Writer::open(&crate::write_guard::test_path(
        dir.path(),
        crate::db::DB_FILE_NAME,
    ))
    .unwrap();
    (dir, writer)
}

fn rating(energy: i64) -> Option<String> {
    rating_for(energy)
}

#[test]
fn energy_1_and_2_are_one_star() {
    assert_eq!(stars_for(1), Some(1));
    assert_eq!(stars_for(2), Some(1));
    assert_eq!(rating(1).as_deref(), Some("51"));
    assert_eq!(rating(2).as_deref(), Some("51"));
}

#[test]
fn energy_3_is_two_stars() {
    assert_eq!(stars_for(3), Some(2));
    assert_eq!(stars_for(4), Some(2));
    assert_eq!(rating(3).as_deref(), Some("102"));
}

#[test]
fn energy_5_and_6_are_three_stars() {
    assert_eq!(stars_for(5), Some(3));
    assert_eq!(stars_for(6), Some(3));
    assert_eq!(rating(5).as_deref(), Some("153"));
}

#[test]
fn energy_7_and_8_are_four_stars() {
    assert_eq!(stars_for(7), Some(4));
    assert_eq!(stars_for(8), Some(4));
    assert_eq!(rating(8).as_deref(), Some("204"));
}

#[test]
fn energy_9_and_10_are_five_stars() {
    assert_eq!(stars_for(9), Some(5));
    assert_eq!(stars_for(10), Some(5));
    assert_eq!(rating(9).as_deref(), Some("255"));
    assert_eq!(rating(10).as_deref(), Some("255"));
}

#[test]
fn no_energy_has_no_stars_and_neither_has_a_number_outside_1_to_10() {
    assert_eq!(stars_for(0), None);
    assert_eq!(stars_for(11), None);
    assert_eq!(stars_for(-1), None);
    assert_eq!(rating_for(0), None);
}

#[test]
fn every_rating_made_from_an_energy_is_one_of_rekordboxs_six_and_reads_back_as_its_stars() {
    for energy in 1..=10 {
        let stars = stars_for(energy).unwrap();
        let rating = rating_for(energy).unwrap();
        assert_eq!(stars_of(&rating), Some(stars), "energy {energy}");
        assert!(crate::conflicts::fits(RATING, &rating), "energy {energy}");
    }
}

#[test]
fn rekordboxs_stars_give_twice_the_energy_and_no_stars_give_none() {
    assert_eq!(energy_for_stars(1), Some(2));
    assert_eq!(energy_for_stars(4), Some(8));
    assert_eq!(energy_for_stars(5), Some(10));
    assert_eq!(energy_for_stars(0), None);
    assert_eq!(energy_for_stars(6), None);
}

#[test]
fn the_setting_is_off_until_it_is_turned_on() {
    let (_dir, writer) = db();
    assert!(!writer.call(|c| is_on(c)).unwrap());

    writer.call(|c| write(c, true)).unwrap();
    assert!(writer.call(|c| is_on(c)).unwrap());

    writer.call(|c| write(c, false)).unwrap();
    assert!(!writer.call(|c| is_on(c)).unwrap());
}

#[test]
fn a_stored_value_that_is_not_a_boolean_reads_as_off() {
    let (_dir, writer) = db();
    for value in ["\"yes\"", "1", "null", "{}"] {
        writer
            .call(move |c| {
                c.execute(
                    "INSERT INTO setting (key, value) VALUES (?1, json(?2))
                     ON CONFLICT (key) DO UPDATE SET value = excluded.value",
                    (ENERGY_AS_STARS, value),
                )
            })
            .unwrap();
        assert!(!writer.call(|c| is_on(c)).unwrap(), "{value}");
    }
}

#[test]
fn the_setting_is_not_in_the_operation_log() {
    let (_dir, writer) = db();
    writer.call(|c| write(c, true)).unwrap();

    let operations: i64 = writer
        .call(|c| c.query_row("SELECT count(*) FROM operation", [], |r| r.get(0)))
        .unwrap();

    assert_eq!(operations, 0);
}
