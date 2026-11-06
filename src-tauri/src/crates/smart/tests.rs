//! Smart crate tests, over a migrated database in a temp dir. Everything is
//! synthetic: made-up titles, and index rows (`track_row`) written straight
//! in, so a rule is checked against exactly the values given.

use proptest::prelude::*;
use proptest::test_runner::{Config, TestRunner};
use rusqlite::types::Value;
use rusqlite::Connection;

use super::compile::compile;
use super::membership::{condition, count, evaluate};
use super::*;
use crate::crates::{add_on, add_recordings_on, create_on, remove_on};
use crate::db::Writer;
use crate::ops::{undo_last_via, UndoOutcome};
use crate::search::flag;

struct Lib {
    writer: Writer,
    _dir: tempfile::TempDir,
}

/// What a test track's index row holds.
#[derive(Default, Clone, Copy)]
struct Facts {
    energy: u8,
    bpm: Option<f64>,
    key: Option<&'static str>,
    genre: Option<&'static str>,
    rating: Option<i64>,
}

impl Lib {
    fn new() -> Lib {
        let dir = tempfile::tempdir().unwrap();
        let writer = Writer::open(&crate::write_guard::test_path(
            dir.path(),
            crate::db::DB_FILE_NAME,
        ))
        .unwrap();
        Lib { _dir: dir, writer }
    }

    fn with<T: Send + 'static>(
        &self,
        f: impl FnOnce(&mut Connection) -> rusqlite::Result<T> + Send + 'static,
    ) -> T {
        self.writer.call(f).unwrap()
    }

    /// A track with an index row; a Library track too when `library`.
    /// Answers (recording id, Library track id).
    fn track(
        &self,
        title: &'static str,
        library: bool,
        facts: Facts,
    ) -> (i64, Option<LibraryTrackId>) {
        self.with(move |c| {
            c.execute(
                "INSERT INTO recording (title, artist) VALUES (?1, 'Kit')",
                [title],
            )?;
            let recording = c.last_insert_rowid();
            let mut flags = flag::energy(facts.energy);
            let mut track = None;
            if library {
                c.execute(
                    "INSERT INTO library_track (recording_id, source_status) VALUES (?1, 'missing')",
                    [recording],
                )?;
                track = Some(LibraryTrackId(c.last_insert_rowid()));
                flags |= flag::IN_LIBRARY;
            }
            let key = facts.key.and_then(crate::search::camelot_order);
            c.execute(
                "INSERT INTO track_row (recording_id, flags, title_key, bpm, key_order, rating,
                                        genre, genre_group)
                 VALUES (?1, ?2, lower(?3), ?4, ?5, ?6, ?7, lower(trim(?7)))",
                rusqlite::params![recording, flags, title, facts.bpm, key, facts.rating, facts.genre],
            )?;
            Ok((recording, track))
        })
    }

    fn create(&self, name: &'static str, rule: SmartRule) -> CrateId {
        self.writer
            .call(move |c| Ok(create_smart_on(c, None, name, &rule)))
            .unwrap()
            .unwrap()
            .unwrap()
    }

    fn held(&self, id: CrateId) -> Vec<LibraryTrackId> {
        self.with(move |c| evaluate(c, id))
    }

    /// A tag group and its tags (made in order), each applied to the given
    /// recordings. Answers the group and the tags.
    fn tags(
        &self,
        group: &'static str,
        tags: Vec<(&'static str, Vec<i64>)>,
    ) -> (TagGroupId, Vec<TagId>) {
        self.with(move |c| {
            let g = crate::vocab::create_group_on(c, group).unwrap().unwrap();
            let mut made = Vec::new();
            for (name, recordings) in tags {
                let t = crate::vocab::create_tag_on(c, g, name).unwrap().unwrap();
                crate::vocab::apply_on(c, t, &recordings).unwrap().unwrap();
                made.push(t);
            }
            Ok((g, made))
        })
    }

    fn undo(&self) {
        match undo_last_via(&self.writer).unwrap() {
            UndoOutcome::Undone { .. } => {}
            other => panic!("expected the undo to work, got {other:?}"),
        }
    }
}

