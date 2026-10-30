//! Energy as rekordbox's star rating (1cC-2; ROADMAP §7): what the merge
//! before a send, the conflict engine and a resolve do about it, with the
//! setting on, and that none of it happens with it off. The mapping and the
//! setting itself are tested in `energy_stars/tests.rs`.

use super::*;
use crate::energy::set_energy_on;
use crate::energy_stars;
use crate::ops::{UndoOutcome, UndoRefusal};
use crate::rekordbox_write::{build, gather, gather_merged, Node};

/// What rekordbox shows for 1 to 5 stars.
const ONE: &str = "51";
const THREE: &str = "153";
const FOUR: &str = "204";

impl Lib {
    /// A source other than the user's gives the track this energy
    /// (`mik`, `local` or `ml`).
    fn source_energy(&self, track: LibraryTrackId, source: &'static str, energy: i64) {
        self.writer
            .call(move |c| {
                c.execute(
                    "INSERT INTO analysis (recording_id, source, energy)
                     SELECT recording_id, ?2, ?3 FROM library_track WHERE id = ?1",
                    (track.0, source, energy),
                )
            })
            .unwrap();
    }

    /// The user gives the track this energy, as the keys do (an operation).
    fn user_energy(&self, track: LibraryTrackId, energy: i64) {
        self.writer
            .call(move |c| {
                let recording: i64 = c.query_row(
                    "SELECT recording_id FROM library_track WHERE id = ?1",
                    [track.0],
                    |r| r.get(0),
                )?;
                Ok(set_energy_on(c, &[recording], energy))
            })
            .unwrap()
            .unwrap();
    }

    /// The energy the track shows (the Overview's), if any.
    fn shown(&self, track: LibraryTrackId) -> Option<i64> {
        self.writer
            .call(move |c| {
                c.query_row(
                    "SELECT d.energy FROM library_track lt
                     JOIN recording_display d ON d.recording_id = lt.recording_id
                     WHERE lt.id = ?1",
                    [track.0],
                    |r| r.get(0),
                )
            })
            .unwrap()
    }

    /// Turns the setting on or off, as the command does.
    fn stars_follow_energy(&self, on: bool) {
        self.writer
            .call(move |c| energy_stars::write(c, on))
            .unwrap();
    }

    /// A new read in which the track's entry holds these attributes (the
    /// user rated it in rekordbox).
    fn rated_in_rekordbox(&self, id: LibraryTrackId, attributes: &[(&str, &str)]) {
        let file: i64 = self
            .writer
            .call(move |c| {
                let file = c.query_row(
                    "SELECT linked_file_id FROM library_track WHERE id = ?1",
                    [id.0],
                    |r| r.get(0),
                )?;
                c.execute("DELETE FROM rekordbox_track WHERE file_id = ?1", [file])?;
                Ok(file)
            })
            .unwrap();
        self.entry(file, THIS_READ, false, attributes);
    }

    fn merge(&self, ids: &[LibraryTrackId]) -> Merged {
        let ids = ids.to_vec();
        self.writer
            .call(move |c| merge_before_send(c, &Mount, &ids))
            .unwrap()
    }

    /// The file a send of these tracks writes after the merge, and its
    /// tracks' attributes.
    fn sent_after_merge(&self, ids: &[LibraryTrackId]) -> (Vec<u8>, Vec<Vec<(String, String)>>) {
        let ids = ids.to_vec();
        self.writer
            .call(move |c| {
                let merged = merge_before_send(c, &Mount, &ids)?;
                let input = gather_merged(c, merged, vec![crate_of(&ids)], Vec::new())?;
                let out = build(&input).unwrap();
                assert!(out.left_out.is_empty(), "{:?}", out.left_out);
                let attributes = out.sent.iter().map(|t| t.attributes.clone()).collect();
                Ok((out.xml().to_vec(), attributes))
            })
            .unwrap()
    }

