//! Timeline, target shape and "fill the gap" (3.5), over a migrated database
//! in a temp dir. Everything is synthetic.

use super::tests::Lib;
use super::timeline::*;
use super::*;
use crate::next::bridge::BridgeReason;
use crate::next::Tolerance;
use crate::search::flag;

/// A volume identity that reads back (the playlist tests' own is a stand-in).
fn volume_identity() -> String {
    crate::volume::identity(crate::volume::IdentitySignals {
        kind: crate::volume::VolumeKind::External,
        unc_share: None,
        serial: Some(0x1A2B_3C4D),
        filesystem: "NTFS",
        guid: None,
    })
    .unwrap()
    .as_str()
    .to_owned()
}

struct Made {
    entry_track: LibraryTrackId,
    recording: i64,
}

impl Lib {
    /// A Library track with the facts given, in the table's sources (BPM and
    /// key from rekordbox, energy from the user) and in the index "fill the
    /// gap" reads its candidates from.
    fn facts(
        &self,
        name: &'static str,
        bpm: Option<f64>,
        key: Option<&'static str>,
        energy: Option<u8>,
        ms: Option<i64>,
    ) -> Made {
        let (file, track) = self.track(name, true);
        let recording: i64 = self
            .writer
            .call(move |c| {
                c.query_row(
                    "SELECT recording_id FROM library_track WHERE id = ?1",
                    [track.0],
                    |r| r.get(0),
                )
            })
            .unwrap();
        self.writer
            .call(move |c| {
                if bpm.is_some() || key.is_some() {
                    c.execute(
                        "INSERT INTO analysis (recording_id, source, bpm, key)
                         VALUES (?1, 'rekordbox', ?2, ?3)",
                        (recording, bpm, key),
                    )?;
                }
                if let Some(energy) = energy {
                    c.execute(
                        "INSERT INTO analysis (recording_id, source, energy)
                         VALUES (?1, 'user', ?2)",
                        (recording, energy),
                    )?;
                }
                c.execute("UPDATE file SET duration_ms = ?2 WHERE id = ?1", (file, ms))?;
                // The table reads a length from a linked file on a volume whose
                // identity reads back.
                c.execute("UPDATE volume SET identity = ?1", [volume_identity()])?;
                // A linked file is where the table reads the length from.
                c.execute(
                    "UPDATE library_track SET kind = 'linked', linked_file_id = ?2, source_status = 'ok'
                     WHERE id = ?1",
                    (track.0, file),
                )?;
                c.execute(
                    "INSERT INTO track_row (recording_id, flags, title_key, bpm, key_order)
                     VALUES (?1, ?2, 'x', ?3, ?4)",
                    (
                        recording,
                        flag::IN_LIBRARY | flag::energy(energy.unwrap_or(0)),
                        bpm,
                        key.and_then(crate::search::camelot_order),
                    ),
                )
            })
            .unwrap();
        Made {
            entry_track: track,
            recording,
        }
    }

    fn timeline(&self, id: PlaylistId) -> Timeline {
        self.writer
            .call(move |c| timeline(c, &crate::scan::system_volumes(), id))
            .unwrap()
            .unwrap()
    }

    fn set_target(&self, id: PlaylistId, points: &[(f64, u8)]) -> Option<i64> {
        let points: Vec<TargetPoint> = points
            .iter()
            .map(|&(at, energy)| TargetPoint {
                at,
                energy: f64::from(energy),
            })
            .collect();
        self.call(move |c| set_target_on(c, id, &points)).unwrap()
    }

    fn fill(
        &self,
        id: PlaylistId,
        after: Option<PlaylistEntryId>,
        include_playlist_tracks: bool,
    ) -> FillGapReply {
        self.writer
            .call(move |c| {
                fill_gap_on(
                    c,
                    &crate::scan::system_volumes(),
                    id,
                    after,
                    10,
                    include_playlist_tracks,
                )
            })
            .unwrap()
            .unwrap()
    }
}

fn ids(lib: &Lib, id: PlaylistId) -> Vec<PlaylistEntryId> {
    lib.order(id).into_iter().map(|(e, _)| e).collect()
}