fn one(condition: Condition) -> SmartRule {
    SmartRule {
        combine: Combine::All,
        conditions: vec![condition],
    }
}

fn energy(op: NumberOp, value: u8) -> Condition {
    Condition::Energy {
        op,
        value,
        to: None,
    }
}

/// The tracks a draft rule matches, Library tracks only, as the live count
/// reads them: (recording ids in title order).
fn matched(lib: &Lib, rule: SmartRule) -> Vec<i64> {
    lib.with(move |c| {
        let mut params = Vec::new();
        let sql = condition(None, Some(&rule), &mut params);
        c.prepare(&format!(
            "SELECT tr.recording_id FROM track_row tr WHERE {sql} ORDER BY tr.title_key"
        ))?
        .query_map(rusqlite::params_from_iter(params.iter()), |r| r.get(0))?
        .collect()
    })
}

/// A small world: five Library tracks and one outside the Library that
/// matches everything.
struct World {
    lib: Lib,
    rec: [i64; 6],
    lt: [Option<LibraryTrackId>; 6],
}

fn world() -> World {
    let lib = Lib::new();
    let tracks = [
        (
            "a peak",
            true,
            Facts {
                energy: 8,
                bpm: Some(126.2),
                key: Some("8A"),
                genre: Some("Deep House"),
                rating: Some(5),
            },
        ),
        (
            "b warm",
            true,
            Facts {
                energy: 4,
                bpm: Some(118.0),
                key: Some("9A"),
                genre: Some("house"),
                rating: Some(3),
            },
        ),
        (
            "c half",
            true,
            Facts {
                energy: 7,
                bpm: Some(63.0),
                key: Some("8B"),
                genre: Some("Techno"),
                rating: Some(0),
            },
        ),
        ("d blank", true, Facts::default()),
        (
            "e fast",
            true,
            Facts {
                energy: 10,
                bpm: Some(174.0),
                key: Some("1A"),
                genre: Some("Drum & Bass"),
                rating: Some(4),
            },
        ),
        (
            "f outside",
            false,
            Facts {
                energy: 9,
                bpm: Some(126.0),
                key: Some("8A"),
                genre: Some("House"),
                rating: Some(5),
            },
        ),
    ];
    let mut rec = [0; 6];
    let mut lt = [None; 6];
    for (i, (title, library, facts)) in tracks.into_iter().enumerate() {
        (rec[i], lt[i]) = lib.track(title, library, facts);
    }
    World { lib, rec, lt }
}