    /// The file a send of these tracks writes with no merge at all, as it
    /// was before energy could be sent.
    fn sent_without_merge(&self, ids: &[LibraryTrackId]) -> Vec<u8> {
        let ids = ids.to_vec();
        self.writer
            .call(move |c| {
                let input = gather(c, &Mount, &ids, vec![crate_of(&ids)], Vec::new())?;
                Ok(build(&input).unwrap().xml().to_vec())
            })
            .unwrap()
    }

    fn would_change_answer(&self) -> energy_stars::WouldChange {
        self.writer
            .call(|c| energy_stars::would_change(c, &Mount))
            .unwrap()
    }

    fn would_change(&self) -> u32 {
        match self.would_change_answer() {
            energy_stars::WouldChange::Counted { tracks, .. } => tracks,
            other => panic!("expected a count, got {other:?}"),
        }
    }

    fn stars_kept(&self, track: LibraryTrackId) -> bool {
        self.writer
            .call(move |c| {
                c.query_row(
                    "SELECT stars_held FROM library_track WHERE id = ?1",
                    [track.0],
                    |r| r.get(0),
                )
            })
            .unwrap()
    }

    fn only_open_or_none(&self) -> Option<Conflict> {
        self.open(&Filter::default()).into_iter().next()
    }

    fn app_ratings(&self) -> i64 {
        self.writer
            .call(|c| {
                c.query_row(
                    "SELECT count(*) FROM app_value WHERE field = 'Rating'",
                    [],
                    |r| r.get(0),
                )
            })
            .unwrap()
    }
}

fn attribute<'a>(track: &'a [(String, String)], name: &str) -> Option<&'a str> {
    track
        .iter()
        .find(|(n, _)| n == name)
        .map(|(_, v)| v.as_str())
}

/// A track rekordbox has at `rating` stars, sent at that rating, whose
/// energy the user gave.
fn rated(lib: &Lib, rating: &str, energy: i64) -> LibraryTrackId {
    let id = lib.known(&[("Name", "A track"), ("Rating", rating)]);
    lib.sent(id, &[("Rating", rating)]);
    lib.user_energy(id, energy);
    id
}

// ---- Off: nothing changes ---------------------------------------------------

#[test]
fn a_send_with_the_setting_off_is_byte_for_byte_the_send_without_energy() {
    // The same Library twice: once with energies, once without. Off is the
    // default, so the merge must not see them.
    let with = Lib::new();
    let a = rated(&with, ONE, 9);
    let b = with.known(&[("Name", "Other"), ("Rating", FOUR)]);
    with.source_energy(b, "mik", 2);
    let without = Lib::new();
    let c = without.known(&[("Name", "A track"), ("Rating", ONE)]);
    without.sent(c, &[("Rating", ONE)]);
    let d = without.known(&[("Name", "Other"), ("Rating", FOUR)]);

    let (merged_with, attributes) = with.sent_after_merge(&[a, b]);
    let (merged_without, _) = without.sent_after_merge(&[c, d]);

    assert_eq!(merged_with, merged_without);
    assert_eq!(merged_with, with.sent_without_merge(&[a, b]));
    assert_eq!(attribute(&attributes[0], "Rating"), Some(ONE));
    assert_eq!(attribute(&attributes[1], "Rating"), Some(FOUR));
}

#[test]
fn with_the_setting_off_a_star_change_in_rekordbox_is_taken_in_and_opens_no_conflict() {
    let lib = Lib::new();
    let id = rated(&lib, FOUR, 8);
    lib.rated_in_rekordbox(id, &[("Name", "A track"), ("Rating", ONE)]);

    let merged = lib.merge(&[id]);

    assert_eq!(merged.open, std::collections::HashMap::new());
    assert_eq!(merged.chosen, std::collections::HashMap::new());
    assert_eq!(lib.rows(), []);
}

#[test]
fn with_the_setting_off_nothing_is_derived_and_nothing_is_written() {
    let lib = Lib::new();
    let id = rated(&lib, ONE, 9);

    let (derived, written) = lib.written_by(move |c| {
        let sent = send_values(c, &Mount, &[id])?;
        let mut app = AppValues::default();
        energy_stars::derive(c, &sent, &mut app)?;
        Ok((app, energy_stars::derived_tracks(c, &[id])?))
    });

    assert_eq!(derived.0, AppValues::default());
    assert!(derived.1.is_empty());
    assert_eq!(written, Vec::<String>::new());
}