// ---- the timeline --------------------------------------------------------------

#[test]
fn the_timeline_lists_entries_in_playlist_order_repeats_kept_with_their_facts() {
    let lib = Lib::new();
    let a = lib.facts("a", Some(124.0), Some("8A"), Some(5), Some(180_000));
    let b = lib.facts("b", Some(126.0), Some("9A"), Some(6), Some(200_000));
    let set = lib.create("Set");
    lib.add(set, &[b.entry_track, a.entry_track, b.entry_track], None);
    let t = lib.timeline(set);
    assert_eq!(t.playlist.track_count, 3);
    let seen: Vec<(u32, i64, Option<&str>, Option<u8>)> = t
        .entries
        .iter()
        .map(|e| (e.position, e.recording_id, e.key.as_deref(), e.energy))
        .collect();
    assert_eq!(
        seen,
        [
            (0, b.recording, Some("9A"), Some(6)),
            (1, a.recording, Some("8A"), Some(5)),
            (2, b.recording, Some("9A"), Some(6)),
        ]
    );
    assert_eq!(t.entries[1].bpm, Some(124.0));
    // Two entries for the same track have two entry ids.
    assert_ne!(t.entries[0].entry_id, t.entries[2].entry_id);
    assert_eq!(t.transitions.len(), 2);
    assert_eq!(t.transitions[0].from_entry, t.entries[0].entry_id);
    assert_eq!(t.transitions[0].to_entry, t.entries[1].entry_id);
}

#[test]
fn start_times_add_up_the_durations_and_a_track_without_one_counts_as_unknown() {
    let lib = Lib::new();
    let a = lib.facts("a", None, None, None, Some(180_000));
    let b = lib.facts("b", None, None, None, None);
    let c = lib.facts("c", None, None, None, Some(240_000));
    let set = lib.create("Set");
    lib.add(set, &[a.entry_track, b.entry_track, c.entry_track], None);
    let t = lib.timeline(set);
    let starts: Vec<i64> = t.entries.iter().map(|e| e.start_ms).collect();
    assert_eq!(starts, [0, 180_000, 180_000]);
    assert_eq!(t.total_ms, 420_000);
    assert_eq!(t.unknown_durations, 1);
    assert_eq!(t.entries[1].duration_ms, None);
}

#[test]
fn transitions_flag_key_clashes_bpm_jumps_and_energy_jumps_and_an_unknown_is_never_a_clash() {
    let lib = Lib::new();
    let fine = lib.facts("fine", Some(124.0), Some("8A"), Some(5), None);
    let next = lib.facts("next", Some(126.0), Some("9A"), Some(6), None);
    let clash = lib.facts("clash", Some(150.0), Some("3B"), Some(9), None);
    let blank = lib.facts("blank", None, None, None, None);
    let half = lib.facts("half", Some(75.0), Some("3B"), Some(9), None);
    let set = lib.create("Set");
    lib.add(
        set,
        &[
            fine.entry_track,
            next.entry_track,
            clash.entry_track,
            blank.entry_track,
            half.entry_track,
        ],
        None,
    );
    let t = lib.timeline(set);
    assert_eq!(t.tolerance, Tolerance::Tight);
    let flags: Vec<(Flow, Flow, Flow)> = t
        .transitions
        .iter()
        .map(|x| (x.key, x.bpm, x.energy))
        .collect();
    assert_eq!(
        flags,
        [
            (Flow::Fine, Flow::Fine, Flow::Fine),
            (Flow::Clash, Flow::Clash, Flow::Clash),
            (Flow::Unknown, Flow::Unknown, Flow::Unknown),
            (Flow::Unknown, Flow::Unknown, Flow::Unknown),
        ]
    );
    assert_eq!(t.transitions[0].energy_step, Some(1));
    assert_eq!(t.transitions[1].energy_step, Some(3));
    assert_eq!(t.transitions[2].energy_step, None);
}