#[test]
fn each_field_and_operator_matches_the_library_tracks_it_says() {
    let w = world();
    let r = |is: &[usize]| is.iter().map(|&i| w.rec[i]).collect::<Vec<_>>();
    let cases: Vec<(Condition, Vec<i64>)> = vec![
        (energy(NumberOp::AtLeast, 7), r(&[0, 2, 4])),
        (energy(NumberOp::AtMost, 7), r(&[1, 2])),
        (energy(NumberOp::Is, 4), r(&[1])),
        // No energy is no value: "is not" leaves it out too.
        (energy(NumberOp::IsNot, 4), r(&[0, 2, 4])),
        (
            Condition::Energy {
                op: NumberOp::Between,
                value: 8,
                to: Some(4),
            },
            r(&[0, 1, 2]),
        ),
        (
            Condition::Bpm {
                op: NumberOp::Between,
                value: 120,
                to: Some(126),
                half_double: false,
            },
            r(&[0]),
        ),
        (
            Condition::Bpm {
                op: NumberOp::Is,
                value: 126,
                half_double: true,
                to: None,
            },
            r(&[0, 2]),
        ),
        (
            Condition::Bpm {
                op: NumberOp::AtLeast,
                value: 170,
                half_double: false,
                to: None,
            },
            r(&[4]),
        ),
        (
            Condition::Bpm {
                op: NumberOp::IsNot,
                value: 118,
                half_double: false,
                to: None,
            },
            r(&[0, 2, 4]),
        ),
        (
            Condition::Key {
                op: ChoiceOp::Is,
                value: "8a".to_owned(),
            },
            r(&[0]),
        ),
        (
            Condition::Key {
                op: ChoiceOp::IsNot,
                value: "8A".to_owned(),
            },
            r(&[1, 2, 4]),
        ),
        (
            Condition::Key {
                op: ChoiceOp::Is,
                value: "Am".to_owned(),
            },
            vec![],
        ),
        (
            Condition::Genre {
                op: TextOp::Is,
                value: " HOUSE ".to_owned(),
            },
            r(&[1]),
        ),
        (
            Condition::Genre {
                op: TextOp::Contains,
                value: "house".to_owned(),
            },
            r(&[0, 1]),
        ),
        (
            Condition::Genre {
                op: TextOp::IsNot,
                value: "house".to_owned(),
            },
            r(&[0, 2, 4]),
        ),
        (
            Condition::Rating {
                op: NumberOp::Between,
                value: 3,
                to: Some(5),
            },
            r(&[0, 1, 4]),
        ),
        // 0 stars is unrated: no value, so nothing is "0", and "is not" and
        // "at most" leave it out like a missing one.
        (
            Condition::Rating {
                op: NumberOp::Is,
                value: 0,
                to: None,
            },
            vec![],
        ),
        (
            Condition::Rating {
                op: NumberOp::IsNot,
                value: 5,
                to: None,
            },
            r(&[1, 4]),
        ),
        (
            Condition::Rating {
                op: NumberOp::AtMost,
                value: 2,
                to: None,
            },
            vec![],
        ),
        // Nothing typed yet is nothing to contain, not "every track with a genre".
        (
            Condition::Genre {
                op: TextOp::Contains,
                value: String::new(),
            },
            vec![],
        ),
        (
            Condition::Genre {
                op: TextOp::Contains,
                value: "  ".to_owned(),
            },
            vec![],
        ),
    ];
    for (condition, expected) in cases {
        assert_eq!(
            matched(&w.lib, one(condition.clone())),
            expected,
            "{condition:?}"
        );
    }
    // No condition: no track.
    let none = SmartRule {
        combine: Combine::All,
        conditions: vec![],
    };
    assert!(matched(&w.lib, none).is_empty());
    // All and any.
    let both = vec![
        energy(NumberOp::AtLeast, 7),
        Condition::Genre {
            op: TextOp::Contains,
            value: "house".to_owned(),
        },
    ];
    let all = SmartRule {
        combine: Combine::All,
        conditions: both.clone(),
    };
    let any = SmartRule {
        combine: Combine::Any,
        conditions: both,
    };
    assert_eq!(matched(&w.lib, all), r(&[0]));
    assert_eq!(matched(&w.lib, any), r(&[0, 1, 2, 4]));
}

#[test]
fn in_crate_reads_a_hand_made_crates_members() {
    let w = world();
    let made = w
        .lib
        .writer
        .call(|c| Ok(create_on(c, "Picked")))
        .unwrap()
        .unwrap()
        .unwrap();
    let picked = vec![w.lt[1].unwrap(), w.lt[4].unwrap()];
    w.lib
        .writer
        .call(move |c| Ok(add_on(c, made, &picked)))
        .unwrap()
        .unwrap()
        .unwrap();
    let is = Condition::InCrate {
        op: ChoiceOp::Is,
        crate_id: made,
    };
    let is_not = Condition::InCrate {
        op: ChoiceOp::IsNot,
        crate_id: made,
    };
    assert_eq!(matched(&w.lib, one(is)), vec![w.rec[1], w.rec[4]]);
    assert_eq!(
        matched(&w.lib, one(is_not)),
        vec![w.rec[0], w.rec[2], w.rec[3]]
    );
}

