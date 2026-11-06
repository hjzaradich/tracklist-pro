//! Wishlists (ROADMAP 2.3): reading pasted text and CSV, matching against a
//! small library of synthetic tracks, and the writes with their Undo. Rows
//! and made-up names only, no audio.

use serde_json::json;

use super::matching::{isrcs_in, keys};
use super::parse::{self, CsvProblem, WishSkipReason};
use super::*;
use crate::search::tests::support::Lib;
use crate::settings::{write_feature_enabled, Feature};

/// A library with wishlists turned on, as the tests below need it.
fn lib_on() -> Lib {
    let lib = Lib::new();
    lib.writer
        .call(|c| write_feature_enabled(c, Feature::Wishlist, true))
        .unwrap();
    lib
}

// ---- paste ----------------------------------------------------------------

type Line = (&'static str, &'static str, Option<&'static str>);

fn pasted(text: &str) -> Vec<(String, String, Option<String>)> {
    parse::paste(text)
        .tracks
        .into_iter()
        .map(|t| (t.artist, t.title, t.version))
        .collect()
}

fn expect(lines: &[Line]) -> Vec<(String, String, Option<String>)> {
    lines
        .iter()
        .map(|(a, t, v)| ((*a).to_owned(), (*t).to_owned(), v.map(str::to_owned)))
        .collect()
}

#[test]
fn a_pasted_line_is_an_artist_and_a_title_split_at_a_spaced_dash() {
    for line in [
        "Quill Ashby - Paper Harbor",
        "Quill Ashby \u{2013} Paper Harbor",
        "Quill Ashby \u{2014} Paper Harbor",
        "  Quill   Ashby  -  Paper Harbor  ",
    ] {
        assert_eq!(
            pasted(line),
            expect(&[("Quill Ashby", "Paper Harbor", None)]),
            "{line}"
        );
    }
    // Only the first dash splits: the rest belongs to the title.
    assert_eq!(
        pasted("Nova Reed - Lantern - Dub"),
        expect(&[("Nova Reed", "Lantern - Dub", None)])
    );
}

#[test]
fn a_last_bracket_is_the_version_unless_it_is_a_featured_artist() {
    assert_eq!(
        pasted(
            "Quill Ashby - Paper Harbor (Extended Mix)\n\
             Quill Ashby - Paper Harbor [Club Mix]\n\
             Quill Ashby - Paper Harbor (Mix (Dub))\n\
             Quill Ashby - Paper Harbor (feat. Nova Reed)\n\
             Quill Ashby - (Extended Mix)"
        ),
        expect(&[
            ("Quill Ashby", "Paper Harbor", Some("Extended Mix")),
            ("Quill Ashby", "Paper Harbor", Some("Club Mix")),
            ("Quill Ashby", "Paper Harbor", Some("Mix (Dub)")),
            ("Quill Ashby", "Paper Harbor (feat. Nova Reed)", None),
            ("Quill Ashby", "(Extended Mix)", None),
        ])
    );
}

#[test]
fn numbers_and_times_in_front_of_a_line_are_set_aside() {
    let wanted = expect(&[("Quill Ashby", "Paper Harbor", None)]);
    for line in [
        "1. Quill Ashby - Paper Harbor",
        "12) Quill Ashby - Paper Harbor",
        "00:00 Quill Ashby - Paper Harbor",
        "3:45 - Quill Ashby - Paper Harbor",
        "[01:02:03] Quill Ashby - Paper Harbor",
        "(12:34) Quill Ashby \u{2013} Paper Harbor",
        "7. 00:00 Quill Ashby - Paper Harbor",
    ] {
        assert_eq!(pasted(line), wanted, "{line}");
    }
    // A number that is part of the name stays.
    assert_eq!(
        pasted("808 Bloom - Candle"),
        expect(&[("808 Bloom", "Candle", None)])
    );
}

#[test]
fn lines_that_cant_be_read_are_reported_with_their_place_and_blank_lines_are_not() {
    let parsed = parse::paste(
        "\u{feff}Quill Ashby - Paper Harbor\n\
         \n\
         just some words\n\
         Nova Reed - Lantern\r\n\
         - Orphan Title\n\
         Dangling Artist -",
    );
    assert_eq!(parsed.tracks.len(), 2);
    let skipped: Vec<(u32, &str, WishSkipReason)> = parsed
        .skipped
        .iter()
        .map(|s| (s.line, s.text.as_str(), s.reason))
        .collect();
    assert_eq!(
        skipped,
        [
            (3, "just some words", WishSkipReason::NoSeparator),
            (5, "- Orphan Title", WishSkipReason::NoSeparator),
            (6, "Dangling Artist -", WishSkipReason::NoSeparator),
        ]
    );
}

// ---- CSV ------------------------------------------------------------------

fn csv_tracks(text: &str) -> Vec<(String, String, Option<String>, Option<String>)> {
    parse::csv(text)
        .unwrap()
        .tracks
        .into_iter()
        .map(|t| (t.artist, t.title, t.version, t.isrc))
        .collect()
}

#[test]
fn an_exportify_like_file_is_read_by_header_name_whatever_the_columns_are() {
    let text = "Track URI,Track Name,Album Name,Artist Name(s),Release Date,ISRC\r\n\
                spotify:track:aaa,Paper Harbor,Harbor Lights,Quill Ashby,2020-01-01,US-ABC-12-34567\r\n\
                spotify:track:bbb,\"Lantern, Part 1\",Night Work,Nova Reed;Dune Moth,2021-02-02,\r\n\
                spotify:track:ccc,Quiet Hours (Dub Mix),Night Work,\"Dune Moth\",2021-02-02,not an isrc\r\n";
    assert_eq!(
        csv_tracks(text),
        [
            (
                "Quill Ashby".to_owned(),
                "Paper Harbor".to_owned(),
                None,
                Some("USABC1234567".to_owned())
            ),
            (
                "Nova Reed, Dune Moth".to_owned(),
                "Lantern, Part 1".to_owned(),
                None,
                None
            ),
            (
                "Dune Moth".to_owned(),
                "Quiet Hours (Dub Mix)".to_owned(),
                None,
                None
            ),
        ]
    );
}

#[test]
fn a_minimal_artist_and_title_file_is_read_and_a_bom_and_unknown_columns_are_ignored() {
    assert_eq!(
        csv_tracks(
            "\u{feff}Title,Plays,Artist,Version\nPaper Harbor,12,Quill Ashby,Extended Mix\n"
        ),
        [(
            "Quill Ashby".to_owned(),
            "Paper Harbor".to_owned(),
            Some("Extended Mix".to_owned()),
            None
        )]
    );
    assert_eq!(
        csv_tracks("artist,title\nQuill Ashby,Paper Harbor").len(),
        1
    );
}

#[test]
fn a_csv_row_without_an_artist_or_a_title_is_skipped_and_counted() {
    let parsed = parse::csv(
        "artist,title\n\
         Quill Ashby,Paper Harbor\n\
         ,Lantern\n\
         \n\
         Nova Reed,\n\
         Dune Moth\n",
    )
    .unwrap();
    assert_eq!(parsed.tracks.len(), 1);
    let skipped: Vec<(u32, WishSkipReason)> =
        parsed.skipped.iter().map(|s| (s.line, s.reason)).collect();
    assert_eq!(
        skipped,
        [
            (3, WishSkipReason::MissingArtist),
            (5, WishSkipReason::MissingTitle),
            (6, WishSkipReason::MissingTitle),
        ]
    );
}

#[test]
fn a_csv_without_an_artist_or_a_title_column_is_refused_by_name() {
    assert_eq!(
        parse::csv("Track Name,Album\nPaper Harbor,Harbor Lights").unwrap_err(),
        CsvProblem::NoArtistColumn
    );
    assert_eq!(
        parse::csv("Artist,Album\nQuill Ashby,Harbor Lights").unwrap_err(),
        CsvProblem::NoTitleColumn
    );
    // "Album Artist" isn't the artist.
    assert_eq!(
        parse::csv("Title,Album Artist\nPaper Harbor,Quill Ashby").unwrap_err(),
        CsvProblem::NoArtistColumn
    );
}

// ---- matching -------------------------------------------------------------

/// A track with one file, its tags giving `title` and `artist`, and
/// `isrc` if given.
fn track(lib: &Lib, title: &str, artist: &str, isrc: Option<&str>) -> (i64, i64) {
    static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let name = format!("{artist} - {title} {n}.mp3");
    let (recording, file) = lib.track(Some(title), Some(artist), &name);
    if let Some(isrc) = isrc {
        let mut frames = vec![
            json!({"key": "TIT2", "value": {"type": "text", "text": title}}),
            json!({"key": "TPE1", "value": {"type": "text", "text": artist}}),
        ];
        frames.push(json!({"key": "TSRC", "value": {"type": "text", "text": isrc}}));
        let raw = json!({ "id3v2": frames }).to_string();
        lib.insert("UPDATE file SET raw_tags = ?1 WHERE id = ?2", (raw, file));
    }
    (recording, file)
}

fn wish(artist: &str, title: &str, version: Option<&str>, isrc: Option<&str>) -> WishTrack {
    WishTrack {
        artist: artist.to_owned(),
        title: title.to_owned(),
        version: version.map(str::to_owned),
        isrc: isrc.map(str::to_owned),
    }
}

fn find(lib: &Lib, w: WishTrack) -> WishMatch {
    lib.writer
        .call(move |c| Ok(Matcher::build(c)?.find(&w)))
        .unwrap()
}

fn key_of(m: &WishMatch) -> String {
    serde_json::to_value(&m.reason).unwrap()["key"]
        .as_str()
        .unwrap()
        .to_owned()
}

#[test]
fn a_track_you_have_is_owned_and_the_reason_says_library_or_all_music() {
    let lib = lib_on();
    let (in_library, file) = track(&lib, "Paper Harbor", "Quill Ashby", None);
    lib.library(in_library, file);
    let (elsewhere, _) = track(&lib, "Lantern", "Nova Reed", None);

    let a = find(&lib, wish("Quill Ashby", "Paper Harbor", None, None));
    assert_eq!(
        (a.kind, a.recording_id, a.in_library),
        (WishMatchKind::Owned, Some(in_library), true)
    );
    assert_eq!(key_of(&a), keys::OWNED_LIBRARY);

    let b = find(&lib, wish("nova  REED", "lantern", None, None));
    assert_eq!(
        (b.kind, b.recording_id, b.in_library),
        (WishMatchKind::Owned, Some(elsewhere), false)
    );
    assert_eq!(key_of(&b), keys::OWNED_ALL_MUSIC);
}

#[test]
fn a_track_you_dont_have_or_by_someone_else_is_missing() {
    let lib = lib_on();
    track(&lib, "Paper Harbor", "Quill Ashby", None);
    for w in [
        wish("Quill Ashby", "Lantern", None, None),
        wish("Nova Reed", "Paper Harbor", None, None),
    ] {
        let m = find(&lib, w);
        assert_eq!((m.kind, m.recording_id), (WishMatchKind::Missing, None));
        assert_eq!(key_of(&m), keys::MISSING);
    }
}

#[test]
fn a_hidden_or_missing_file_does_not_make_a_track_owned_but_another_file_does() {
    let lib = lib_on();
    let (_, hidden_file) = track(&lib, "Paper Harbor", "Quill Ashby", None);
    let (_, gone_file) = track(&lib, "Lantern", "Nova Reed", None);
    let (two_files, _) = track(&lib, "Quiet Hours", "Dune Moth", None);
    lib.insert(
        "UPDATE file SET hidden_reason = 'user' WHERE id = ?1",
        (hidden_file,),
    );
    lib.insert("UPDATE file SET present = 0 WHERE id = ?1", (gone_file,));
    let second = lib.file(two_files, "copy.mp3", "extra");
    lib.insert(
        "UPDATE file SET hidden_reason = 'user' WHERE id = ?1",
        (second,),
    );

    let kind = |a: &str, t: &str| find(&lib, wish(a, t, None, None)).kind;
    assert_eq!(kind("Quill Ashby", "Paper Harbor"), WishMatchKind::Missing);
    assert_eq!(kind("Nova Reed", "Lantern"), WishMatchKind::Missing);
    // One file is hidden and the other is there: the track is owned.
    assert_eq!(kind("Dune Moth", "Quiet Hours"), WishMatchKind::Owned);
}

#[test]
fn another_cut_counts_as_owned_and_a_rework_is_only_a_hint() {
    let lib = lib_on();
    let (cut, _) = track(&lib, "Paper Harbor (Extended Mix)", "Quill Ashby", None);
    let (rework, _) = track(&lib, "Lantern (Dune Moth Remix)", "Nova Reed", None);

    let a = find(
        &lib,
        wish("Quill Ashby", "Paper Harbor", Some("Radio Edit"), None),
    );
    assert_eq!(
        (a.kind, a.recording_id),
        (WishMatchKind::OwnedOtherCut, Some(cut))
    );
    assert_eq!(key_of(&a), keys::OTHER_CUT_ALL_MUSIC);

    let b = find(&lib, wish("Nova Reed", "Lantern", None, None));
    assert_eq!(
        (b.kind, b.recording_id),
        (WishMatchKind::Rework, Some(rework))
    );
    assert_eq!(key_of(&b), keys::REWORK);

    // The same version is owned, with the version asked for in either place.
    let c = find(
        &lib,
        wish("Quill Ashby", "Paper Harbor", Some("Extended Mix"), None),
    );
    assert_eq!((c.kind, c.recording_id), (WishMatchKind::Owned, Some(cut)));
    let d = find(
        &lib,
        wish("Quill Ashby", "Paper Harbor (Extended Mix)", None, None),
    );
    assert_eq!(d.kind, WishMatchKind::Owned);
}

#[test]
fn the_same_version_beats_another_cut_and_a_library_track_beats_one_in_all_music() {
    let lib = lib_on();
    let (_cut, _) = track(&lib, "Paper Harbor (Extended Mix)", "Quill Ashby", None);
    let (exact, _) = track(&lib, "Paper Harbor (Radio Edit)", "Quill Ashby", None);
    let wanted = wish("Quill Ashby", "Paper Harbor", Some("Radio Edit"), None);
    let m = find(&lib, wanted.clone());
    assert_eq!(
        (m.kind, m.recording_id),
        (WishMatchKind::Owned, Some(exact))
    );

    let (twin, twin_file) = track(&lib, "Paper Harbor (Radio Edit)", "Quill Ashby", None);
    lib.library(twin, twin_file);
    let m = find(&lib, wanted);
    assert_eq!((m.recording_id, m.in_library), (Some(twin), true));
}

#[test]
fn an_isrc_match_wins_over_a_name_match() {
    let lib = lib_on();
    let (by_name, _) = track(&lib, "Paper Harbor (Extended Mix)", "Quill Ashby", None);
    let (by_isrc, _) = track(
        &lib,
        "Paper Harbor (Dub Mix)",
        "Quill Ashby",
        Some("ZZ0000000001"),
    );
    let m = find(
        &lib,
        wish(
            "Quill Ashby",
            "Paper Harbor",
            Some("Extended Mix"),
            Some("zz-000-00-00001"),
        ),
    );
    assert_ne!(Some(by_name), m.recording_id);
    assert_eq!(
        (m.kind, m.recording_id),
        (WishMatchKind::Owned, Some(by_isrc))
    );
    assert_eq!(key_of(&m), keys::OWNED_ISRC_ALL_MUSIC);
    // Without the ISRC the name decides.
    let m = find(
        &lib,
        wish("Quill Ashby", "Paper Harbor", Some("Extended Mix"), None),
    );
    assert_eq!(m.recording_id, Some(by_name));
    // An ISRC nobody has falls back to the name too.
    let m = find(
        &lib,
        wish(
            "Quill Ashby",
            "Paper Harbor",
            Some("Extended Mix"),
            Some("QQ0000000009"),
        ),
    );
    assert_eq!(m.recording_id, Some(by_name));
}

#[test]
fn an_isrc_of_a_hidden_file_does_not_count() {
    let lib = lib_on();
    let (_, file) = track(&lib, "Lantern", "Nova Reed", Some("ZZ0000000002"));
    lib.insert(
        "UPDATE file SET hidden_reason = 'user' WHERE id = ?1",
        (file,),
    );
    let m = find(
        &lib,
        wish("Someone Else", "Other Thing", None, Some("ZZ0000000002")),
    );
    assert_eq!(m.kind, WishMatchKind::Missing);
}

#[test]
fn tags_that_are_not_shaped_like_tags_give_no_isrc_and_dont_fail_the_match() {
    for raw in [
        r#"{"id3v2": "oops"}"#,
        r#"{"id3v2": [{"key": 7, "value": 3}]}"#,
        r#"{"id3v2": [{"key": "TSRC"}]}"#,
        r#"{"id3v2": [{"key": "TSRC", "value": {"type": "text", "text": "short"}}]}"#,
        r#"{"id3v2": null, "vorbis_comments": [[]]}"#,
        "{}",
        "not json",
        "[1, 2]",
    ] {
        assert_eq!(isrcs_in(raw), Vec::<String>::new(), "{raw}");
    }
    assert_eq!(
        isrcs_in(
            r#"{"id3v2": [{"key": "TSRC", "value": {"type": "text", "text": "zz-0000-000003\u0000QQ0000000004"}}],
                "vorbis_comments": [{"key": "isrc", "value": {"type": "text", "text": "ZZ0000000005"}}],
                "mp4_ilst": [{"key": "----:com.apple.iTunes:ISRC", "value": {"type": "text", "text": "ZZ0000000006"}}]}"#
        ),
        [
            "ZZ0000000003",
            "QQ0000000004",
            "ZZ0000000006",
            "ZZ0000000005"
        ]
    );

    let lib = lib_on();
    let (bad, bad_file) = track(&lib, "Lantern", "Nova Reed", None);
    lib.insert(
        "UPDATE file SET raw_tags = ?1 WHERE id = ?2",
        (
            r#"{"id3v2": [{"key": "TSRC", "value": 5}], "ape": "x"}"#.to_owned(),
            bad_file,
        ),
    );
    let (good, _) = track(&lib, "Paper Harbor", "Quill Ashby", Some("ZZ0000000007"));
    // The row with unreadable tags is skipped; the rest still matches.
    let m = find(&lib, wish("Who", "What", None, Some("ZZ0000000007")));
    assert_eq!(m.recording_id, Some(good));
    let _ = bad;
}