// ---- On: the send ---------------------------------------------------------

#[test]
fn a_send_with_the_setting_on_writes_the_derived_rating_and_keeps_the_stars_of_a_track_with_no_energy(
) {
    let lib = Lib::new();
    lib.stars_follow_energy(true);
    let with_energy = rated(&lib, ONE, 8);
    let without = lib.known(&[("Name", "No energy"), ("Rating", THREE)]);
    lib.sent(without, &[("Rating", THREE)]);

    let (_, attributes) = lib.sent_after_merge(&[with_energy, without]);

    assert_eq!(attribute(&attributes[0], "Rating"), Some(FOUR));
    assert_eq!(attribute(&attributes[1], "Rating"), Some(THREE));
}

#[test]
fn a_track_rekordbox_does_not_know_gets_its_rating_from_its_energy() {
    let lib = Lib::new();
    lib.stars_follow_energy(true);
    let (new, _) = lib.linked();
    lib.source_energy(new, "local", 3);
    let (no_energy, _) = lib.linked();

    let (_, attributes) = lib.sent_after_merge(&[new, no_energy]);

    assert_eq!(attribute(&attributes[0], "Rating"), Some("102"));
    assert_eq!(
        attribute(&attributes[1], "Rating"),
        None,
        "no energy, no rating"
    );
}

#[test]
fn the_rating_is_made_from_the_energy_the_overview_shows() {
    let lib = Lib::new();
    lib.stars_follow_energy(true);
    // The user's word wins over Mixed In Key's, and Mixed In Key's over the
    // estimate's.
    let user = lib.known(&[("Rating", ONE)]);
    lib.sent(user, &[("Rating", ONE)]);
    lib.source_energy(user, "mik", 2);
    lib.user_energy(user, 10);
    let mik = lib.known(&[("Rating", ONE)]);
    lib.sent(mik, &[("Rating", ONE)]);
    lib.source_energy(mik, "mik", 6);
    lib.source_energy(mik, "local", 10);

    let (_, attributes) = lib.sent_after_merge(&[user, mik]);

    assert_eq!(attribute(&attributes[0], "Rating"), Some("255"));
    assert_eq!(attribute(&attributes[1], "Rating"), Some(THREE));
}

#[test]
fn the_first_send_after_turning_it_on_writes_the_stars_of_a_track_never_sent() {
    // The user confirmed the change when turning the setting on, so a track
    // with no base is not held as a conflict.
    let lib = Lib::new();
    lib.stars_follow_energy(true);
    let id = lib.known(&[("Rating", FOUR)]);
    lib.user_energy(id, 2);

    let merged = lib.merge(&[id]);

    assert_eq!(merged.open, std::collections::HashMap::new());
    assert_eq!(merged.chosen[&id], [("Rating".to_owned(), ONE.to_owned())]);
    assert_eq!(lib.rows(), []);
}

#[test]
fn a_change_of_energy_moves_the_stars_with_no_write_of_its_own() {
    let lib = Lib::new();
    lib.stars_follow_energy(true);
    let id = rated(&lib, FOUR, 8);
    assert_eq!(lib.merge(&[id]).chosen, std::collections::HashMap::new());
    let before = lib.operations().len();

    lib.user_energy(id, 2);

    assert_eq!(
        lib.operations().len(),
        before + 1,
        "only the energy is logged"
    );
    assert_eq!(
        lib.merge(&[id]).chosen[&id],
        [("Rating".to_owned(), ONE.to_owned())]
    );
    assert_eq!(lib.app_ratings(), 0, "the rating is derived, never stored");
}

// ---- The count --------------------------------------------------------------