#[test]
fn a_bpm_of_zero_is_no_bpm_so_bpm_is_not_leaves_it_out() {
    let lib = Lib::new();
    let facts = |bpm: f64| Facts {
        bpm: Some(bpm),
        ..Facts::default()
    };
    let (zero, _) = lib.track("a zero", true, facts(0.0));
    let (same, _) = lib.track("b same", true, facts(118.0));
    let (other, _) = lib.track("c other", true, facts(130.0));
    let (none, _) = lib.track("d none", true, Facts::default());
    let bpm = |op, half_double| Condition::Bpm {
        op,
        value: 118,
        to: None,
        half_double,
    };
    assert_eq!(matched(&lib, one(bpm(NumberOp::IsNot, false))), vec![other]);
    assert_eq!(matched(&lib, one(bpm(NumberOp::IsNot, true))), vec![other]);
    assert_eq!(matched(&lib, one(bpm(NumberOp::Is, false))), vec![same]);
    let _ = (zero, none);
}

#[test]
fn in_crate_is_an_uncorrelated_list_not_a_lookup_per_track() {
    for op in [ChoiceOp::Is, ChoiceOp::IsNot] {
        let mut params = Vec::new();
        let sql = compile(
            &one(Condition::InCrate {
                op,
                crate_id: CrateId(7),
            }),
            &mut params,
        );
        assert!(sql.contains("IN (SELECT"), "{sql}");
        assert!(!sql.contains("EXISTS"), "{sql}");
        // The subquery never names the outer row, so it is built once.
        let inner = &sql[sql.find("(SELECT").unwrap()..];
        assert!(!inner.contains("tr."), "{sql}");
    }
}