// ---- writes and Undo ------------------------------------------------------

fn run<T: Send + 'static>(
    lib: &Lib,
    job: impl FnOnce(&mut rusqlite::Connection) -> Result<T, OpsError> + Send + 'static,
) -> T {
    lib.writer.call(move |c| Ok(job(c))).unwrap().unwrap()
}

fn dump(lib: &Lib) -> Vec<String> {
    lib.writer
        .call(|c| {
            let mut out = Vec::new();
            for sql in [
                "SELECT 'w' || id || '|' || name || '|' || source || '|' || created_at
                 FROM wishlist ORDER BY id",
                "SELECT 'i' || id || '|' || wishlist_id || '|' || position || '|' || raw_artist
                     || '|' || raw_title || '|' || ifnull(raw_version, '-') || '|'
                     || ifnull(raw_isrc, '-') || '|' || bought || '|' || ifnull(notes, '-')
                     || '|' || created_at
                 FROM wish_item ORDER BY id",
            ] {
                let mut stmt = c.prepare(sql)?;
                let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
                for row in rows {
                    out.push(row?);
                }
            }
            Ok(out)
        })
        .unwrap()
}

fn operations(lib: &Lib) -> i64 {
    lib.writer
        .call(|c| c.query_row("SELECT count(*) FROM operation", [], |r| r.get(0)))
        .unwrap()
}