#[test]
fn the_count_is_the_tracks_with_an_energy_whose_stars_differ_from_rekordboxs() {
    let lib = Lib::new();
    // Same stars: energy 8 is 4 stars.
    let same = lib.known(&[("Rating", FOUR)]);
    lib.user_energy(same, 7);
    // Different stars.
    let differs = lib.known(&[("Rating", ONE)]);
    lib.user_energy(differs, 8);
    // rekordbox has no rating for it: 0 stars against 1.
    let unrated = lib.known(&[("Name", "Unrated")]);
    lib.source_energy(unrated, "mik", 2);
    // No energy: keeps its stars, so it doesn't count.
    let _no_energy = lib.known(&[("Rating", ONE)]);
    // rekordbox doesn't have this one: it has no stars to change.
    let (new, _) = lib.linked();
    lib.user_energy(new, 6);

    assert_eq!(lib.would_change(), 2);
    // It's the count to ask before turning the setting on, and it asks for
    // nothing to be on.
    assert_eq!(lib.would_change(), 2);
    lib.stars_follow_energy(true);
    assert_eq!(lib.would_change(), 2, "the same with it on");
    assert_eq!(lib.rows(), []);
}

#[test]
fn the_count_says_when_rekordbox_was_read_and_says_so_when_it_never_was() {
    let lib = Lib::new();
    let (track, _) = lib.linked();
    lib.user_energy(track, 9);
    // No read yet: not "0 tracks".
    assert_eq!(lib.would_change_answer(), energy_stars::WouldChange::NoRead);

    let known = lib.known(&[("Rating", ONE)]);
    lib.user_energy(known, 9);

    assert_eq!(
        lib.would_change_answer(),
        energy_stars::WouldChange::Counted {
            tracks: 1,
            read_at: THIS_READ.to_owned()
        }
    );
}

#[test]
fn the_count_leaves_out_a_row_kept_from_an_older_read() {
    let lib = Lib::new();
    let (stale, file) = lib.linked();
    lib.entry(file, EARLIER_READ, false, &[("Rating", ONE)]);
    lib.user_energy(stale, 9);
    // The newest read is a later one that has other tracks.
    let current = lib.known(&[("Rating", ONE)]);
    lib.user_energy(current, 9);

    assert_eq!(lib.would_change(), 1);
}

// ---- A star change in rekordbox ---------------------------------------------

#[test]
fn a_star_change_in_rekordbox_is_a_conflict_only_while_the_setting_is_on() {
    let lib = Lib::new();
    let id = rated(&lib, FOUR, 8);
    lib.rated_in_rekordbox(id, &[("Name", "A track"), ("Rating", ONE)]);

    // On: the energy hasn't moved, rekordbox's stars did.
    lib.stars_follow_energy(true);
    let merged = lib.merge(&[id]);

    assert_eq!(merged.open[&id], ["Rating"]);
    let open = lib.only_open();
    assert_eq!(open.field, "Rating");
    assert_eq!(values(&open), (Some(FOUR), Some(ONE), None));
    // rekordbox's stars go out meanwhile, as for any field in dispute.
    assert_eq!(merged.chosen, std::collections::HashMap::new());
}

#[test]
fn a_track_with_no_energy_follows_rekordboxs_stars_with_the_setting_on() {
    let lib = Lib::new();
    lib.stars_follow_energy(true);
    let id = lib.known(&[("Rating", FOUR)]);
    lib.sent(id, &[("Rating", FOUR)]);
    lib.rated_in_rekordbox(id, &[("Rating", ONE)]);

    let merged = lib.merge(&[id]);

    assert_eq!(merged.open, std::collections::HashMap::new());
    assert_eq!(lib.rows(), []);
}

#[test]
fn a_send_that_rekordbox_did_not_take_up_sends_the_stars_again_instead_of_asking() {
    let lib = Lib::new();
    lib.stars_follow_energy(true);
    // Sent 4 stars over rekordbox's 1, but never imported: rekordbox still
    // shows the 1 star the send replaced.
    let id = lib.known(&[("Rating", ONE)]);
    lib.sent(id, &[("Rating", FOUR)]);
    lib.user_energy(id, 8);
    lib.run(&format!(
        "UPDATE sync_base SET replaced = '{ONE}' WHERE field = 'Rating'"
    ));

    let merged = lib.merge(&[id]);

    assert_eq!(merged.open, std::collections::HashMap::new());
    assert_eq!(merged.chosen[&id], [("Rating".to_owned(), FOUR.to_owned())]);
}

