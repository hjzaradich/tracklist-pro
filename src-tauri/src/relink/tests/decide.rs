//! Confirming a probable match, "Not the same", and withdrawing a
//! confirmation (1bE-6; [`crate::relink::decide`]), through relink's own
//! rules, the Review queue, the add to the Library and the operation log.
//! Everything is made up: no file is read.

use super::follow::Drives;
use super::*;
use crate::library::{promote_with, Confirmed, LibraryError};
use crate::ops::UndoOutcome;
use crate::relink::decide::{
    confirm_match, reject_match, store, withdraw, Proposal, Refused, Verdict, CONFIRM_OPERATION,
    REJECT_OPERATION, WITHDRAW_OPERATION,
};
use crate::review::actions::RelinkOffer;
use crate::review::{ReviewGroup, ReviewItemId, ReviewKind};
use crate::send_values::SentAs;

const OLD: &str = "E:/Gone/Omicron.mp3";

impl Lib {
    fn confirm_item(&self, row: i64, file: i64) -> Result<Option<i64>, Refused> {
        self.writer
            .call(move |c| Ok(confirm_match(c, row, file)))
            .unwrap()
    }

    fn reject_item(&self, row: i64, file: i64) -> Result<Option<i64>, Refused> {
        self.writer
            .call(move |c| Ok(reject_match(c, row, file)))
            .unwrap()
    }

    fn withdraw_item(&self, row: i64) -> Result<Option<i64>, Refused> {
        self.writer.call(move |c| Ok(withdraw(c, row))).unwrap()
    }

    /// The user said `location`'s entry is not `file`, stored the way the
    /// app stores it, without an item to say it on.
    fn said_not_the_same(&self, location: &str, file: i64, method: Method) {
        let key = location::decode(location).unwrap().match_key();
        let about = Proposal {
            file,
            method: method.as_str().to_owned(),
            confidence: Some(0.5),
        };
        self.writer
            .call(move |c| {
                Ok(crate::ops::record(
                    c,
                    REJECT_OPERATION,
                    &serde_json::json!({}),
                    |rec| store(rec, &key, Verdict::NotSame, &about),
                ))
            })
            .unwrap()
            .unwrap();
    }

    /// Undo, as the Undo button runs it; returns the kind it took back.
    fn undo_decision(&self) -> String {
        let outcome = self
            .writer
            .call(|c| Ok(crate::ops::undo_for_user(c, None)))
            .unwrap()
            .unwrap();
        match outcome {
            UndoOutcome::Undone { operation } => operation.kind,
            other => panic!("not undone: {other:?}"),
        }
    }

    /// The entries of the "Confirm relink" items in the queue.
    fn relink_items(&self) -> Vec<i64> {
        self.writer
            .call(|c| crate::review::items(c, &Drives, ReviewGroup::NeedsCloserLook, 0, 100))
            .unwrap()
            .items
            .into_iter()
            .filter(|i| i.kind == ReviewKind::Relink)
            .map(|i| i.id.a)
            .collect()
    }

    /// What the item of `row` offers to decide, and the reason it shows.
    fn asked(&self, row: i64) -> Option<(RelinkOffer, String)> {
        let id = ReviewItemId {
            kind: ReviewKind::Relink,
            a: row,
            b: None,
        };
        let comparison = self
            .writer
            .call(move |c| crate::review::comparison(c, &Drives, id))
            .unwrap()?;
        let reason = comparison.suggestion?.reasons()[0].key().to_owned();
        Some((comparison.relink?, reason))
    }

    /// The `relink` table: (verdict, file, the audio byte recorded).
    fn said(&self) -> Vec<(String, i64, Option<u8>)> {
        self.writer
            .call(|c| {
                c.prepare("SELECT verdict, file_id, audio_hash FROM relink ORDER BY id")?
                    .query_map([], |r| {
                        Ok((
                            r.get(0)?,
                            r.get(1)?,
                            r.get::<_, Option<Vec<u8>>>(2)?
                                .and_then(|h| h.first().copied()),
                        ))
                    })?
                    .collect()
            })
            .unwrap()
    }