#[test]
fn a_smart_crate_holds_the_rule_plus_always_less_never_and_each_list_undoes() {
    let w = world();
    let id = w.lib.create("Peak", one(energy(NumberOp::AtLeast, 7)));
    let lts = w.lt;
    let lt = move |i: usize| lts[i].unwrap();
    assert_eq!(w.lib.held(id), vec![lt(0), lt(2), lt(4)]);

    // Dropped on it (by Library track, and by track id): always included.
    let changed = w
        .lib
        .writer
        .call(move |c| Ok(add_on(c, id, &[lt(1)])))
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(changed.changed, 1);
    let rec3 = w.rec[3];
    let added = w
        .lib
        .writer
        .call(move |c| Ok(add_recordings_on(c, id, &[rec3])))
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(added.added, 1);
    assert_eq!(w.lib.held(id), vec![lt(0), lt(1), lt(2), lt(3), lt(4)]);

    // Removed from it: never included, whether the rule or the list had it.
    w.lib
        .writer
        .call(move |c| Ok(remove_on(c, id, &[lt(0), lt(1)])))
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(w.lib.held(id), vec![lt(2), lt(3), lt(4)]);
    let kinds = w.lib.with(move |c| {
        c.prepare("SELECT kind FROM crate_entry WHERE crate_id = ?1 ORDER BY library_track_id")?
            .query_map([id.0], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()
    });
    assert_eq!(kinds, ["never_include", "never_include", "always_include"]);
    let last = w.lib.with(|c| {
        c.query_row(
            "SELECT kind FROM operation ORDER BY id DESC LIMIT 1",
            [],
            |r| r.get::<_, String>(0),
        )
    });
    assert_eq!(last, NEVER_OPERATION);

    // Each is one operation; undo walks them back.
    w.lib.undo();
    assert_eq!(w.lib.held(id), vec![lt(0), lt(1), lt(2), lt(3), lt(4)]);
    w.lib.undo();
    w.lib.undo();
    assert_eq!(w.lib.held(id), vec![lt(0), lt(2), lt(4)]);

    // A new rule is its own operation; the same rule records nothing.
    let rule = one(Condition::Genre {
        op: TextOp::Is,
        value: "house".to_owned(),
    });
    let again = rule.clone();
    let op = w
        .lib
        .writer
        .call(move |c| Ok(set_rule_on(c, id, &rule)))
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(op.is_some());
    let none = w
        .lib
        .writer
        .call(move |c| Ok(set_rule_on(c, id, &again)))
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(none.is_none());
    assert_eq!(w.lib.held(id), vec![lt(1)]);
    w.lib.undo();
    assert_eq!(w.lib.held(id), vec![lt(0), lt(2), lt(4)]);
    w.lib.undo();
    assert!(w
        .lib
        .with(move |c| super::membership::stored_rule(c, id))
        .is_none());
}

#[test]
fn a_rule_that_does_not_read_matches_nothing_and_the_always_list_still_goes() {
    let w = world();
    let id = w.lib.create("Odd", one(energy(NumberOp::AtLeast, 1)));
    let keep = w.lt[3].unwrap();
    w.lib
        .writer
        .call(move |c| Ok(include_on(c, id, &[keep], Inclusion::Always)))
        .unwrap()
        .unwrap()
        .unwrap();
    w.lib.with(move |c| {
        c.execute(
            "UPDATE crate SET rules = '{\"combine\":\"all\",\"conditions\":[{\"field\":\"tag\"}]}' WHERE id = ?1",
            [id.0],
        )
    });
    assert_eq!(w.lib.held(id), vec![keep]);
    assert_eq!(w.lib.with(move |c| count(c, id)), 1);
}

#[test]
fn a_rule_too_long_is_refused_before_anything_is_written() {
    let w = world();
    let long = SmartRule {
        combine: Combine::Any,
        conditions: vec![energy(NumberOp::Is, 5); MAX_CONDITIONS + 1],
    };
    let refused = w
        .lib
        .writer
        .call(move |c| Ok(create_smart_on(c, None, "Long", &long)))
        .unwrap()
        .unwrap();
    assert!(matches!(refused, Err(CratesError::RuleTooLong)));
    let text = one(Condition::Genre {
        op: TextOp::Is,
        value: "x".repeat(MAX_TEXT_CHARS + 1),
    });
    let refused = w
        .lib
        .writer
        .call(move |c| Ok(create_smart_on(c, None, "Long", &text)))
        .unwrap()
        .unwrap();
    assert!(matches!(refused, Err(CratesError::RuleTooLong)));
    assert_eq!(
        w.lib
            .with(|c| c.query_row("SELECT count(*) FROM crate", [], |r| r.get::<_, i64>(0))),
        0
    );
}

#[test]
fn a_send_writes_a_smart_crate_as_it_evaluates_and_the_merge_never_compares_it() {
    let w = world();
    let id = w.lib.create("Peak", one(energy(NumberOp::AtLeast, 7)));
    let tree = w.lib.with(|c| crate::send::crate_tree(c));
    let lt = |i: usize| w.lt[i].unwrap();
    assert_eq!(
        tree,
        vec![crate::rekordbox_write::Node::Playlist {
            name: "Peak".to_owned(),
            entries: vec![lt(0), lt(2), lt(4)],
        }]
    );
    // The app owns the playlist: a membership conflict's field at its path
    // names no crate, so nothing from rekordbox is merged into it.
    let key = crate::rekordbox_write::sibling_key;
    let field = crate::conflicts::membership::field_of(&[
        key(crate::rekordbox_write::CRATES_FOLDER),
        key("Peak"),
    ]);
    assert_eq!(
        w.lib
            .with(move |c| crate::conflicts::membership::crate_of(c, &field)),
        None
    );
    // The list names it without counting it (a count is a scan of the Library
    // per smart crate); the count is asked for on its own.
    let listed = w.lib.with(|c| crate::crates::list(c));
    assert_eq!(listed[0].kind, CrateKind::Smart);
    assert_eq!(listed[0].id, id);
    assert_eq!(listed[0].track_count, None);
    assert_eq!(w.lib.with(move |c| count(c, id)), 3);
}

fn has(tag: TagId) -> Condition {
    Condition::Tag {
        op: TagOp::Has,
        tag_id: tag,
    }
}

fn has_not(tag: TagId) -> Condition {
    Condition::Tag {
        op: TagOp::HasNot,
        tag_id: tag,
    }
}

#[test]
fn has_a_tag_and_does_not_have_a_tag_match_the_library_tracks_they_say() {
    let w = world();
    let r = w.rec;
    // peak: a, c, and the outside track f; warm: b.
    let (_, tags) = w.lib.tags(
        "Role",
        vec![("peak", vec![r[0], r[2], r[5]]), ("warm", vec![r[1]])],
    );
    let (peak, warm) = (tags[0], tags[1]);
    assert_eq!(matched(&w.lib, one(has(peak))), vec![r[0], r[2]]);
    assert_eq!(matched(&w.lib, one(has(warm))), vec![r[1]]);
    // A track with no tag at all doesn't have the tag.
    assert_eq!(matched(&w.lib, one(has_not(peak))), vec![r[1], r[3], r[4]]);
    assert_eq!(
        matched(&w.lib, one(has_not(warm))),
        vec![r[0], r[2], r[3], r[4]]
    );
}

#[test]
fn has_any_tag_in_a_group_matches_a_track_with_one_of_its_tags_and_not_another_groups() {
    let w = world();
    let r = w.rec;
    let (role, _) = w.lib.tags(
        "Role",
        vec![("peak", vec![r[0]]), ("warm", vec![r[1], r[0]])],
    );
    let (vocal, _) = w.lib.tags("Vocal", vec![("acapella", vec![r[4]])]);
    let in_group = |op, group_id| Condition::TagGroup { op, group_id };
    assert_eq!(
        matched(&w.lib, one(in_group(TagOp::Has, role))),
        vec![r[0], r[1]]
    );
    assert_eq!(
        matched(&w.lib, one(in_group(TagOp::Has, vocal))),
        vec![r[4]]
    );
    assert_eq!(
        matched(&w.lib, one(in_group(TagOp::HasNot, role))),
        vec![r[2], r[3], r[4]]
    );
}

#[test]
fn a_deleted_tag_or_group_matches_no_track_and_the_rule_stays_readable() {
    let w = world();
    let r = w.rec;
    let (group, tags) = w
        .lib
        .tags("Role", vec![("peak", vec![r[0]]), ("warm", vec![r[1]])]);
    let (peak, warm) = (tags[0], tags[1]);
    let rule = SmartRule {
        combine: Combine::Any,
        conditions: vec![has(peak), has_not(peak)],
    };
    let saved = w.lib.create("Tagged", rule.clone());
    assert_eq!(w.lib.held(saved).len(), 5);
    // The vocabulary can't delete a tag yet; take it out of the database.
    w.lib.with(move |c| {
        c.execute("DELETE FROM tag_applied WHERE tag_id = ?1", [peak.0])?;
        c.execute("DELETE FROM tag WHERE id = ?1", [peak.0])?;
        Ok(())
    });
    for condition in [has(peak), has_not(peak)] {
        assert!(matched(&w.lib, one(condition)).is_empty());
    }
    assert!(w.lib.held(saved).is_empty());
    // The other tag is untouched.
    assert_eq!(matched(&w.lib, one(has(warm))), vec![r[1]]);
    // The stored rule reads back as it was saved.
    let stored = w
        .lib
        .with(move |c| membership::stored_rule(c, saved))
        .unwrap()
        .unwrap();
    assert_eq!(stored, rule);
    // With the group gone too, nothing has a tag of it and nothing lacks one.
    w.lib.with(move |c| {
        c.execute("DELETE FROM tag_applied", [])?;
        c.execute("DELETE FROM tag", [])?;
        c.execute("DELETE FROM tag_group WHERE id = ?1", [group.0])?;
        Ok(())
    });
    let in_group = |op| Condition::TagGroup {
        op,
        group_id: group,
    };
    assert!(matched(&w.lib, one(in_group(TagOp::Has))).is_empty());
    assert!(matched(&w.lib, one(in_group(TagOp::HasNot))).is_empty());
}

#[test]
fn an_empty_tag_group_has_no_tag_to_have_so_any_in_it_matches_nothing_and_none_in_it_matches_every_track(
) {
    let w = world();
    let (group, _) = w.lib.tags("Empty", vec![]);
    let in_group = |op| Condition::TagGroup {
        op,
        group_id: group,
    };
    assert!(matched(&w.lib, one(in_group(TagOp::Has))).is_empty());
    assert_eq!(
        matched(&w.lib, one(in_group(TagOp::HasNot))),
        vec![w.rec[0], w.rec[1], w.rec[2], w.rec[3], w.rec[4]]
    );
}

#[test]
fn tag_conditions_combine_with_other_fields_as_all_or_any() {
    let w = world();
    let r = w.rec;
    let (_, tags) = w.lib.tags("Role", vec![("peak", vec![r[0], r[2], r[3]])]);
    let peak = tags[0];
    // Energy at least 7 (a, c, e) and tagged peak (a, c, d).
    let both = |combine| SmartRule {
        combine,
        conditions: vec![has(peak), energy(NumberOp::AtLeast, 7)],
    };
    assert_eq!(matched(&w.lib, both(Combine::All)), vec![r[0], r[2]]);
    assert_eq!(
        matched(&w.lib, both(Combine::Any)),
        vec![r[0], r[2], r[3], r[4]]
    );
    // Not tagged peak, with energy at most 8: b only (d is tagged, e is 10).
    let rule = SmartRule {
        combine: Combine::All,
        conditions: vec![has_not(peak), energy(NumberOp::AtMost, 8)],
    };
    assert_eq!(matched(&w.lib, rule), vec![r[1]]);
}

#[test]
fn a_rule_in_the_stored_json_form_reads_a_tag_condition_by_id() {
    let rule = SmartRule::from_json(
        r#"{"combine":"all","conditions":[{"field":"tag","op":"has","tagId":4},
            {"field":"tagGroup","op":"hasNot","groupId":2}]}"#,
    )
    .unwrap();
    assert_eq!(
        rule.conditions,
        vec![
            has(TagId(4)),
            Condition::TagGroup {
                op: TagOp::HasNot,
                group_id: TagGroupId(2)
            }
        ]
    );
}

