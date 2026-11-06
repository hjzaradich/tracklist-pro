//! Play history (3.6): reading m3u8 text, matching paths, and the writes
//! with their Undo. Made-up names and rows only, no audio.

use rusqlite::Connection;

use super::m3u8::{self, Entry};
use super::matching::absolute;
use super::*;
use crate::search::tests::support::{Lib, Mount};
use crate::settings::{write_feature_enabled, Feature};

fn run<T: Send + 'static>(
    lib: &Lib,
    job: impl FnOnce(&mut Connection) -> Result<T, OpsError> + Send + 'static,
) -> T {
    lib.writer.call(move |c| Ok(job(c))).unwrap().unwrap()
}

fn read<T: Send + 'static>(
    lib: &Lib,
    job: impl FnOnce(&Connection) -> rusqlite::Result<T> + Send + 'static,
) -> T {
    lib.writer.call(move |c| job(c)).unwrap()
}

fn mounted(c: &Connection) -> Mounted {
    Mounted::ask(&crate::relink::identities(c).unwrap(), &Mount)
}

fn import(lib: &Lib, file: &'static str, text: &'static str) -> HistoryOutcome {
    run(lib, move |c| {
        let m = mounted(c);
        import_m3u8(c, &m, file, text.as_bytes())
    })
}

fn done(outcome: &HistoryOutcome) -> (i64, u32, u32, u32) {
    match outcome {
        HistoryOutcome::Done {
            session_id,
            plays,
            matched,
            skipped_lines,
            operation_id,
        } => {
            assert!(operation_id.is_some());
            (*session_id, *plays, *matched, *skipped_lines)
        }
        other => panic!("not done: {other:?}"),
    }
}

fn undo(lib: &Lib) {
    let undone = run(lib, |c| ops::undo_for_user(c, None));
    assert!(
        matches!(undone, ops::UndoOutcome::Undone { .. }),
        "{undone:?}"
    );
}

fn operations(lib: &Lib) -> i64 {
    read(lib, |c| {
        c.query_row("SELECT count(*) FROM operation", [], |r| r.get(0))
    })
}

fn entry(path: &str, artist: Option<&str>, title: Option<&str>) -> Entry {
    Entry {
        path: path.to_owned(),
        artist: artist.map(str::to_owned),
        title: title.map(str::to_owned),
    }
}

// ---- reading m3u8 -------------------------------------------------------------

#[test]
fn an_m3u8_with_bom_crlf_header_and_extinf_gives_its_plays_in_order() {
    let text = "\u{FEFF}#EXTM3U\r\n#EXTINF:301,Artist One - Title One\r\nE:\\Music\\one.mp3\r\n\r\n#EXTINF:-1,Title Only\r\nsub/two.flac\r\nE:\\Music\\three.mp3\r\n";
    let parsed = m3u8::parse(text.as_bytes());
    assert_eq!(
        parsed.entries,
        vec![
            entry(r"E:\Music\one.mp3", Some("Artist One"), Some("Title One")),
            entry("sub/two.flac", None, Some("Title Only")),
            entry(r"E:\Music\three.mp3", None, None),
        ]
    );
    assert_eq!(parsed.skipped, 0);
}

#[test]
fn bad_lines_are_skipped_and_counted_and_the_rest_is_read() {
    let mut bytes = b"#EXTINF:abc,Broken - Line\nE:/Music/a.mp3\n".to_vec();
    bytes.extend_from_slice(b"\xFF\xFE not text\n");
    bytes.extend_from_slice(b"bad\x01path.mp3\n");
    bytes.extend_from_slice(b"#EXTVLCOPT:a comment\n");
    bytes.extend_from_slice(b"https://example.invalid/stream\n");
    bytes.extend_from_slice(b"file://localhost/E:/Music/b%20c.mp3\n");
    let parsed = m3u8::parse(&bytes);
    assert_eq!(
        parsed.entries,
        vec![
            entry("E:/Music/a.mp3", None, None),
            entry("E:/Music/b c.mp3", None, None),
        ]
    );
    // The broken #EXTINF, the line that isn't UTF-8, the control
    // character and the URL that names no file.
    assert_eq!(parsed.skipped, 4);
}