#[test]
fn turning_it_off_withdraws_the_open_conflict_and_the_rating_follows_rekordbox_again() {
    let lib = Lib::new();
    lib.stars_follow_energy(true);
    let id = rated(&lib, FOUR, 8);
    lib.rated_in_rekordbox(id, &[("Name", "A track"), ("Rating", ONE)]);
    lib.merge(&[id]);
    assert_eq!(lib.only_open().field, "Rating");

    lib.stars_follow_energy(false);
    let merged = lib.merge(&[id]);

    assert_eq!(lib.rows(), [], "the sides no longer disagree");
    assert_eq!(merged.chosen, std::collections::HashMap::new());
    let (_, attributes) = lib.sent_after_merge(&[id]);
    assert_eq!(attribute(&attributes[0], "Rating"), Some(ONE));
}

// ---- Keeping one side -------------------------------------------------------

/// A track with 4 stars sent from energy 8, rated 3 stars in rekordbox
/// since, with the conflict open.
fn in_dispute(lib: &Lib) -> (LibraryTrackId, Conflict) {
    lib.stars_follow_energy(true);
    let id = rated(lib, FOUR, 8);
    lib.rated_in_rekordbox(id, &[("Name", "A track"), ("Rating", THREE)]);
    lib.merge(&[id]);
    (id, lib.only_open())
}

#[test]
fn keeping_the_apps_stars_sends_the_energys_stars_again_and_changes_no_energy() {
    let lib = Lib::new();
    let (id, open) = in_dispute(&lib);

    lib.resolve(open.id, Choice::KeepApp).unwrap();

    assert_eq!(lib.shown(id), Some(8));
    assert_eq!(lib.app_ratings(), 0);
    let merged = lib.merge(&[id]);
    assert_eq!(merged.chosen[&id], [("Rating".to_owned(), FOUR.to_owned())]);
    assert_eq!(merged.open, std::collections::HashMap::new());
    let (_, attributes) = lib.sent_after_merge(&[id]);
    assert_eq!(attribute(&attributes[0], "Rating"), Some(FOUR));
}

#[test]
fn keeping_rekordboxs_stars_sets_the_energy_to_twice_the_stars() {
    let lib = Lib::new();
    let (id, open) = in_dispute(&lib);

    lib.resolve(open.id, Choice::KeepRekordbox).unwrap();

    assert_eq!(lib.shown(id), Some(6), "3 stars, energy 6");
    assert_eq!(lib.app_ratings(), 0);
    let merged = lib.merge(&[id]);
    assert_eq!(merged.open, std::collections::HashMap::new());
    let (_, attributes) = lib.sent_after_merge(&[id]);
    assert_eq!(attribute(&attributes[0], "Rating"), Some(THREE));
}

#[test]
fn keeping_rekordboxs_stars_sets_the_users_energy_over_another_sources() {
    let lib = Lib::new();
    lib.stars_follow_energy(true);
    // The energy comes from Mixed In Key; the user has none.
    let id = lib.known(&[("Rating", FOUR)]);
    lib.sent(id, &[("Rating", FOUR)]);
    lib.source_energy(id, "mik", 8);
    lib.rated_in_rekordbox(id, &[("Rating", ONE)]);
    lib.merge(&[id]);

    lib.resolve(lib.only_open().id, Choice::KeepRekordbox)
        .unwrap();

    assert_eq!(lib.shown(id), Some(2), "1 star, energy 2: the user's word");
}