fn undo(lib: &Lib) {
    let undone = run(lib, |c| ops::undo_for_user(c, None));
    assert!(
        matches!(undone, ops::UndoOutcome::Undone { .. }),
        "{undone:?}"
    );
}

fn made(outcome: &WishlistOutcome) -> (Option<i64>, i64, u32) {
    match outcome {
        WishlistOutcome::Done {
            operation_id,
            wishlist_id,
            added,
            ..
        } => (*operation_id, wishlist_id.expect("a list"), *added),
        other => panic!("{other:?}"),
    }
}

/// Runs a write that must be one operation and one Undo from where it was.
fn one_operation(
    lib: &Lib,
    write: impl FnOnce(&mut rusqlite::Connection) -> Result<WishlistOutcome, OpsError> + Send + 'static,
) {
    let (before, ops_before) = (dump(lib), operations(lib));
    let outcome = run(lib, write);
    let (operation, ..) = made(&outcome);
    assert!(operation.is_some(), "{outcome:?}");
    assert_eq!(operations(lib), ops_before + 1);
    assert_ne!(dump(lib), before);
    undo(lib);
    assert_eq!(dump(lib), before);
}

fn listed(lib: &Lib) -> i64 {
    let outcome = run(lib, |c| {
        create_from_paste(c, "Friday", "Quill Ashby - Paper Harbor (Extended Mix)\nNova Reed - Lantern\nDune Moth - Quiet Hours")
    });
    made(&outcome).1
}