#[test]
fn nothing_but_garbage_gives_no_plays_and_never_panics() {
    for bytes in [
        &b""[..],
        b"\xEF\xBB\xBF",
        b"#EXTM3U\n",
        b"\n\n\r\n",
        b"#EXTINF:\n",
    ] {
        assert!(m3u8::parse(bytes).entries.is_empty());
    }
}

#[test]
fn a_date_in_the_file_name_is_found_only_when_it_is_a_real_day() {
    assert_eq!(m3u8::date_in("2026-05-03").as_deref(), Some("2026-05-03"));
    assert_eq!(
        m3u8::date_in("HISTORY 2024-02-29 (2)").as_deref(),
        Some("2024-02-29")
    );
    assert_eq!(m3u8::date_in("2023-02-29"), None);
    assert_eq!(m3u8::date_in("12026-05-03"), None);
    assert_eq!(m3u8::date_in("Friday set"), None);
    assert!(m3u8::real_day("2026-12-31"));
    assert!(!m3u8::real_day("2026-13-01"));
    assert!(!m3u8::real_day("2026-1-01"));
    assert!(!m3u8::real_day("2026-+1-01"));
}

#[test]
fn relative_paths_are_taken_from_the_playlist_folder() {
    let base = Some(r"E:\Exports");
    assert_eq!(absolute("a.mp3", base).as_deref(), Some("E:/Exports/a.mp3"));
    assert_eq!(
        absolute(r"..\Music\a.mp3", base).as_deref(),
        Some("E:/Music/a.mp3")
    );
    assert_eq!(
        absolute("./x/../a.mp3", base).as_deref(),
        Some("E:/Exports/a.mp3")
    );
    assert_eq!(absolute(r"F:\a.mp3", base).as_deref(), Some("F:/a.mp3"));
    assert_eq!(
        absolute(r"\\server\share\a.mp3", None).as_deref(),
        Some("//server/share/a.mp3")
    );
    assert_eq!(absolute("a.mp3", None), None);
    assert_eq!(absolute("../../../a.mp3", base), None);
}

// ---- importing -------------------------------------------------------------------

const SET: &str = "#EXTM3U\n\
#EXTINF:300,Artist A - Track A\n\
e:\\music\\a.mp3\n\
#EXTINF:300,Partner - Not Mine\n\
E:\\Elsewhere\\partner.mp3\n\
#EXTINF:300,Artist B - Track B\n\
..\\Music\\b.mp3\n\
#EXTINF:oops\n";

/// Two tracks in the library, `a.mp3` and `b.mp3` in `E:\Music`.
fn library() -> (Lib, i64, i64) {
    let lib = Lib::new();
    let (a, _) = lib.track(Some("Track A"), Some("Artist A"), "a.mp3");
    let (b, _) = lib.track(Some("Track B"), Some("Artist B"), "b.mp3");
    (lib, a, b)
}

#[test]
fn importing_a_file_makes_one_session_matched_by_path_with_unmatched_plays_kept() {
    let (lib, a, b) = library();
    let outcome = import(&lib, r"E:\Exports\2026-05-03.m3u8", SET);
    let (session, total, matched, skipped) = done(&outcome);
    assert_eq!((total, matched, skipped), (3, 2, 1));
    assert_eq!(operations(&lib), 1);

    let list = read(&lib, sessions);
    assert_eq!(list.len(), 1);
    assert_eq!(
        (list[0].name.as_str(), list[0].played_on.as_str()),
        ("2026-05-03", "2026-05-03")
    );
    assert_eq!(
        (list[0].source.as_str(), list[0].plays, list[0].matched),
        ("m3u8", 3, 2)
    );

    let reply = read(&lib, move |c| plays(c, session));
    let got: Vec<_> = reply
        .plays
        .iter()
        .map(|p| {
            (
                p.position,
                p.recording_id,
                p.artist.as_deref(),
                p.path.as_str(),
            )
        })
        .collect();
    assert_eq!(
        got,
        vec![
            (0, Some(a), Some("Artist A"), r"e:\music\a.mp3"),
            (1, None, Some("Partner"), r"E:\Elsewhere\partner.mp3"),
            (2, Some(b), Some("Artist B"), r"..\Music\b.mp3"),
        ]
    );
    assert!(!reply.wishlist_offer, "wishlists are off");
}