    fn operation_kinds(&self) -> Vec<String> {
        self.writer
            .call(|c| {
                c.prepare("SELECT kind FROM operation WHERE undone_at IS NULL ORDER BY id")?
                    .query_map([], |r| r.get(0))?
                    .collect()
            })
            .unwrap()
    }
}

/// A rekordbox entry whose file is gone from `E:\Gone`, and the only file
/// with its name, elsewhere: a filename-only match, probable. Returns the
/// library, what's mounted, the file and the entry's row, relinked.
fn a_name_guess() -> (Lib, Mounted, i64, i64) {
    let (lib, music, mounted) = e_music();
    let guess = lib.file(music, "Elsewhere/Omicron.mp3", None);
    let row = lib.track_with(&loc(OLD), Some("200"), &[("Name", "Omicron")]);
    lib.relink(&mounted);
    assert_eq!(lib.matched(row), name_only(guess));
    assert!(lib.probable(row));
    assert_eq!(lib.relink_items(), [row]);
    (lib, mounted, guess, row)
}

fn confirmed(file: i64) -> Option<Match> {
    Some((file, "filename_only".to_owned(), 1.0))
}

// ---- confirm ---------------------------------------------------------------------

#[test]
fn confirming_a_probable_match_trusts_it_at_once_and_every_later_run_keeps_it() {
    let (lib, mounted, guess, row) = a_name_guess();
    assert_eq!(
        lib.asked(row).map(|(offer, _)| offer),
        Some(RelinkOffer {
            file_id: guess,
            confirmed_before: false,
        })
    );

    assert!(lib.confirm_item(row, guess).unwrap().is_some());
    // Trusted as the action returned, and out of the queue: no job ran.
    assert_eq!(lib.matched(row), confirmed(guess));
    assert!(!lib.probable(row));
    assert_eq!(lib.relink_items(), Vec::<i64>::new());
    assert_eq!(lib.asked(row), None);
    assert_eq!(lib.operation_kinds(), [CONFIRM_OPERATION]);
    assert_eq!(lib.said(), [("same".to_owned(), guess, None)]);

    // The relink run that follows decides the same, and so does a fresh
    // read of rekordbox.
    let before = lib.all_states();
    lib.relink(&mounted);
    assert_eq!(lib.all_states(), before);
    lib.fresh_read();
    lib.relink(&mounted);
    assert_eq!(lib.all_states(), before);
    assert_eq!(lib.relink_items(), Vec::<i64>::new());
}

#[test]
fn a_confirmation_applies_to_every_entry_at_that_location() {
    // rekordbox holds the Location twice, in two spellings.
    let (lib, music, mounted) = e_music();
    let guess = lib.file(music, "Elsewhere/Omicron.mp3", None);
    let row = lib.track(&loc(OLD), Some("200"));
    lib.relink(&mounted);
    assert_eq!(lib.matched(row), name_only(guess));
    let twin = lib.track("file://localhost/e:/gone/OMICRON.mp3", Some("200"));

    lib.confirm_item(row, guess).unwrap();
    assert_eq!(lib.matched(row), confirmed(guess));
    assert_eq!(lib.matched(twin), confirmed(guess));
    lib.relink(&mounted);
    assert_eq!(lib.matched(row), confirmed(guess));
    assert_eq!(lib.matched(twin), confirmed(guess));
}

#[test]
fn undo_of_a_confirmation_brings_the_item_back_also_after_the_job_recorded_the_audio() {
    let (lib, mounted, guess, row) = a_name_guess();
    // Confirmed before the file was hashed: no audio is recorded yet.
    lib.confirm_item(row, guess).unwrap();
    assert_eq!(lib.said(), [("same".to_owned(), guess, None)]);
    // The hash stage runs, and the relink job fills the audio in, outside
    // the operation log.
    lib.hashed(guess, 7);
    lib.relink(&mounted);
    assert_eq!(lib.said(), [("same".to_owned(), guess, Some(7))]);

    assert_eq!(lib.undo_decision(), CONFIRM_OPERATION);
    assert_eq!(lib.said(), []);
    // Probable again as the undo returned, as it was found.
    assert_eq!(lib.matched(row), name_only(guess));
    assert!(lib.probable(row));
    assert_eq!(lib.relink_items(), [row]);
    // And the relink run decides the same from scratch.
    lib.relink(&mounted);
    assert_eq!(lib.matched(row), name_only(guess));
    assert!(lib.probable(row));
}