#[test]
fn a_tempo_at_half_or_double_time_is_not_a_bpm_jump() {
    let lib = Lib::new();
    let a = lib.facts("a", Some(150.0), Some("8A"), None, None);
    let b = lib.facts("b", Some(75.0), Some("8A"), None, None);
    let set = lib.create("Set");
    lib.add(set, &[a.entry_track, b.entry_track], None);
    let t = lib.timeline(set);
    assert_eq!(t.transitions[0].bpm, Flow::Fine);
    assert_eq!(t.transitions[0].bpm_delta, Some(0.0));
}

#[test]
fn the_wide_setting_widens_what_a_clash_is() {
    let lib = Lib::new();
    let a = lib.facts("a", Some(120.0), Some("8A"), None, None);
    let b = lib.facts("b", Some(132.0), Some("10A"), None, None);
    let set = lib.create("Set");
    lib.add(set, &[a.entry_track, b.entry_track], None);
    let tight = lib.timeline(set);
    assert_eq!(
        (tight.transitions[0].key, tight.transitions[0].bpm),
        (Flow::Clash, Flow::Clash)
    );
    lib.call(|c| {
        crate::next::write_settings(
            c,
            crate::next::Settings {
                tolerance: Tolerance::Wide,
                ..Default::default()
            },
        )?;
        Ok(())
    });
    let wide = lib.timeline(set);
    assert_eq!(wide.tolerance, Tolerance::Wide);
    assert_eq!(
        (wide.transitions[0].key, wide.transitions[0].bpm),
        (Flow::Fine, Flow::Fine)
    );
}

#[test]
fn an_estimated_energy_is_never_shown_only_the_users_or_a_tag_s() {
    let lib = Lib::new();
    let a = lib.facts("a", Some(124.0), Some("8A"), None, None);
    let b = lib.facts("b", Some(124.0), Some("8A"), Some(4), None);
    let rec = a.recording;
    lib.run(
        "INSERT INTO analysis (recording_id, source, energy) VALUES (?1, 'local', 8)",
        [rec],
    );
    let set = lib.create("Set");
    lib.add(set, &[a.entry_track, b.entry_track], None);
    let t = lib.timeline(set);
    assert_eq!(t.entries[0].energy, None);
    assert_eq!(t.entries[1].energy, Some(4));
    assert_eq!(t.transitions[0].energy, Flow::Unknown);
}

#[test]
fn a_key_that_is_the_apps_estimate_is_marked_and_still_counts_for_the_flags() {
    let lib = Lib::new();
    let own = lib.facts("own", Some(124.0), Some("8A"), None, None);
    // Only the app's own estimate of its key (the "local" source).
    let guessed = lib.facts("guessed", Some(124.0), None, None, None);
    let clash = lib.facts("clash", Some(124.0), Some("3B"), None, None);
    lib.run(
        "INSERT INTO analysis (recording_id, source, key) VALUES (?1, 'local', '9A')",
        [guessed.recording],
    );
    let set = lib.create("Set");
    lib.add(
        set,
        &[own.entry_track, guessed.entry_track, clash.entry_track],
        None,
    );
    let t = lib.timeline(set);
    let shown: Vec<(Option<&str>, bool)> = t
        .entries
        .iter()
        .map(|e| (e.key.as_deref(), e.key_estimated))
        .collect();
    assert_eq!(
        shown,
        [(Some("8A"), false), (Some("9A"), true), (Some("3B"), false)]
    );
    // The estimate is used as the table uses it: 8A to 9A is a neighbour, and
    // 9A to 3B is a clash.
    assert_eq!(t.transitions[0].key, Flow::Fine);
    assert_eq!(t.transitions[1].key, Flow::Clash);
}

#[test]
fn a_playlist_that_does_not_exist_has_no_timeline() {
    let lib = Lib::new();
    let found = lib
        .writer
        .call(|c| timeline(c, &crate::scan::system_volumes(), PlaylistId(99)))
        .unwrap();
    assert!(found.is_none());
}

// ---- the target shape ----------------------------------------------------------