/// A track with Mixed In Key's energy 5 and the user's own 8, 4 stars sent,
/// rated 0 stars in rekordbox since, the conflict settled by keeping
/// rekordbox's.
fn kept_at_zero_stars(lib: &Lib) -> LibraryTrackId {
    lib.stars_follow_energy(true);
    let id = lib.known(&[("Name", "A track"), ("Rating", FOUR)]);
    lib.sent(id, &[("Rating", FOUR)]);
    lib.source_energy(id, "mik", 5);
    lib.user_energy(id, 8);
    lib.rated_in_rekordbox(id, &[("Name", "A track"), ("Rating", "0")]);
    lib.merge(&[id]);
    let open = lib.only_open();
    assert_eq!(values(&open), (Some(FOUR), Some("0"), None));
    lib.resolve(open.id, Choice::KeepRekordbox).unwrap();
    id
}

#[test]
fn keeping_rekordboxs_zero_stars_clears_the_users_energy_and_the_track_falls_back() {
    let lib = Lib::new();
    let id = kept_at_zero_stars(&lib);

    assert_eq!(lib.shown(id), Some(5), "Mixed In Key's energy shows again");
    assert!(lib.stars_kept(id), "and the choice is remembered");
}

#[test]
fn after_keeping_rekordboxs_zero_stars_a_fallback_energy_raises_no_conflict_and_the_stars_stay() {
    let lib = Lib::new();
    let id = kept_at_zero_stars(&lib);

    // Mixed In Key says 5, which would be 3 stars; the choice stands.
    let merged = lib.merge(&[id]);
    assert_eq!(merged.open, std::collections::HashMap::new());
    assert_eq!(lib.only_open_or_none(), None);
    let (_, attributes) = lib.sent_after_merge(&[id]);
    assert_eq!(attribute(&attributes[0], "Rating"), Some("0"));
    // A send, and the next one: still nothing.
    lib.sent(id, &[("Rating", "0")]);
    assert_eq!(lib.merge(&[id]).chosen, std::collections::HashMap::new());
    assert_eq!(
        lib.rows()
            .iter()
            .filter(|c| c.status == Status::Open)
            .count(),
        0
    );
}

#[test]
fn a_later_star_change_in_rekordbox_on_a_marked_track_is_just_rekordboxs_value() {
    let lib = Lib::new();
    let id = kept_at_zero_stars(&lib);
    lib.sent(id, &[("Rating", "0")]);
    lib.rated_in_rekordbox(id, &[("Name", "A track"), ("Rating", ONE)]);

    let merged = lib.merge(&[id]);

    assert_eq!(merged.open, std::collections::HashMap::new());
    assert_eq!(merged.chosen, std::collections::HashMap::new());
    let (_, attributes) = lib.sent_after_merge(&[id]);
    assert_eq!(attribute(&attributes[0], "Rating"), Some(ONE));
}

#[test]
fn the_turn_on_count_and_the_preflights_count_skip_a_marked_track() {
    let lib = Lib::new();
    let id = kept_at_zero_stars(&lib);
    // Another track whose stars do change.
    let other = lib.known(&[("Rating", ONE)]);
    lib.user_energy(other, 8);

    assert_eq!(lib.would_change(), 1, "only the other track");
    let merged = lib.merge(&[id, other]);
    let counted = lib
        .writer
        .call(move |c| energy_stars::stars_changed_by(c, &merged))
        .unwrap();
    assert_eq!(counted, Some(1));
}

#[test]
fn setting_an_energy_on_a_marked_track_ends_the_marker_and_the_stars_follow_the_energy_again() {
    let lib = Lib::new();
    let id = kept_at_zero_stars(&lib);
    lib.sent(id, &[("Rating", "0")]);

    lib.user_energy(id, 8);

    assert!(!lib.stars_kept(id));
    let merged = lib.merge(&[id]);
    assert_eq!(merged.chosen[&id], [("Rating".to_owned(), FOUR.to_owned())]);
    let (_, attributes) = lib.sent_after_merge(&[id]);
    assert_eq!(attribute(&attributes[0], "Rating"), Some(FOUR));
}

#[test]
fn undoing_the_energy_that_ended_the_marker_puts_the_marker_back() {
    let lib = Lib::new();
    let id = kept_at_zero_stars(&lib);
    lib.user_energy(id, 8);
    assert!(!lib.stars_kept(id));

    assert_eq!(lib.undone(), crate::energy::SET_ENERGY_OPERATION);

    assert!(lib.stars_kept(id));
    assert_eq!(lib.shown(id), Some(5));
}