#[test]
fn only_a_match_that_is_waiting_can_be_confirmed_or_refused() {
    let (lib, music, mounted) = e_music();
    let own = lib.file(music, "Alpha.mp3", Some(200_000));
    let other = lib.file(music, "Beta.mp3", Some(200_000));
    let row = lib.track(&loc("E:/Music/Alpha.mp3"), Some("200"));
    lib.relink(&mounted);
    assert_eq!(lib.matched(row), path(own));

    // A trusted match isn't waiting; nor is another file, or no entry.
    assert!(matches!(lib.confirm_item(row, own), Err(Refused::ItemGone)));
    assert!(matches!(lib.reject_item(row, own), Err(Refused::ItemGone)));
    assert!(matches!(
        lib.confirm_item(row, other),
        Err(Refused::ItemGone)
    ));
    assert!(matches!(
        lib.confirm_item(9_999, own),
        Err(Refused::ItemGone)
    ));
    assert!(matches!(lib.withdraw_item(row), Err(Refused::ItemGone)));
    assert_eq!(lib.matched(row), path(own));
    assert_eq!(lib.said(), []);
    assert_eq!(lib.operation_kinds(), Vec::<String>::new());

    // Answered once, a match can't be answered again.
    let (lib, _mounted, guess, row) = a_name_guess();
    lib.confirm_item(row, guess).unwrap();
    assert!(matches!(
        lib.confirm_item(row, guess),
        Err(Refused::ItemGone)
    ));
    assert!(matches!(
        lib.reject_item(row, guess),
        Err(Refused::ItemGone)
    ));
    assert_eq!(lib.operation_kinds(), [CONFIRM_OPERATION]);
}

// ---- a confirmed match whose audio changed -------------------------------------------

#[test]
fn a_confirmed_match_whose_file_holds_other_audio_asks_again_until_confirmed_again() {
    let (lib, mounted, guess, row) = a_name_guess();
    lib.hashed(guess, 1);
    lib.confirm_item(row, guess).unwrap();
    assert_eq!(lib.said(), [("same".to_owned(), guess, Some(1))]);

    // The file is replaced by other audio under the same name.
    lib.hashed(guess, 2);
    lib.relink(&mounted);
    // The confirmation is kept and still holds the file, but it is only
    // probable: it waits in Review again, and says why.
    assert_eq!(
        lib.matched(row),
        Some((guess, "filename_only".to_owned(), 0.5))
    );
    assert!(lib.probable(row));
    assert_eq!(lib.relink_items(), [row]);
    let (offer, reason) = lib.asked(row).unwrap();
    assert_eq!(
        offer,
        RelinkOffer {
            file_id: guess,
            confirmed_before: true,
        }
    );
    assert_eq!(reason, "review:relink.user");

    // Confirmed again: trusted, for the audio it holds now.
    lib.confirm_item(row, guess).unwrap();
    assert_eq!(lib.matched(row), confirmed(guess));
    assert!(!lib.probable(row));
    assert_eq!(lib.said(), [("same".to_owned(), guess, Some(2))]);
    lib.relink(&mounted);
    assert_eq!(lib.matched(row), confirmed(guess));
    assert!(!lib.probable(row));

    // Undo of confirming again: the earlier confirmation is back, and with
    // it the question.
    assert_eq!(lib.undo_decision(), CONFIRM_OPERATION);
    assert_eq!(lib.said(), [("same".to_owned(), guess, Some(1))]);
    assert!(lib.probable(row));
    assert_eq!(lib.relink_items(), [row]);
    lib.relink(&mounted);
    assert!(lib.probable(row));
}