#[test]
fn creating_a_wishlist_is_one_operation_and_undo_removes_it_with_its_items() {
    let lib = lib_on();
    let (before, ops_before) = (dump(&lib), operations(&lib));
    let outcome = run(&lib, |c| {
        create_from_paste(
            c,
            "  Friday  ",
            "Quill Ashby - Paper Harbor\nnonsense\nNova Reed - Lantern",
        )
    });
    let (operation, list, added) = made(&outcome);
    assert!(operation.is_some());
    assert_eq!(added, 2);
    let WishlistOutcome::Done { skipped, .. } = &outcome else {
        unreachable!()
    };
    assert_eq!(skipped.len(), 1);
    assert_eq!(operations(&lib), ops_before + 1);
    let items = lib.writer.call(move |c| items(c, list)).unwrap();
    assert_eq!(items.iter().map(|i| i.position).collect::<Vec<_>>(), [0, 1]);
    let summaries = lib.writer.call(|c| wishlists(c)).unwrap();
    assert_eq!(
        (
            summaries[0].name.as_str(),
            summaries[0].total,
            summaries[0].source
        ),
        ("Friday", 2, WishlistSource::Paste)
    );
    undo(&lib);
    assert_eq!(dump(&lib), before);
}

#[test]
fn creating_from_a_csv_file_is_one_operation_with_a_csv_source() {
    let lib = lib_on();
    let before = dump(&lib);
    let outcome = run(&lib, |c| {
        create_from_csv(
            c,
            "Export",
            b"Track Name,Artist Name(s)\nPaper Harbor,Quill Ashby\n",
        )
    });
    let (operation, list, added) = made(&outcome);
    assert!((operation.is_some(), added) == (true, 1));
    let summaries = lib.writer.call(|c| wishlists(c)).unwrap();
    assert_eq!(
        (summaries[0].id, summaries[0].source),
        (list, WishlistSource::Csv)
    );
    undo(&lib);
    assert_eq!(dump(&lib), before);
    // A file with no title column writes nothing.
    let refused = run(&lib, |c| {
        create_from_csv(c, "Export", b"Artist\nQuill Ashby\n")
    });
    assert_eq!(
        refused,
        WishlistOutcome::Refused {
            reason: WishlistRefusal::NoTitleColumn
        }
    );
    assert_eq!(dump(&lib), before);
}