#[test]
fn a_send_evaluates_a_tag_rule_with_the_always_and_never_lists() {
    let w = world();
    let r = w.rec;
    let lt = |i: usize| w.lt[i].unwrap();
    let (_, tags) = w.lib.tags("Role", vec![("peak", vec![r[0], r[1], r[5]])]);
    let peak = tags[0];
    let id = w.lib.create("Peak tag", one(has(peak)));
    // The outside track f has the tag and isn't a Library track.
    let tree = w.lib.with(|c| crate::send::crate_tree(c));
    assert_eq!(
        tree,
        vec![crate::rekordbox_write::Node::Playlist {
            name: "Peak tag".to_owned(),
            entries: vec![lt(0), lt(1)],
        }]
    );
    // Tagging another track puts it in at the next send; the lists still apply.
    let (always, never) = (lt(4), lt(0));
    w.lib.with(move |c| {
        include_on(c, id, &[always], Inclusion::Always)
            .unwrap()
            .unwrap();
        include_on(c, id, &[never], Inclusion::Never)
            .unwrap()
            .unwrap();
        crate::vocab::apply_on(c, peak, &[r[2]]).unwrap().unwrap();
        Ok(())
    });
    let tree = w.lib.with(|c| crate::send::crate_tree(c));
    assert_eq!(
        tree,
        vec![crate::rekordbox_write::Node::Playlist {
            name: "Peak tag".to_owned(),
            entries: vec![lt(1), lt(2), lt(4)],
        }]
    );
    assert_eq!(w.lib.with(move |c| count(c, id)), 3);
}