#[test]
fn not_the_same_on_a_match_confirmed_before_takes_the_confirmation_away_too() {
    let (lib, mounted, guess, row) = a_name_guess();
    lib.hashed(guess, 1);
    lib.confirm_item(row, guess).unwrap();
    lib.hashed(guess, 2);
    lib.relink(&mounted);
    assert!(lib.probable(row));

    lib.reject_item(row, guess).unwrap();
    assert_eq!(lib.said(), [("not_same".to_owned(), guess, Some(2))]);
    assert_eq!(lib.matched(row), None);
    lib.relink(&mounted);
    assert_eq!(lib.matched(row), None);

    // One Undo brings both back: the confirmation, and the question.
    assert_eq!(lib.undo_decision(), REJECT_OPERATION);
    assert_eq!(lib.said(), [("same".to_owned(), guess, Some(1))]);
    assert_eq!(
        lib.matched(row),
        Some((guess, "filename_only".to_owned(), 0.5))
    );
    assert!(lib.probable(row));
}

// ---- not the same -----------------------------------------------------------------------

#[test]
fn not_the_same_takes_the_match_away_at_once_and_no_run_makes_it_again() {
    let (lib, mounted, guess, row) = a_name_guess();
    assert!(lib.reject_item(row, guess).unwrap().is_some());
    assert_eq!(lib.matched(row), None);
    assert!(!lib.probable(row));
    assert_eq!(lib.relink_items(), Vec::<i64>::new());
    assert_eq!(lib.operation_kinds(), [REJECT_OPERATION]);
    assert_eq!(lib.said(), [("not_same".to_owned(), guess, None)]);

    let summary = lib.relink(&mounted);
    assert_eq!(lib.matched(row), None);
    assert_eq!(summary.missing, 1);
    lib.fresh_read();
    lib.relink(&mounted);
    assert_eq!(lib.matched(row), None);
    assert_eq!(lib.relink_items(), Vec::<i64>::new());
}

#[test]
fn undo_of_not_the_same_brings_the_match_and_its_item_back() {
    let (lib, mounted, guess, row) = a_name_guess();
    lib.reject_item(row, guess).unwrap();
    assert_eq!(lib.undo_decision(), REJECT_OPERATION);
    assert_eq!(lib.said(), []);
    assert_eq!(lib.matched(row), name_only(guess));
    assert!(lib.probable(row));
    assert_eq!(lib.relink_items(), [row]);
    lib.relink(&mounted);
    assert_eq!(lib.matched(row), name_only(guess));
    assert!(lib.probable(row));
}

#[test]
fn after_not_the_same_another_file_can_be_proposed_and_that_is_a_new_item() {
    let (lib, mounted, guess, row) = a_name_guess();
    lib.reject_item(row, guess).unwrap();
    lib.relink(&mounted);
    assert_eq!(lib.matched(row), None);

    // The refused file goes, and another file with the name turns up.
    lib.gone(guess);
    let other = lib.file(1, "Other place/Omicron.mp3", None);
    lib.relink(&mounted);
    assert_eq!(lib.matched(row), name_only(other));
    assert!(lib.probable(row));
    assert_eq!(lib.relink_items(), [row]);
    assert_eq!(lib.asked(row).map(|(offer, _)| offer.file_id), Some(other));
    // The first answer still stands for the first file.
    lib.back(guess);
    lib.gone(other);
    lib.relink(&mounted);
    assert_eq!(lib.matched(row), None);
}

#[test]
fn a_refused_file_still_makes_its_name_ambiguous_so_saying_no_never_makes_a_match() {
    let (lib, mounted, guess, row) = a_name_guess();
    lib.reject_item(row, guess).unwrap();
    // A second file with the name: two files, so the name says nothing,
    // whatever was said about one of them.
    let second = lib.file(1, "Other place/Omicron.mp3", None);
    lib.audio_hash(guess, 1);
    lib.audio_hash(second, 2);
    lib.relink(&mounted);
    assert_eq!(lib.matched(row), None);
}

