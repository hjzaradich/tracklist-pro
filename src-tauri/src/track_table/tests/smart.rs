//! A smart crate as a table source (3.3): its rows and its live count come
//! from the one condition the send uses, over the index the job keeps.

use super::*;
use crate::crates::smart::{
    create_smart_on, include_on, membership, Combine, Condition, Inclusion, NumberOp, SmartRule,
    TextOp,
};

fn bpm_rule(value: u16, half_double: bool) -> SmartRule {
    SmartRule {
        combine: Combine::All,
        conditions: vec![Condition::Bpm {
            op: NumberOp::Is,
            value,
            to: None,
            half_double,
        }],
    }
}

fn library_track(lib: &Lib, recording: i64) -> LibraryTrackId {
    lib.writer
        .call(move |c| {
            c.query_row(
                "SELECT id FROM library_track WHERE recording_id = ?1",
                [recording],
                |r| r.get(0),
            )
        })
        .map(LibraryTrackId)
        .unwrap()
}

#[test]
fn a_draft_rule_lists_and_counts_the_library_tracks_it_matches() {
    let f = fixture();
    let [r1, r2, ..] = f.recs;
    let draft = |rule| TableSource::Smart {
        id: None,
        rule: Some(rule),
    };
    // 124 BPM: Delta Drive; Alpha Wave (not in the Library) is left out.
    let page = run(&f.lib, query(draft(bpm_rule(124, false))));
    assert_eq!((ids(&page), page.total), (vec![r1], Some(1)));
    // 64 at half/double time is 128: alpha wave.
    let page = run(&f.lib, query(draft(bpm_rule(64, true))));
    assert_eq!(ids(&page), vec![r2]);
    // The genre, as the table shows it, and any of two.
    let rule = SmartRule {
        combine: Combine::Any,
        conditions: vec![
            Condition::Genre {
                op: TextOp::Is,
                value: "HOUSE".to_owned(),
            },
            Condition::Bpm {
                op: NumberOp::AtMost,
                value: 100,
                to: None,
                half_double: false,
            },
        ],
    };
    let page = run(&f.lib, query(draft(rule)));
    assert_eq!(page.total, Some(2));
}

#[test]
fn a_saved_smart_crate_lists_what_a_send_writes() {
    let f = fixture();
    let [r1, r2, _, r4, ..] = f.recs;
    let rule = bpm_rule(124, true);
    let id = f
        .lib
        .writer
        .call(move |c| Ok(create_smart_on(c, None, "Peak", &rule)))
        .unwrap()
        .unwrap()
        .unwrap();
    let (keep, drop) = (library_track(&f.lib, r4), library_track(&f.lib, r1));
    for (track, list) in [(keep, Inclusion::Always), (drop, Inclusion::Never)] {
        f.lib
            .writer
            .call(move |c| Ok(include_on(c, id, &[track], list)))
            .unwrap()
            .unwrap()
            .unwrap();
    }
    let source = TableSource::Smart {
        id: Some(id),
        rule: None,
    };
    let page = run(&f.lib, query(source.clone()));
    assert_eq!(ids(&page), vec![r4]);
    let sent = f
        .lib
        .writer
        .call(move |c| membership::evaluate(c, id))
        .unwrap();
    assert_eq!(sent, vec![keep]);
    // A draft for the crate keeps its lists: 128 adds alpha wave.
    let draft = TableSource::Smart {
        id: Some(id),
        rule: Some(bpm_rule(128, false)),
    };
    assert_eq!(ids(&run(&f.lib, query(draft))), vec![r2, r4]);
    // A smart crate that isn't there is no page, as a crate isn't.
    let gone = TableSource::Smart {
        id: Some(CrateId(id.0 + 99)),
        rule: None,
    };
    assert!(run_as_is(&f.lib, query(gone)).is_none());
    // Its formats and a track's place in it read the same rows.
    let formats = f.lib.writer.call(move |c| formats(c, &source)).unwrap();
    assert_eq!(formats, Some(vec!["mp3".to_owned()]));
}