/// Text the user could type, with a marker that appears in no SQL here.
fn user_text() -> impl Strategy<Value = String> {
    ".{0,12}".prop_map(|s| format!("Zq{s}'\"; --"))
}

fn number_op() -> impl Strategy<Value = NumberOp> {
    prop_oneof![
        Just(NumberOp::Is),
        Just(NumberOp::IsNot),
        Just(NumberOp::AtLeast),
        Just(NumberOp::AtMost),
        Just(NumberOp::Between),
    ]
}

fn choice_op() -> impl Strategy<Value = ChoiceOp> {
    prop_oneof![Just(ChoiceOp::Is), Just(ChoiceOp::IsNot)]
}

fn tag_op() -> impl Strategy<Value = TagOp> {
    prop_oneof![Just(TagOp::Has), Just(TagOp::HasNot)]
}

fn any_condition() -> impl Strategy<Value = Condition> {
    let text_op = prop_oneof![
        Just(TextOp::Is),
        Just(TextOp::IsNot),
        Just(TextOp::Contains)
    ];
    prop_oneof![
        (number_op(), any::<u8>(), proptest::option::of(any::<u8>()))
            .prop_map(|(op, value, to)| Condition::Energy { op, value, to }),
        (
            number_op(),
            any::<u16>(),
            proptest::option::of(any::<u16>()),
            any::<bool>()
        )
            .prop_map(|(op, value, to, half_double)| Condition::Bpm {
                op,
                value,
                to,
                half_double
            }),
        (choice_op(), prop_oneof![user_text(), Just("8A".to_owned())])
            .prop_map(|(op, value)| Condition::Key { op, value }),
        (text_op, user_text()).prop_map(|(op, value)| Condition::Genre { op, value }),
        (number_op(), any::<u8>(), proptest::option::of(any::<u8>()))
            .prop_map(|(op, value, to)| Condition::Rating { op, value, to }),
        (choice_op(), any::<i64>()).prop_map(|(op, id)| Condition::InCrate {
            op,
            crate_id: CrateId(id)
        }),
        (tag_op(), any::<i64>()).prop_map(|(op, id)| Condition::Tag {
            op,
            tag_id: TagId(id)
        }),
        (tag_op(), any::<i64>()).prop_map(|(op, id)| Condition::TagGroup {
            op,
            group_id: TagGroupId(id)
        }),
    ]
}