#[test]
fn the_target_is_a_straight_line_between_points_and_flat_beyond_the_ends() {
    let shape = tidy_target(&[
        TargetPoint {
            at: 0.2,
            energy: 3.0,
        },
        TargetPoint {
            at: 0.8,
            energy: 9.0,
        },
    ]);
    assert_eq!(target_at(&[], 0.5), None);
    assert_eq!(target_at(&shape, 0.0), Some(3.0));
    assert_eq!(target_at(&shape, 0.2), Some(3.0));
    assert!((target_at(&shape, 0.5).unwrap() - 6.0).abs() < 1e-9);
    assert_eq!(target_at(&shape, 0.8), Some(9.0));
    assert_eq!(target_at(&shape, 1.0), Some(9.0));
    let one = tidy_target(&[TargetPoint {
        at: 0.5,
        energy: 7.0,
    }]);
    assert_eq!(target_at(&one, 0.0), Some(7.0));
}

#[test]
fn a_target_shape_is_tidied_not_refused() {
    // A drawn point far outside the scale (or below zero) still decodes, as
    // it crosses IPC, and is pulled in.
    let wild: Vec<TargetPoint> =
        serde_json::from_str(r#"[{"at":0.5,"energy":-4},{"at":2,"energy":999}]"#).unwrap();
    assert_eq!(
        tidy_target(&wild),
        [
            TargetPoint {
                at: 0.5,
                energy: 1.0
            },
            TargetPoint {
                at: 1.0,
                energy: 10.0
            },
        ]
    );
    let tidy = tidy_target(&[
        TargetPoint {
            at: 0.9,
            energy: 300.0,
        },
        TargetPoint {
            at: 0.7,
            energy: f64::NAN,
        },
        TargetPoint {
            at: -2.0,
            energy: 0.0,
        },
        TargetPoint {
            at: f64::NAN,
            energy: 5.0,
        },
        TargetPoint {
            at: 0.5,
            energy: 4.0,
        },
        TargetPoint {
            at: 0.5,
            energy: 6.0,
        },
    ]);
    assert_eq!(
        tidy,
        [
            TargetPoint {
                at: 0.0,
                energy: 1.0
            },
            TargetPoint {
                at: 0.5,
                energy: 6.0
            },
            TargetPoint {
                at: 0.9,
                energy: 10.0
            },
        ]
    );
    let many: Vec<TargetPoint> = (0..100)
        .map(|i| TargetPoint {
            at: f64::from(i) / 100.0,
            energy: 5.0,
        })
        .collect();
    assert_eq!(tidy_target(&many).len(), MOST_TARGET_POINTS);
}

#[test]
fn the_timeline_gives_the_target_at_each_entry_and_the_gap_to_it() {
    let lib = Lib::new();
    let a = lib.facts("a", None, None, Some(3), None);
    let b = lib.facts("b", None, None, Some(8), None);
    let c = lib.facts("c", None, None, None, None);
    let set = lib.create("Set");
    lib.add(set, &[a.entry_track, b.entry_track, c.entry_track], None);
    assert!(lib.timeline(set).target.is_empty());
    assert_eq!(lib.timeline(set).entries[0].target, None);
    lib.set_target(set, &[(0.0, 2), (1.0, 10)]);
    let t = lib.timeline(set);
    let target: Vec<Option<f64>> = t.entries.iter().map(|e| e.target).collect();
    assert_eq!(target, [Some(2.0), Some(6.0), Some(10.0)]);
    let gap: Vec<Option<f64>> = t.entries.iter().map(|e| e.gap).collect();
    assert_eq!(gap, [Some(1.0), Some(2.0), None]);
    assert_eq!(t.target.len(), 2);
}

#[test]
fn setting_and_clearing_the_target_are_one_named_undo_each() {
    let lib = Lib::new();
    let set = lib.create("Set");
    assert_eq!(lib.call(move |c| clear_target_on(c, set)).unwrap(), None);

    let before = lib.state();
    assert!(lib.set_target(set, &[(0.0, 3), (1.0, 8)]).is_some());
    let first = lib.timeline(set).target;
    assert_eq!(first.len(), 2);
    // The same shape again changes nothing, so records nothing.
    assert_eq!(lib.set_target(set, &[(0.0, 3), (1.0, 8)]), None);

    assert!(lib.set_target(set, &[(0.5, 6)]).is_some());
    assert_eq!(lib.timeline(set).target.len(), 1);
    assert_eq!(lib.undo(), SET_TARGET_OPERATION);
    assert_eq!(lib.timeline(set).target, first);

    assert!(lib
        .call(move |c| clear_target_on(c, set))
        .unwrap()
        .is_some());
    assert!(lib.timeline(set).target.is_empty());
    assert_eq!(lib.undo(), CLEAR_TARGET_OPERATION);
    assert_eq!(lib.timeline(set).target, first);

    assert_eq!(lib.undo(), SET_TARGET_OPERATION);
    assert_eq!(lib.state(), before);
}

#[test]
fn a_shape_with_no_usable_point_clears_the_target() {
    let lib = Lib::new();
    let set = lib.create("Set");
    lib.set_target(set, &[(0.5, 5)]);
    assert!(lib.set_target(set, &[(f64::NAN, 5)]).is_some());
    assert!(lib.timeline(set).target.is_empty());
    assert_eq!(lib.undo(), CLEAR_TARGET_OPERATION);
}

#[test]
fn the_target_of_a_playlist_that_does_not_exist_is_refused() {
    let lib = Lib::new();
    let refused = lib
        .writer
        .call(|c| Ok(set_target_on(c, PlaylistId(7), &[])))
        .unwrap()
        .unwrap();
    assert!(matches!(refused, Err(PlaylistsError::NotFound)));
}

// ---- fill the gap --------------------------------------------------------------

#[test]
fn fill_gap_offers_a_true_key_bridge_first_with_the_gap_it_fills() {
    let lib = Lib::new();
    let a = lib.facts("a", Some(124.0), Some("8A"), Some(5), None);
    let b = lib.facts("b", Some(125.0), Some("10A"), Some(7), None);
    let bridge = lib.facts("bridge", Some(124.0), Some("9A"), Some(6), None);
    let far = lib.facts("far", Some(124.0), Some("3B"), Some(6), None);
    let set = lib.create("Set");
    lib.add(set, &[a.entry_track, b.entry_track], None);
    let first = ids(&lib, set)[0];
    let reply = lib.fill(set, Some(first), false);
    let found: Vec<i64> = reply
        .suggestions
        .iter()
        .map(|s| s.row.recording_id)
        .collect();
    assert_eq!(found[0], bridge.recording);
    // The key-clashing track still fits on tempo and energy, so it comes after.
    assert_eq!(found, [bridge.recording, far.recording]);
    let reasons = &reply.suggestions[0].reasons;
    assert!(reasons.contains(&BridgeReason::KeyBridge {
        from: "8A".into(),
        via: "9A".into(),
        to: "10A".into()
    }));
    assert!(reasons.contains(&BridgeReason::EnergyStep {
        from: Some(5),
        via: 6,
        to: Some(7)
    }));
    assert!(reply.suggestions.iter().all(|s| !s.reasons.is_empty()));
    assert_eq!(reply.target, None);
}

#[test]
fn fill_gap_leaves_out_the_playlists_own_tracks_unless_asked() {
    let lib = Lib::new();
    let a = lib.facts("a", Some(124.0), Some("8A"), Some(5), None);
    let b = lib.facts("b", Some(125.0), Some("10A"), Some(7), None);
    let kept = lib.facts("kept", Some(124.0), Some("9A"), Some(6), None);
    let set = lib.create("Set");
    lib.add(set, &[a.entry_track, b.entry_track, kept.entry_track], None);
    let first = ids(&lib, set)[0];
    let without = lib.fill(set, Some(first), false);
    assert!(without
        .suggestions
        .iter()
        .all(|s| s.row.recording_id != kept.recording));
    let with = lib.fill(set, Some(first), true);
    assert!(with
        .suggestions
        .iter()
        .any(|s| s.row.recording_id == kept.recording));
    // The neighbors are never offered for their own gap.
    assert!(with
        .suggestions
        .iter()
        .all(|s| s.row.recording_id != a.recording && s.row.recording_id != b.recording));
}

#[test]
fn fill_gap_aims_at_the_target_energy_for_that_place() {
    let lib = Lib::new();
    let a = lib.facts("a", None, Some("8A"), Some(4), None);
    let b = lib.facts("b", None, Some("8A"), Some(4), None);
    let low = lib.facts("low", None, Some("8A"), Some(4), None);
    let high = lib.facts("high", None, Some("8A"), Some(9), None);
    let set = lib.create("Set");
    lib.add(set, &[a.entry_track, b.entry_track], None);
    let first = ids(&lib, set)[0];
    lib.call(|c| {
        crate::next::write_settings(
            c,
            crate::next::Settings {
                weights: crate::next::Weights {
                    key: 0,
                    bpm: 0,
                    energy: 5,
                    played: 0,
                    genre: 0,
                    tags: 0,
                },
                tolerance: Tolerance::Wide,
            },
        )?;
        Ok(())
    });
    let flat = lib.fill(set, Some(first), false);
    assert_eq!(flat.suggestions[0].row.recording_id, low.recording);
    lib.set_target(set, &[(0.0, 9), (1.0, 9)]);
    let aimed = lib.fill(set, Some(first), false);
    assert_eq!(aimed.target, Some(9.0));
    assert_eq!(aimed.suggestions[0].row.recording_id, high.recording);
    assert_eq!(
        aimed.suggestions[0].reasons,
        [BridgeReason::EnergyTarget {
            via: 9,
            target: 9.0
        }]
    );
}

#[test]
fn fill_gap_at_the_end_and_the_start_weighs_the_one_side_there_is() {
    let lib = Lib::new();
    let a = lib.facts("a", Some(124.0), Some("8A"), None, None);
    let b = lib.facts("b", Some(124.0), Some("10A"), None, None);
    let x = lib.facts("x", Some(124.0), Some("9A"), None, None);
    let set = lib.create("Set");
    lib.add(set, &[a.entry_track, b.entry_track], None);
    let entries = ids(&lib, set);
    let end = lib.fill(set, Some(entries[1]), false);
    assert_eq!(end.suggestions[0].row.recording_id, x.recording);
    assert!(matches!(
        end.suggestions[0].reasons[0],
        BridgeReason::KeyFits { to: None, .. }
    ));
    let start = lib.fill(set, None, false);
    assert!(matches!(
        start.suggestions[0].reasons[0],
        BridgeReason::KeyFits { from: None, .. }
    ));
}

#[test]
fn fill_gap_refuses_an_entry_that_is_not_in_the_playlist() {
    let lib = Lib::new();
    let set = lib.create("Set");
    let refused = lib
        .writer
        .call(move |c| {
            fill_gap_on(
                c,
                &crate::scan::system_volumes(),
                set,
                Some(PlaylistEntryId(999)),
                5,
                false,
            )
        })
        .unwrap();
    assert!(matches!(refused, Err(PlaylistsError::EntryNotFound)));
}

#[test]
fn twenty_thousand_library_tracks_are_weighed_and_only_those_that_fit_the_gap_are_offered() {
    let lib = Lib::new();
    let a = lib.facts("a", Some(124.0), Some("8A"), Some(5), None);
    let b = lib.facts("b", Some(125.0), Some("10A"), Some(7), None);
    let bridge = lib.facts("bridge", Some(124.0), Some("9A"), Some(6), None);
    // Library tracks the index knows (no table row needed: none fits, so
    // none is shown), nowhere near the gap in key, tempo or energy.
    lib.run(
        "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 20000)
         INSERT INTO track_row (recording_id, flags, title_key, bpm, key_order)
         SELECT 1000000 + i, 1 | (10 << 8), 'x', 200, 7 FROM n",
        (),
    );
    let set = lib.create("Set");
    lib.add(set, &[a.entry_track, b.entry_track], None);
    let first = ids(&lib, set)[0];
    let reply = lib.fill(set, Some(first), false);
    assert_eq!(reply.considered, 20_000 + 3);
    assert_eq!(reply.suggestions.len(), 1);
    assert_eq!(reply.suggestions[0].row.recording_id, bridge.recording);
}