#[test]
fn keeping_rekordboxs_zero_stars_leaves_a_track_with_no_other_energy_with_none() {
    let lib = Lib::new();
    lib.stars_follow_energy(true);
    let id = rated(&lib, FOUR, 8);
    // An entry with no rating at all is 0 stars too.
    lib.rated_in_rekordbox(id, &[("Name", "A track")]);
    lib.merge(&[id]);

    lib.resolve(lib.only_open().id, Choice::KeepRekordbox)
        .unwrap();

    assert_eq!(lib.shown(id), None);
    let merged = lib.merge(&[id]);
    assert_eq!(merged.open, std::collections::HashMap::new());
    assert_eq!(merged.chosen, std::collections::HashMap::new());
    assert_eq!(lib.would_change(), 0, "nothing of its own to send");
}

#[test]
fn a_rating_given_by_hand_in_a_star_conflict_sets_the_energy_as_rekordboxs_does() {
    let lib = Lib::new();
    let (id, open) = in_dispute(&lib);

    let settled = lib.resolve(open.id, Choice::Edit(ONE.to_owned())).unwrap();

    assert_eq!(settled.value.status, Status::Edited);
    assert_eq!(lib.shown(id), Some(2));
    assert_eq!(lib.app_ratings(), 0);
}

#[test]
fn keeping_rekordboxs_stars_for_all_ratings_sets_each_energy_in_one_operation() {
    let lib = Lib::new();
    lib.stars_follow_energy(true);
    let a = rated(&lib, FOUR, 8);
    let b = rated(&lib, FOUR, 7);
    lib.rated_in_rekordbox(a, &[("Rating", ONE)]);
    lib.rated_in_rekordbox(b, &[("Rating", "0")]);
    lib.merge(&[a, b]);
    assert_eq!(lib.open(&Filter::default()).len(), 2);
    let operations = lib.operations().len();

    let settled = lib
        .resolve_all(
            &Filter {
                field: Some("Rating".to_owned()),
                tracks: None,
            },
            Choice::KeepRekordbox,
        )
        .unwrap();

    assert_eq!(settled.value.len(), 2);
    assert_eq!(lib.operations().len(), operations + 1);
    assert_eq!(lib.shown(a), Some(2));
    assert_eq!(lib.shown(b), None);
    assert_eq!(lib.undone(), RESOLVE_ALL_OPERATION);
    assert_eq!(lib.shown(a), Some(8));
    assert_eq!(lib.shown(b), Some(7));
}

#[test]
fn undoing_keep_rekordboxs_puts_the_energy_and_the_conflict_back() {
    let lib = Lib::new();
    let (id, open) = in_dispute(&lib);
    lib.resolve(open.id, Choice::KeepRekordbox).unwrap();
    assert_eq!(lib.shown(id), Some(6));

    assert_eq!(lib.undone(), RESOLVE_OPERATION);

    assert_eq!(
        lib.shown(id),
        Some(8),
        "the user's energy is back as it was"
    );
    let reopened = lib.only_open();
    assert_eq!(reopened.id, open.id);
    assert_eq!(values(&reopened), values(&open));
}

#[test]
fn undoing_keep_rekordboxs_at_zero_stars_brings_the_cleared_energy_back() {
    let lib = Lib::new();
    lib.stars_follow_energy(true);
    let id = rated(&lib, FOUR, 8);
    lib.rated_in_rekordbox(id, &[("Name", "A track"), ("Rating", "0")]);
    lib.merge(&[id]);
    lib.resolve(lib.only_open().id, Choice::KeepRekordbox)
        .unwrap();
    assert_eq!(lib.shown(id), None);

    assert_eq!(lib.undone(), RESOLVE_OPERATION);

    assert_eq!(lib.shown(id), Some(8));
    assert_eq!(lib.only_open().field, "Rating");
}