fn any_rule() -> impl Strategy<Value = SmartRule> {
    (
        prop_oneof![Just(Combine::All), Just(Combine::Any)],
        proptest::collection::vec(any_condition(), 0..8),
    )
        .prop_map(|(combine, conditions)| SmartRule {
            combine,
            conditions,
        })
}

#[test]
fn any_rule_compiles_to_sql_that_runs_with_every_value_bound() {
    let w = world();
    let saved = w.lib.create(
        "Random",
        SmartRule {
            combine: Combine::All,
            conditions: vec![],
        },
    );
    let mut runner = TestRunner::new(Config {
        cases: 300,
        failure_persistence: None,
        ..Config::default()
    });
    runner
        .run(
            &(any_rule(), proptest::option::of(any::<i64>())),
            |(rule, id)| {
                // The JSON the crate stores reads back as the same rule.
                prop_assert_eq!(SmartRule::from_json(&rule.to_json()), Some(rule.clone()));
                let mut params = Vec::new();
                let sql = condition(id.map(CrateId), Some(&rule), &mut params);
                prop_assert!(!sql.contains("Zq"), "user text in the SQL: {}", sql);
                // Every value is a numbered parameter, each one used.
                let marks = (1..=params.len()).all(|n| sql.contains(&format!("?{n}")));
                prop_assert!(marks);
                let past = format!("?{}", params.len() + 1);
                prop_assert!(!sql.contains(&past));
                let alone = compile(&rule, &mut Vec::new());
                prop_assert!(!alone.contains("Zq"));
                let n = w.lib.with(move |c| {
                    c.query_row(
                        &format!("SELECT count(*) FROM track_row tr WHERE {sql}"),
                        rusqlite::params_from_iter(params.iter()),
                        |r| r.get::<_, i64>(0),
                    )
                });
                prop_assert!((0..=6).contains(&n));
                // Saved, the same rule is what a send, the list and the count read.
                let json = rule.to_json();
                let (held, counted) = w.lib.with(move |c| {
                    c.execute(
                        "UPDATE crate SET rules = ?1 WHERE id = ?2",
                        rusqlite::params![json, saved.0],
                    )?;
                    Ok((evaluate(c, saved)?, count(c, saved)?))
                });
                prop_assert_eq!(held.len(), counted as usize);
                prop_assert!(held.len() <= 5);
                Ok(())
            },
        )
        .unwrap();
    // Text that looks like SQL is only ever a value.
    let sly = one(Condition::Genre {
        op: TextOp::Is,
        value: "x' OR '1'='1".to_owned(),
    });
    assert!(matched(&w.lib, sly).is_empty());
    let mut params = Vec::new();
    compile(
        &one(Condition::Genre {
            op: TextOp::Is,
            value: "Deep House".to_owned(),
        }),
        &mut params,
    );
    assert_eq!(params, vec![Value::Text("deep house".to_owned())]);
}