#[test]
fn every_other_write_is_one_operation_and_one_undo_puts_it_back() {
    let lib = lib_on();
    let list = listed(&lib);
    let first = lib.writer.call(move |c| items(c, list)).unwrap()[0].id;
    one_operation(&lib, move |c| rename(c, list, "Saturday"));
    one_operation(&lib, move |c| {
        add_from_paste(c, list, "Sable Row - Midnight\nQuill Ashby - Last Light")
    });
    one_operation(&lib, move |c| remove_item(c, first));
    one_operation(&lib, move |c| set_bought(c, first, true));
    one_operation(&lib, move |c| set_notes(c, first, Some("  vinyl only  ")));
    one_operation(&lib, move |c| delete(c, list));
}

#[test]
fn a_write_that_changes_nothing_records_no_operation_and_a_bad_one_is_refused() {
    let lib = lib_on();
    let list = listed(&lib);
    let item = lib.writer.call(move |c| items(c, list)).unwrap()[0].id;
    let (before, ops_before) = (dump(&lib), operations(&lib));
    for outcome in [
        run(&lib, move |c| rename(c, list, "Friday")),
        run(&lib, move |c| set_bought(c, item, false)),
        run(&lib, move |c| set_notes(c, item, Some("  "))),
        run(&lib, move |c| add_from_paste(c, list, "nonsense")),
    ] {
        assert_eq!(made(&outcome).0, None);
    }
    assert_eq!((dump(&lib), operations(&lib)), (before.clone(), ops_before));
    let refusals = [
        (
            run(&lib, |c| rename(c, 99, "x")),
            WishlistRefusal::NoSuchWishlist,
        ),
        (
            run(&lib, move |c| rename(c, list, "  ")),
            WishlistRefusal::NameEmpty,
        ),
        (
            run(&lib, |c| delete(c, 99)),
            WishlistRefusal::NoSuchWishlist,
        ),
        (
            run(&lib, |c| set_bought(c, 99, true)),
            WishlistRefusal::NoSuchItem,
        ),
        (
            run(&lib, |c| remove_item(c, 99)),
            WishlistRefusal::NoSuchItem,
        ),
        (
            run(&lib, |c| add_from_paste(c, 99, "A - B")),
            WishlistRefusal::NoSuchWishlist,
        ),
        (
            run(&lib, move |c| {
                set_notes(c, item, Some(&"x".repeat(MAX_NOTES_CHARS + 1)))
            }),
            WishlistRefusal::NotesTooLong,
        ),
        (
            run(&lib, |c| {
                create_from_paste(c, &"n".repeat(MAX_NAME_CHARS + 1), "A - B")
            }),
            WishlistRefusal::NameTooLong,
        ),
    ];
    for (outcome, reason) in refusals {
        assert_eq!(outcome, WishlistOutcome::Refused { reason });
    }
    assert_eq!((dump(&lib), operations(&lib)), (before, ops_before));
}