#[test]
fn one_undo_of_keeping_rekordboxs_zero_stars_restores_the_energy_the_marker_and_the_conflict() {
    let lib = Lib::new();
    let id = kept_at_zero_stars(&lib);
    assert!(lib.stars_kept(id));
    assert_eq!(lib.merge(&[id]).open, std::collections::HashMap::new());

    assert_eq!(lib.undone(), RESOLVE_OPERATION);

    assert_eq!(lib.shown(id), Some(8), "the user's energy is back");
    assert!(!lib.stars_kept(id), "and the marker is gone");
    assert_eq!(lib.only_open().field, "Rating", "and the conflict is open");
    assert_eq!(lib.merge(&[id]).open[&id], ["Rating"]);
}

#[test]
fn undoing_keep_rekordboxs_is_refused_once_a_send_carried_the_stars() {
    let lib = Lib::new();
    let (id, open) = in_dispute(&lib);
    lib.resolve(open.id, Choice::KeepRekordbox).unwrap();
    // A send since: the base moves to the stars sent.
    lib.run("UPDATE sync_base SET synced_at = '2099-01-01T00:00:00.000Z' WHERE field = 'Rating'");

    match lib.undo() {
        UndoOutcome::Refused { reason, .. } => assert_eq!(reason, UndoRefusal::SentSince),
        other => panic!("expected the undo to be refused, got {other:?}"),
    }
    assert_eq!(lib.shown(id), Some(6), "nothing was undone");
}

// ---- Off: a resolve is as it was --------------------------------------------

#[test]
fn with_the_setting_off_settling_a_rating_conflict_touches_no_energy() {
    let lib = Lib::new();
    let id = lib.known(&[("Rating", ONE)]);
    lib.user_energy(id, 8);
    let open = {
        lib.detect(&[track(id, vec![apart("Rating", FOUR, THREE, "0")])]);
        lib.only_open()
    };

    lib.resolve(open.id, Choice::KeepRekordbox).unwrap();

    assert_eq!(lib.shown(id), Some(8));
}

/// A send writes the tracks that are in a crate (1.9 rule 4).
fn crate_of(ids: &[LibraryTrackId]) -> Node {
    Node::Playlist {
        name: "Crate".to_owned(),
        entries: ids.to_vec(),
    }
}

// ---- Turning it off: the rating follows rekordbox ---------------------------

#[test]
fn after_keeping_the_apps_stars_and_turning_it_off_the_next_send_writes_rekordboxs_value() {
    let lib = Lib::new();
    let (id, open) = in_dispute(&lib);
    lib.resolve(open.id, Choice::KeepApp).unwrap();
    lib.stars_follow_energy(false);

    let merged = lib.merge(&[id]);

    assert_eq!(merged.chosen, std::collections::HashMap::new());
    assert_eq!(merged.open, std::collections::HashMap::new());
    let (_, attributes) = lib.sent_after_merge(&[id]);
    assert_eq!(attribute(&attributes[0], "Rating"), Some(THREE));
    assert_eq!(lib.app_ratings(), 0);
}

#[test]
fn an_own_rating_from_before_is_dropped_for_a_track_with_energy_when_the_setting_changes() {
    let lib = Lib::new();
    // An old own Rating on a track that has an energy, and one on a track
    // that has none.
    let with = lib.known(&[("Rating", ONE)]);
    lib.set(with, "Rating", "255").unwrap();
    lib.user_energy(with, 4);
    let without = lib.known(&[("Rating", ONE)]);
    lib.set(without, "Rating", THREE).unwrap();

    lib.writer.call(|c| energy_stars::set_on(c, true)).unwrap();
    assert_eq!(
        lib.app_ratings(),
        1,
        "the one for a track with no energy stays"
    );
    lib.writer.call(|c| energy_stars::set_on(c, false)).unwrap();

    // The rating follows rekordbox again for the track that had an energy.
    let merged = lib.merge(&[with]);
    assert_eq!(merged.chosen, std::collections::HashMap::new());
    let (_, attributes) = lib.sent_after_merge(&[with]);
    assert_eq!(attribute(&attributes[0], "Rating"), Some(ONE));
}