#[test]
fn not_the_same_stands_in_the_way_in_every_step_from_two_to_five() {
    // Step 2: the same file name and a fitting length.
    let (lib, music, mounted) = e_music();
    let file = lib.file(music, "Elsewhere/Alpha.mp3", Some(200_000));
    let location = loc("E:/Gone/Alpha.mp3");
    let row = lib.track(&location, Some("200"));
    lib.relink(&mounted);
    assert_eq!(lib.method(row).as_deref(), Some("filename_duration"));
    lib.said_not_the_same(&location, file, Method::FilenameDuration);
    lib.relink(&mounted);
    assert_eq!(lib.matched(row), None, "step 2 (and 5)");

    // Step 3: the only file of that length in the Location's own folder.
    let (lib, music, mounted) = e_music();
    let file = lib.file(music, "Renamed.mp3", Some(200_000));
    let location = loc("E:/Music/Beta.mp3");
    let row = lib.track(&location, Some("200"));
    lib.relink(&mounted);
    assert_eq!(lib.method(row).as_deref(), Some("unique_duration"));
    lib.said_not_the_same(&location, file, Method::UniqueDuration);
    lib.relink(&mounted);
    assert_eq!(lib.matched(row), None, "step 3");

    // Step 4: a file with the audio the gone file had.
    let (lib, music, mounted) = e_music();
    let was = lib.file(music, "Gamma.mp3", Some(200_000));
    lib.hashed(was, 1);
    lib.gone(was);
    let file = lib.file(music, "Moved/Third.mp3", Some(200_000));
    lib.hashed(file, 1);
    let location = loc("E:/Music/Gamma.mp3");
    let row = lib.track(&location, Some("200"));
    lib.relink(&mounted);
    assert_eq!(lib.matched(row), same_audio(file));
    lib.said_not_the_same(&location, file, Method::Fingerprint);
    lib.relink(&mounted);
    assert_eq!(lib.matched(row), None, "step 4");

    // Step 5 is `not_the_same_takes_the_match_away_at_once…` above.
}

#[test]
fn a_file_at_the_locations_own_path_is_matched_whatever_was_said() {
    // Step 1 isn't a guess: it is the file rekordbox itself opens.
    let (lib, music, mounted) = e_music();
    let own = lib.file(music, "Alpha.mp3", Some(200_000));
    let location = loc("E:/Music/Alpha.mp3");
    let row = lib.track(&location, Some("200"));
    lib.said_not_the_same(&location, own, Method::FilenameOnly);
    lib.relink(&mounted);
    assert_eq!(lib.matched(row), path(own));
}

#[test]
fn a_carried_match_to_a_refused_file_is_dropped() {
    let (lib, music, mounted) = e_music();
    let file = lib.file(music, "Elsewhere/Delta.mp3", Some(300_000));
    let location = loc("E:/Gone/Other name.mp3");
    let row = lib.track(&location, Some("200"));
    lib.matched_before(row, file, "gig_stick", 0.7);
    lib.relink(&mounted);
    assert_eq!(lib.method(row).as_deref(), Some("gig_stick"));
    lib.said_not_the_same(&location, file, Method::GigStick);
    lib.relink(&mounted);
    assert_eq!(lib.matched(row), None);
}

#[test]
fn a_not_the_same_about_a_file_that_holds_other_audio_now_no_longer_stands_in_the_way() {
    let (lib, mounted, guess, row) = a_name_guess();
    lib.hashed(guess, 1);
    lib.reject_item(row, guess).unwrap();
    assert_eq!(lib.said(), [("not_same".to_owned(), guess, Some(1))]);
    lib.relink(&mounted);
    assert_eq!(lib.matched(row), None);

    // Replaced by other audio under the same name: another question.
    lib.hashed(guess, 2);
    lib.relink(&mounted);
    assert_eq!(lib.matched(row), name_only(guess));
    assert!(lib.probable(row));
    assert_eq!(lib.relink_items(), [row]);
    // Said again, for the audio of now: it stands again.
    lib.reject_item(row, guess).unwrap();
    assert_eq!(lib.said(), [("not_same".to_owned(), guess, Some(2))]);
    lib.relink(&mounted);
    assert_eq!(lib.matched(row), None);
}

#[test]
fn a_not_the_same_said_before_the_file_was_hashed_gets_its_audio_recorded_by_the_job() {
    let (lib, mounted, guess, row) = a_name_guess();
    lib.reject_item(row, guess).unwrap();
    assert_eq!(lib.said(), [("not_same".to_owned(), guess, None)]);
    lib.hashed(guess, 4);
    lib.relink(&mounted);
    assert_eq!(lib.said(), [("not_same".to_owned(), guess, Some(4))]);
    // Undo still works after that.
    assert_eq!(lib.undo_decision(), REJECT_OPERATION);
    assert_eq!(lib.said(), []);
    assert_eq!(lib.matched(row), name_only(guess));
}