#[test]
fn a_paste_with_nothing_readable_makes_no_wishlist_and_says_what_it_skipped() {
    let lib = lib_on();
    let outcome = run(&lib, |c| create_from_paste(c, "Empty", "words\nmore words"));
    let WishlistOutcome::Done {
        operation_id,
        wishlist_id,
        added,
        skipped,
    } = outcome
    else {
        panic!("refused");
    };
    assert_eq!(
        (operation_id, wishlist_id, added, skipped.len()),
        (None, None, 0, 2)
    );
    assert_eq!(dump(&lib), Vec::<String>::new());
}

#[test]
fn the_list_counts_each_result_and_the_bought_items() {
    let lib = lib_on();
    let (owned, file) = track(&lib, "Paper Harbor (Extended Mix)", "Quill Ashby", None);
    lib.library(owned, file);
    track(&lib, "Lantern (Dune Moth Remix)", "Nova Reed", None);
    let list = listed(&lib);
    let item = lib.writer.call(move |c| items(c, list)).unwrap()[0].id;
    run(&lib, move |c| set_bought(c, item, true));
    let summary = lib.writer.call(|c| wishlists(c)).unwrap().remove(0);
    // Paper Harbor (Extended Mix) is owned, Lantern only reworked, Quiet Hours missing.
    assert_eq!(
        (
            summary.total,
            summary.owned,
            summary.owned_other_cut,
            summary.rework,
            summary.missing,
            summary.bought
        ),
        (3, 1, 0, 1, 1, 1)
    );
    let kinds: Vec<WishMatchKind> = lib
        .writer
        .call(move |c| items(c, list))
        .unwrap()
        .iter()
        .map(|i| i.result.kind)
        .collect();
    assert_eq!(
        kinds,
        [
            WishMatchKind::Owned,
            WishMatchKind::Rework,
            WishMatchKind::Missing
        ]
    );
}