#[test]
fn undoing_an_import_takes_the_session_and_its_plays_away() {
    let (lib, _, _) = library();
    done(&import(&lib, r"E:\Exports\Friday.m3u8", SET));
    undo(&lib);
    let left: (i64, i64) = read(&lib, |c| {
        c.query_row(
            "SELECT (SELECT count(*) FROM session), (SELECT count(*) FROM play_event)",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
    });
    assert_eq!(left, (0, 0));
}

#[test]
fn a_file_without_a_date_is_dated_the_day_it_is_imported() {
    let (lib, _, _) = library();
    done(&import(&lib, r"E:\Exports\Friday.m3u8", SET));
    let today: String = read(&lib, |c| {
        c.query_row("SELECT date('now', 'localtime')", [], |r| r.get(0))
    });
    let list = read(&lib, sessions);
    assert_eq!(
        (list[0].name.as_str(), list[0].played_on.as_str()),
        ("Friday", today.as_str())
    );
}

#[test]
fn the_same_plays_in_the_same_order_are_refused_as_already_imported() {
    let (lib, _, _) = library();
    let (first, ..) = done(&import(&lib, r"E:\Exports\one.m3u8", SET));
    // Another name, another spelling of the paths: still the same set.
    let again = import(
        &lib,
        r"E:\Exports\two.m3u8",
        "E:/MUSIC/A.MP3\nE:/Elsewhere/PARTNER.mp3\nE:\\Music\\b.mp3\n",
    );
    assert_eq!(
        again,
        HistoryOutcome::Refused {
            reason: HistoryRefusal::AlreadyImported,
            session_id: Some(first),
            skipped_lines: 0,
        }
    );
    // Another order is another set.
    done(&import(
        &lib,
        r"E:\Exports\three.m3u8",
        "E:/Music/b.mp3\nE:/Music/a.mp3\nE:/Elsewhere/partner.mp3\n",
    ));
    assert_eq!(operations(&lib), 2);
}

#[test]
fn bad_data_never_stops_an_import_and_a_file_of_no_plays_is_refused() {
    let (lib, _, _) = library();
    let garbage = import(&lib, r"E:\x.m3u8", "#EXTM3U\n#EXTINF:zz\n\u{1}\n");
    assert_eq!(
        garbage,
        HistoryOutcome::Refused {
            reason: HistoryRefusal::NoPlays,
            session_id: None,
            skipped_lines: 2,
        }
    );
    let big = vec![b'a'; m3u8::MAX_BYTES + 1];
    let outcome = run(&lib, move |c| {
        let m = mounted(c);
        import_m3u8(c, &m, "big.m3u8", &big)
    });
    assert_eq!(outcome, refused(HistoryRefusal::FileTooBig));
    // A relative path with no folder to take it from is kept, unmatched.
    let outcome = run(&lib, |c| {
        let m = mounted(c);
        import_m3u8(c, &m, "", b"a.mp3\n")
    });
    let (_, total, matched, _) = done(&outcome);
    assert_eq!((total, matched), (1, 0));
    assert_eq!(operations(&lib), 1);
}

#[test]
fn correcting_a_date_is_one_operation_and_undo_puts_the_old_one_back() {
    let (lib, _, _) = library();
    let (session, ..) = done(&import(&lib, r"E:\Exports\2026-05-03.m3u8", SET));
    assert_eq!(
        run(&lib, move |c| set_date(c, session, "2026-02-30")),
        refused(HistoryRefusal::BadDate)
    );
    assert_eq!(
        run(&lib, |c| set_date(c, 999, "2026-05-04")),
        refused(HistoryRefusal::NoSuchSession)
    );
    done(&run(&lib, move |c| set_date(c, session, " 2026-05-04 ")));
    assert_eq!(read(&lib, sessions)[0].played_on, "2026-05-04");
    assert_eq!(operations(&lib), 2);
    // The same date again changes nothing and makes no operation.
    let same = run(&lib, move |c| set_date(c, session, "2026-05-04"));
    assert!(matches!(
        same,
        HistoryOutcome::Done {
            operation_id: None,
            ..
        }
    ));
    undo(&lib);
    assert_eq!(read(&lib, sessions)[0].played_on, "2026-05-03");
}

// ---- what other features read ----------------------------------------------------

#[test]
fn transitions_count_each_track_played_right_before_another() {
    let (lib, a, b) = library();
    let (c_track, _) = lib.track(Some("Track C"), Some("Artist C"), "c.mp3");
    done(&import(
        &lib,
        r"E:\1.m3u8",
        "E:/Music/a.mp3\nE:/Music/b.mp3\nE:/Music/c.mp3\n",
    ));
    // An unmatched play between b and c breaks that pair; a repeat isn't one.
    done(&import(
        &lib,
        r"E:\2.m3u8",
        "E:/Music/a.mp3\nE:/Music/b.mp3\nE:/Gone/x.mp3\nE:/Music/c.mp3\nE:/Music/c.mp3\n",
    ));
    let all = read(&lib, |c| transitions(c, None));
    assert_eq!(
        all,
        vec![
            Transition {
                from: a,
                to: b,
                count: 2
            },
            Transition {
                from: b,
                to: c_track,
                count: 1
            },
        ]
    );
    let from_b = read(&lib, move |c| transitions(c, Some(b)));
    assert_eq!(
        from_b,
        vec![Transition {
            from: b,
            to: c_track,
            count: 1
        }]
    );
}

#[test]
fn importing_history_leaves_rekordbox_play_counts_and_last_played_as_they_are() {
    let (lib, _, _) = library();
    let file: i64 = read(&lib, |c| {
        c.query_row("SELECT id FROM file WHERE rel_path = 'a.mp3'", [], |r| {
            r.get(0)
        })
    });
    lib.rekordbox_entry(file, &[("PlayCount", "7"), ("LastPlayed", "2026-01-02")]);
    let snapshot = |lib: &Lib| -> Vec<(Option<i64>, Option<String>, String)> {
        read(lib, |c| {
            c.prepare(
                "SELECT play_count, json_extract(attributes, '$.LastPlayed'), attributes
                 FROM rekordbox_track ORDER BY id",
            )?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect()
        })
    };
    let before = snapshot(&lib);
    assert_eq!(
        (before[0].0, before[0].1.as_deref()),
        (Some(7), Some("2026-01-02"))
    );
    done(&import(&lib, r"E:\2026-05-03.m3u8", SET));
    assert_eq!(snapshot(&lib), before);
}

// ---- wishlist ---------------------------------------------------------------------

#[test]
fn unmatched_plays_go_to_a_wishlist_in_one_undoable_step_only_while_wishlists_are_on() {
    let (lib, _, _) = library();
    let text = "#EXTINF:1,Partner - Not Mine\nE:/x/1.mp3\n#EXTINF:1,No Artist\nE:/x/2.mp3\nE:/Music/a.mp3\n";
    let (session, ..) = done(&import(&lib, r"E:\set.m3u8", text));
    let list: i64 = read(&lib, |c| {
        c.execute(
            "INSERT INTO wishlist (name, source) VALUES ('Wants', 'paste')",
            [],
        )?;
        Ok(c.last_insert_rowid())
    });

    let off = run(&lib, move |c| add_unmatched_to_wishlist(c, session, list));
    assert!(matches!(off, WishlistOutcome::Refused { .. }), "{off:?}");

    lib.writer
        .call(|c| write_feature_enabled(c, Feature::Wishlist, true))
        .unwrap();
    assert!(read(&lib, move |c| plays(c, session)).wishlist_offer);
    let before = operations(&lib);
    let added = run(&lib, move |c| add_unmatched_to_wishlist(c, session, list));
    match &added {
        WishlistOutcome::Done {
            added,
            skipped,
            skipped_total,
            ..
        } => {
            assert_eq!((*added, *skipped_total), (1, 1));
            assert_eq!(
                (skipped[0].line, skipped[0].reason),
                (2, WishSkipReason::MissingArtist)
            );
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(operations(&lib), before + 1);
    let items: Vec<(String, String)> = read(&lib, |c| {
        c.prepare("SELECT raw_artist, raw_title FROM wish_item")?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect()
    });
    assert_eq!(items, vec![("Partner".to_owned(), "Not Mine".to_owned())]);
    undo(&lib);
    let left: i64 = read(&lib, |c| {
        c.query_row("SELECT count(*) FROM wish_item", [], |r| r.get(0))
    });
    assert_eq!(left, 0);
}