#[test]
fn confirming_a_file_takes_back_a_not_the_same_said_about_it() {
    let (lib, mounted, guess, row) = a_name_guess();
    lib.hashed(guess, 1);
    lib.reject_item(row, guess).unwrap();
    lib.hashed(guess, 2);
    lib.relink(&mounted);
    assert_eq!(lib.matched(row), name_only(guess));

    lib.confirm_item(row, guess).unwrap();
    assert_eq!(lib.said(), [("same".to_owned(), guess, Some(2))]);
    assert_eq!(lib.matched(row), confirmed(guess));
    lib.relink(&mounted);
    assert_eq!(lib.matched(row), confirmed(guess));
}

// ---- withdraw ---------------------------------------------------------------------------

#[test]
fn withdrawing_a_confirmation_returns_the_entry_to_recomputation() {
    let (lib, mounted, guess, row) = a_name_guess();
    lib.confirm_item(row, guess).unwrap();
    lib.relink(&mounted);
    assert_eq!(lib.matched(row), confirmed(guess));

    assert!(lib.withdraw_item(row).unwrap().is_some());
    assert_eq!(
        lib.operation_kinds(),
        [CONFIRM_OPERATION, WITHDRAW_OPERATION]
    );
    assert_eq!(lib.said(), []);
    // Probable again at once, as it was found; the run decides it from
    // scratch, as if it had never been confirmed.
    assert_eq!(lib.matched(row), name_only(guess));
    assert!(lib.probable(row));
    assert_eq!(lib.relink_items(), [row]);
    lib.relink(&mounted);
    assert_eq!(lib.matched(row), name_only(guess));
    assert!(lib.probable(row));

    // Undo of the withdrawal: confirmed again.
    assert_eq!(lib.undo_decision(), WITHDRAW_OPERATION);
    assert_eq!(lib.said().len(), 1);
    assert_eq!(lib.matched(row), confirmed(guess));
    assert!(!lib.probable(row));
    lib.relink(&mounted);
    assert_eq!(lib.matched(row), confirmed(guess));
}

// ---- the add to the Library ---------------------------------------------------------------

const NO_DIRS: crate::fragile::FragileDirs = crate::fragile::FragileDirs {
    downloads: None,
    temp: Vec::new(),
};

#[test]
fn a_track_whose_only_match_is_probable_cannot_be_added_until_it_is_confirmed() {
    let (lib, mounted, guess, row) = a_name_guess();
    let recording = lib.group(&[guess]);
    lib.relink(&mounted);
    let add = || {
        promote_with(
            &lib.writer,
            &Drives,
            &NO_DIRS,
            recording,
            Confirmed::default(),
        )
    };
    assert!(matches!(add(), Err(LibraryError::MatchNotConfirmed)));

    lib.confirm_item(row, guess).unwrap();
    // Confirmed: the add works at once, links the file rekordbox uses, and
    // the track is sent as rekordbox's own entry.
    let added = add().unwrap().done();
    assert!(added.added);
    assert_eq!(lib.linked_file(added.library_track.id), guess);
    assert_eq!(lib.sent(added.library_track.id), SentAs::Entry(row));
    lib.relink(&mounted);
    assert_eq!(lib.sent(added.library_track.id), SentAs::Entry(row));
}

#[test]
fn after_not_the_same_the_track_can_be_added_as_one_rekordbox_does_not_know() {
    let (lib, mounted, guess, row) = a_name_guess();
    let recording = lib.group(&[guess]);
    lib.relink(&mounted);
    lib.reject_item(row, guess).unwrap();
    let added = promote_with(
        &lib.writer,
        &Drives,
        &NO_DIRS,
        recording,
        Confirmed::default(),
    )
    .unwrap()
    .done();
    assert_eq!(lib.linked_file(added.library_track.id), guess);
    assert_eq!(lib.sent(added.library_track.id), SentAs::New);
    lib.relink(&mounted);
    assert_eq!(lib.matched(row), None);
    assert_eq!(lib.sent(added.library_track.id), SentAs::New);
}