// ---- the switch -----------------------------------------------------------

fn switch(lib: &Lib, on: bool) {
    lib.writer
        .call(move |c| write_feature_enabled(c, Feature::Wishlist, on))
        .unwrap();
}

fn is_on(lib: &Lib) -> bool {
    lib.writer.call(|c| enabled(c)).unwrap()
}

#[test]
fn wishlists_are_off_in_a_new_database_and_only_a_true_value_turns_them_on() {
    let lib = Lib::new();
    assert!(!is_on(&lib));
    switch(&lib, true);
    assert!(is_on(&lib));
    switch(&lib, false);
    assert!(!is_on(&lib));
    for odd in ["\"true\"", "1", "null"] {
        lib.writer
            .call(move |c| {
                c.execute(
                    "UPDATE setting SET value = ?1 WHERE key = 'feature_wishlist'",
                    [odd],
                )
            })
            .unwrap();
        assert!(!is_on(&lib), "{odd}");
    }
}

#[test]
fn the_joining_point_for_the_inbox_returns_nothing_while_wishlists_are_off() {
    let lib = lib_on();
    let (_, file) = track(&lib, "Paper Harbor", "Quill Ashby", None);
    listed(&lib);
    assert_eq!(
        lib.writer
            .call(move |c| matching_wish_for(c, file))
            .unwrap(),
        None
    );
    switch(&lib, false);
    assert_eq!(
        lib.writer
            .call(move |c| matching_wish_for(c, file))
            .unwrap(),
        None
    );
}

#[test]
fn every_wishlist_function_refuses_while_wishlists_are_off_except_deleting() {
    let lib = lib_on();
    let list = listed(&lib);
    let item = lib.writer.call(move |c| items(c, list)).unwrap()[0].id;
    switch(&lib, false);
    let (before, ops_before) = (dump(&lib), operations(&lib));
    let off = WishlistOutcome::Refused {
        reason: WishlistRefusal::NotTurnedOn,
    };
    assert_eq!(run(&lib, |c| create_from_paste(c, "x", "A - B")), off);
    assert_eq!(
        run(&lib, |c| create_from_csv(c, "x", b"artist,title\nA,B")),
        off
    );
    assert_eq!(run(&lib, move |c| add_from_paste(c, list, "A - B")), off);
    assert_eq!(run(&lib, move |c| rename(c, list, "y")), off);
    assert_eq!(run(&lib, move |c| remove_item(c, item)), off);
    assert_eq!(run(&lib, move |c| set_bought(c, item, true)), off);
    assert_eq!(run(&lib, move |c| set_notes(c, item, Some("n"))), off);
    assert_eq!(
        lib.writer.call(|c| wishlists_reply(c)).unwrap(),
        WishlistsReply::Refused {
            reason: WishlistRefusal::NotTurnedOn
        }
    );
    assert_eq!(
        lib.writer.call(move |c| items_reply(c, list)).unwrap(),
        WishItemsReply::Refused {
            reason: WishlistRefusal::NotTurnedOn
        }
    );
    assert_eq!((dump(&lib), operations(&lib)), (before, ops_before));

    // Deleting what is there still works, and Undo puts it back.
    let before = dump(&lib);
    let outcome = run(&lib, move |c| delete(c, list));
    assert!(made(&outcome).0.is_some());
    assert_eq!(dump(&lib), Vec::<String>::new());
    undo(&lib);
    assert_eq!(dump(&lib), before);
}

#[test]
fn every_wishlist_command_refuses_over_ipc_while_wishlists_are_off() {
    use crate::ipc::testing::{app, invoke};
    let (_data, app) = app();
    let off = json!({"outcome": "refused", "reason": "notTurnedOn"});
    assert_eq!(
        invoke(&app, "feature_enabled", json!({"feature": "wishlist"})),
        Ok(json!(false))
    );
    for (command, args) in [
        ("wishlists_list", json!({})),
        ("wishlist_items", json!({"wishlistId": 1})),
        (
            "wishlist_create_from_paste",
            json!({"name": "n", "pasted": "A - B"}),
        ),
        (
            "wishlist_create_from_csv",
            json!({"name": "n", "path": "no/such.csv"}),
        ),
        (
            "wishlist_add_from_paste",
            json!({"wishlistId": 1, "pasted": "A - B"}),
        ),
        ("wishlist_rename", json!({"wishlistId": 1, "name": "n"})),
        ("wishlist_remove_item", json!({"itemId": 1})),
        ("wishlist_set_bought", json!({"itemId": 1, "bought": true})),
        ("wishlist_set_notes", json!({"itemId": 1, "notes": "n"})),
    ] {
        assert_eq!(invoke(&app, command, args), Ok(off.clone()), "{command}");
    }
    assert_eq!(
        invoke(
            &app,
            "set_feature_enabled",
            json!({"feature": "wishlist", "on": true})
        ),
        Ok(json!(null))
    );
    assert_eq!(
        invoke(&app, "feature_enabled", json!({"feature": "wishlist"})),
        Ok(json!(true))
    );
    let made = invoke(
        &app,
        "wishlist_create_from_paste",
        json!({"name": "n", "pasted": "A - B"}),
    );
    assert_eq!(made.unwrap()["outcome"], "done");
}
