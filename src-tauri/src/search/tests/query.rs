//! What a search finds: the spec of ROADMAP 1.11 "What search covers".

use super::support::{Lib, Mount};
use crate::library::LibraryTrackId;
use crate::search::{match_expression, Why, MAX_TERMS, MAX_TERM_CHARS};

#[test]
fn capitals_and_accents_dont_matter_in_the_text_or_the_search() {
    let lib = Lib::new();
    let (cafe, _) = lib.track(
        Some("Caf\u{e9} Quiet Harbour"),
        Some("\u{c9}nergie"),
        "a.mp3",
    );
    let (plain, _) = lib.track(Some("Plain Title"), None, "b.mp3");
    // The same accent as a letter and a combining mark (NFD).
    let (nfd, _) = lib.track(Some("Cafe\u{301} Nordlicht"), None, "c.mp3");

    for search in ["cafe", "CAFE", "caf\u{e9}", "CAF\u{c9}", "cafe\u{301}"] {
        assert_eq!(lib.finds(search), vec![cafe, nfd], "{search}");
    }
    assert_eq!(lib.finds("energie"), vec![cafe]);
    assert_eq!(lib.finds("\u{c9}NERGIE"), vec![cafe]);
    assert_eq!(lib.finds("PLAIN"), vec![plain]);
}

#[test]
fn a_word_matches_from_its_start_not_from_its_middle() {
    let lib = Lib::new();
    let (house, _) = lib.track(Some("Deep House Nation"), None, "a.mp3");
    lib.track(Some("Warehouse"), None, "b.mp3");

    assert_eq!(lib.finds("hou"), vec![house]);
    assert_eq!(lib.finds("House"), vec![house]);
    assert_eq!(lib.finds("ouse"), Vec::<i64>::new());
    // Every word of the search has to match, in any order.
    assert_eq!(lib.finds("nat deep"), vec![house]);
    assert_eq!(lib.finds("deep warehouse"), Vec::<i64>::new());
}

/// A track whose only mention of `Zebra` is in the field, as its tags or
/// its file name give it.
fn one_field_is_searched(field: &str) {
    let lib = Lib::new();
    let (other, _) = lib.track(Some("Something else"), None, "other.mp3");
    let (target, file) = lib.track(None, None, "target.mp3");
    if field == "File name" {
        lib.insert(
            "UPDATE file SET rel_path = 'Sub/Zebra crossing.mp3',
                 rel_path_key = 'Sub/Zebra crossing.mp3' WHERE id = ?1",
            (file,),
        );
    } else {
        lib.tag(file, &[(field, "Zebra crossing")]);
    }
    assert_eq!(lib.finds("zebr"), vec![target], "{field}");
    assert_eq!(lib.finds("crossing"), vec![target], "{field}");
    assert!(!lib.finds("zebra").contains(&other), "{field}");
}

#[test]
fn the_title_is_searched() {
    one_field_is_searched("Name");
}

#[test]
fn the_artist_is_searched() {
    one_field_is_searched("Artist");
}

#[test]
fn the_album_is_searched() {
    one_field_is_searched("Album");
}

#[test]
fn the_remixer_is_searched() {
    one_field_is_searched("Remixer");
}

#[test]
fn the_label_is_searched() {
    one_field_is_searched("Label");
}

#[test]
fn the_genre_is_searched() {
    one_field_is_searched("Genre");
}

#[test]
fn the_comment_is_searched() {
    one_field_is_searched("Comments");
}

#[test]
fn the_file_name_is_searched() {
    one_field_is_searched("File name");
}

#[test]
fn the_file_name_is_searched_without_its_folder_or_its_extension() {
    let lib = Lib::new();
    let (track, _) = lib.track(Some("Tune"), None, "Crates/Warmup/harbour lights.mp3");
    assert_eq!(lib.finds("harbour"), vec![track]);
    // Not the folders, and not the extension (owner decision, 2026-10-12).
    assert_eq!(lib.finds("warmup"), Vec::<i64>::new());
    assert_eq!(lib.finds("crates"), Vec::<i64>::new());
    assert_eq!(lib.finds("mp3"), Vec::<i64>::new());
}

#[test]
fn a_track_shown_under_its_file_name_is_found_by_it_without_the_extension() {
    let lib = Lib::new();
    let (track, _) = lib.track(None, None, "Sub/Untitled jam.flac");
    assert_eq!(lib.finds("untitled"), vec![track]);
    assert_eq!(lib.finds("flac"), Vec::<i64>::new());
}

#[test]
fn every_file_of_a_track_is_searched_by_name() {
    let lib = Lib::new();
    let (track, _) = lib.track(Some("Tune"), None, "tune.flac");
    lib.file(track, "Old/tune (copy).mp3", "extra");
    assert_eq!(lib.finds("copy"), vec![track]);
}

#[test]
fn a_library_track_is_found_by_the_title_rekordbox_has_for_it_and_the_lists_show() {
    let lib = Lib::new();
    let (track, file) = lib.track(Some("Tag title"), Some("Tag artist"), "known.mp3");
    lib.library(track, file);
    lib.rekordbox_entry(
        file,
        &[
            ("Name", "Their title"),
            ("Artist", "Their artist"),
            ("Album", "Their album"),
            ("Remixer", "Their remixer"),
            ("Label", "Their label"),
            ("Genre", "Their genre"),
            ("Comments", "Their comment"),
        ],
    );
    for word in [
        "title", "artist", "album", "remixer", "label", "genre", "comment",
    ] {
        assert_eq!(lib.finds(&format!("their {word}")), vec![track], "{word}");
    }
    // What the lists show is rekordbox's, so the tag's title isn't shown.
    assert_eq!(lib.finds("tag"), Vec::<i64>::new());
}

#[test]
fn the_index_holds_what_the_lists_show_for_every_kind_of_track() {
    let lib = Lib::new();
    let (known, known_file) = lib.track(Some("Tag one"), None, "one.mp3");
    let first = lib.library(known, known_file);
    lib.rekordbox_entry(known_file, &[("Name", "Shown by rekordbox one")]);
    let (linked, linked_file) =
        lib.track(Some("Shown from tags two"), Some("Artist two"), "two.mp3");
    let second = lib.library(linked, linked_file);
    let (plain, _) = lib.track(Some("Shown from tags three"), None, "three.mp3");
    let (nameless, _) = lib.track(None, None, "Shown from the file four.mp3");
    lib.settle();

    let shown = lib
        .writer
        .call(move |c| {
            let in_library = crate::send_values::shown(
                c,
                &Mount,
                &[LibraryTrackId(first), LibraryTrackId(second)],
            )?;
            let elsewhere = crate::send_values::shown_recordings(c, &[plain, nameless])?;
            let mut titles: Vec<(i64, String)> = vec![
                (known, in_library[&LibraryTrackId(first)].title.clone()),
                (linked, in_library[&LibraryTrackId(second)].title.clone()),
            ];
            titles.extend(elsewhere.into_iter().map(|(id, s)| (id, s.title)));
            Ok(titles)
        })
        .unwrap();
    assert_eq!(shown.len(), 4);
    for (track, title) in shown {
        // The title as the lists show it (a file's name without its
        // extension where it stands in for a missing title).
        let title = crate::search::file_name_text(&title);
        assert_eq!(lib.finds(&title), vec![track], "{title}");
    }
}

#[test]
fn a_track_not_in_the_library_takes_nothing_from_rekordbox() {
    let lib = Lib::new();
    let (track, file) = lib.track(Some("Tag title"), None, "plain.mp3");
    // rekordbox has the file, under another title, but the track isn't in
    // the Library.
    lib.rekordbox_entry(file, &[("Name", "Their title"), ("Album", "Their album")]);
    assert_eq!(lib.finds("tag"), vec![track]);
    assert_eq!(lib.finds("their"), Vec::<i64>::new());
    let before = lib.index_contents();

    // A rekordbox read that replaces everything changes none of its row.
    lib.exec("DELETE FROM rekordbox_track");
    assert!(lib.pending(), "a rekordbox read is noted");
    assert_eq!(lib.index_contents(), before);
}

#[test]
fn a_retagged_files_old_words_stop_matching_and_its_new_ones_match() {
    let lib = Lib::new();
    let (track, file) = lib.track(Some("Old Words"), Some("Same Artist"), "t.mp3");
    assert_eq!(lib.finds("old"), vec![track]);
    lib.tag(file, &[("Name", "Fresh Words"), ("Artist", "Same Artist")]);
    assert!(lib.pending());
    assert_eq!(lib.finds("old"), Vec::<i64>::new());
    assert_eq!(lib.finds("fresh"), vec![track]);
    assert_eq!(lib.finds("same"), vec![track]);
}

#[test]
fn a_renamed_file_is_found_under_its_new_name_only() {
    let lib = Lib::new();
    let (track, file) = lib.track(Some("Tune"), None, "before.mp3");
    assert_eq!(lib.finds("before"), vec![track]);
    lib.insert(
        "UPDATE file SET rel_path = 'after.mp3', rel_path_key = 'after.mp3' WHERE id = ?1",
        (file,),
    );
    assert_eq!(lib.finds("before"), Vec::<i64>::new());
    assert_eq!(lib.finds("after"), vec![track]);
}

#[test]
fn a_file_moved_to_another_track_takes_its_words_with_it() {
    let lib = Lib::new();
    let (one, file) = lib.track(Some("Moving"), None, "move.mp3");
    let (two, _) = lib.track(Some("Staying"), None, "stay.mp3");
    assert_eq!(lib.finds("moving"), vec![one]);
    lib.insert(
        "UPDATE recording_file SET recording_id = ?1, role = 'extra' WHERE file_id = ?2",
        (two, file),
    );
    // `one` has no file now: it is no track in the lists, and has no row.
    assert_eq!(lib.finds("moving"), Vec::<i64>::new());
    assert_eq!(lib.finds("move"), vec![two]);
    assert_eq!(lib.finds("staying"), vec![two]);
}

#[test]
fn a_removed_track_is_not_found_any_more() {
    let lib = Lib::new();
    let (gone, _) = lib.track(Some("Short Lived"), None, "x.mp3");
    let (kept, _) = lib.track(Some("Short Kept"), None, "y.mp3");
    assert_eq!(lib.finds("short"), vec![gone, kept]);
    lib.insert(
        "DELETE FROM recording_file WHERE recording_id = ?1",
        (gone,),
    );
    lib.insert("DELETE FROM recording WHERE id = ?1", (gone,));
    assert_eq!(lib.finds("short"), vec![kept]);
}

#[test]
fn a_track_with_no_file_and_not_in_the_library_has_no_row() {
    let lib = Lib::new();
    lib.insert("INSERT INTO recording (title) VALUES ('Orphan words')", ());
    assert_eq!(lib.finds("orphan"), Vec::<i64>::new());
    // A Library track has one, whatever its file's state.
    let (track, file) = lib.track(None, None, "listed.mp3");
    lib.library(track, file);
    assert_eq!(lib.finds("listed"), vec![track]);
}

#[test]
fn a_hidden_file_is_found_and_marked_hidden_with_its_reason() {
    let lib = Lib::new();
    let (by_user, user_file) = lib.track(Some("Hidden by user"), None, "u.mp3");
    let (cleared, cleared_file) = lib.track(Some("Hidden cleared inbox"), None, "c.mp3");
    let (extra_of, _) = lib.track(Some("Hidden extra"), None, "e.flac");
    let extra_file = lib.file(extra_of, "e-copy.mp3", "extra");
    lib.insert(
        "UPDATE file SET hidden_reason = 'user' WHERE id = ?1",
        (user_file,),
    );
    lib.insert(
        "UPDATE file SET hidden_reason = 'inbox_cleared' WHERE id = ?1",
        (cleared_file,),
    );

    let found = lib.found("hidden").unwrap();
    assert_eq!(found.tracks, vec![by_user, cleared, extra_of]);
    let hidden: Vec<_> = found.hidden.iter().map(|h| (h.file_id, h.why)).collect();
    assert_eq!(
        hidden,
        vec![
            (user_file, Why::UserHid),
            (cleared_file, Why::ClearedInbox),
            (extra_file, Why::ExtraFile)
        ]
    );
    // A search that matches only shown files reports none.
    lib.track(Some("Visible"), None, "v.mp3");
    assert!(lib.found("visible").unwrap().hidden.is_empty());
}

#[test]
fn a_hidden_extra_copy_of_a_shown_track_says_the_track_has_a_shown_file_and_a_lone_hidden_file_says_it_has_none(
) {
    // Owner decision, 2026-10-12: "N hidden files match" counts only the
    // hidden files whose track isn't in the results already.
    let lib = Lib::new();
    let (shown_track, _) = lib.track(Some("Both kinds"), None, "shown.flac");
    let extra = lib.file(shown_track, "shown (copy).mp3", "extra");
    let (lone_track, lone_file) = lib.track(Some("Both kinds lonely"), None, "lone.mp3");
    lib.insert(
        "UPDATE file SET hidden_reason = 'user' WHERE id = ?1",
        (lone_file,),
    );

    let found = lib.found("both").unwrap();
    assert_eq!(found.tracks, vec![shown_track, lone_track]);
    let flags: Vec<_> = found
        .hidden
        .iter()
        .map(|h| (h.file_id, h.recording_id, h.track_has_shown_file))
        .collect();
    assert_eq!(
        flags,
        vec![(extra, shown_track, true), (lone_file, lone_track, false)]
    );
}

#[test]
fn text_with_no_letters_or_digits_filters_nothing() {
    let lib = Lib::new();
    lib.track(Some("Anything"), None, "a.mp3");
    for text in ["", "   ", "-", "*", "\"", "\"\"", "()", "- * :", "\u{3000}"] {
        assert_eq!(match_expression(text), None, "{text:?}");
        assert_eq!(lib.found(text), None, "{text:?}");
    }
}

#[test]
fn hostile_text_is_only_text_and_never_a_syntax_error() {
    let lib = Lib::new();
    let (android, _) = lib.track(Some("Android Nearly"), Some("Orion"), "a.mp3");
    let (quoted, _) = lib.track(Some("Say \"Hello\" Now"), Some("Zed"), "b.mp3");
    let (plain, _) = lib.track(Some("Plain"), Some("Title Colon"), "c.mp3");

    // Operators and column filters are words like any other.
    assert_eq!(lib.finds("AND"), vec![android]);
    assert_eq!(lib.finds("OR"), vec![android]);
    assert_eq!(lib.finds("NEAR"), vec![android]);
    assert_eq!(lib.finds("android AND nearly"), vec![android]);
    assert_eq!(lib.finds("NEAR(android nearly)"), Vec::<i64>::new());
    assert_eq!(lib.finds("title:plain"), Vec::<i64>::new());
    assert_eq!(lib.finds("artist:zed"), Vec::<i64>::new());
    assert_eq!(lib.finds("{title}:plain"), Vec::<i64>::new());
    assert_eq!(lib.finds("-plain"), vec![plain]);
    assert_eq!(lib.finds("^plain"), vec![plain]);
    assert_eq!(lib.finds("pl*"), vec![plain]);
    assert_eq!(lib.finds("*plain*"), vec![plain]);
    assert_eq!(lib.finds("title: colon"), vec![plain]);
    // Quotes, in any state of balance.
    assert_eq!(lib.finds("\"hello\""), vec![quoted]);
    assert_eq!(lib.finds("\"hello"), vec![quoted]);
    assert_eq!(lib.finds("hello\""), vec![quoted]);
    assert_eq!(lib.finds("say \"\"\" now"), vec![quoted]);
    // Nothing of these can fail or find everything.
    for text in [
        "(",
        ")",
        "((a",
        "a OR",
        "OR a",
        "a AND",
        "NOT a",
        "x:",
        ":x",
        "a + b",
        "a, b",
        "\"a\"\"b\"",
        "\\",
        "'",
        "%",
        "_",
        "?",
        "\u{0}",
        "\u{1f}",
        "\u{ff1a}",
        "\u{1f3b5}",
        "a\u{0}b",
    ] {
        let found = lib.found(text);
        assert!(
            found.as_ref().is_none_or(|f| f.tracks.len() < 3),
            "{text:?} found everything: {found:?}"
        );
    }
}

#[test]
fn a_query_is_built_from_a_bounded_number_of_bounded_words() {
    let words: Vec<String> = (0..40).map(|n| format!("word{n}")).collect();
    let expression = match_expression(&words.join(" ")).unwrap();
    assert_eq!(expression.matches("\"*").count(), MAX_TERMS);
    let long = "x".repeat(MAX_TERM_CHARS * 3);
    let expression = match_expression(&long).unwrap();
    assert_eq!(expression, format!("\"{}\"*", "x".repeat(MAX_TERM_CHARS)));

    // The words past the limit are ignored, not an error.
    let lib = Lib::new();
    let title = (0..MAX_TERMS)
        .map(|n| format!("w{n}"))
        .collect::<Vec<_>>()
        .join(" ");
    let (track, _) = lib.track(Some(&title), None, "a.mp3");
    let search = format!("{title} thirteenth fourteenth");
    assert_eq!(lib.finds(&search), vec![track]);
    // A word longer than the limit matches by its first characters.
    let (long_track, _) = lib.track(Some(&long), None, "b.mp3");
    assert_eq!(lib.finds(&long), vec![long_track]);
}

#[test]
fn only_the_start_of_a_long_comment_is_searched() {
    let lib = Lib::new();
    let (track, file) = lib.track(Some("Chatty"), None, "a.mp3");
    let comment = format!("early {} late", "filler ".repeat(300));
    lib.tag(file, &[("Name", "Chatty"), ("Comments", &comment)]);
    assert_eq!(lib.finds("early"), vec![track]);
    assert_eq!(lib.finds("late"), Vec::<i64>::new());
}

#[test]
fn a_search_says_when_the_index_is_behind_the_tracks() {
    let lib = Lib::new();
    let (track, file) = lib.track(Some("Behind"), None, "a.mp3");
    assert!(lib.found_as_is("behind").unwrap().behind);
    assert_eq!(lib.finds("behind"), vec![track]);
    assert!(!lib.found_as_is("behind").unwrap().behind);
    lib.tag(file, &[("Name", "Ahead")]);
    let stale = lib.found_as_is("behind").unwrap();
    assert!(stale.behind, "a retag is noted");
    assert_eq!(
        stale.tracks,
        vec![track],
        "the old row stands until it is settled"
    );
    assert!(!lib.found("ahead").unwrap().behind);
}

#[test]
fn the_hidden_files_are_the_same_whether_looked_up_by_track_or_by_walking_them() {
    let lib = Lib::new();
    let mut tracks = Vec::new();
    for n in 0..6 {
        let (track, file) = lib.track(Some(&format!("Common {n}")), None, &format!("{n}.mp3"));
        match n % 3 {
            0 => {
                lib.file(track, &format!("{n}-copy.mp3"), "extra");
            }
            1 => {
                lib.insert(
                    "UPDATE file SET hidden_reason = 'inbox_cleared' WHERE id = ?1",
                    (file,),
                );
            }
            _ => {}
        }
        tracks.push(track);
    }
    lib.settle();
    let both = |wanted: Vec<i64>| {
        lib.writer
            .call(move |c| {
                Ok((
                    crate::search::query::hidden_files(c, &wanted, true)?,
                    crate::search::query::hidden_files(c, &wanted, false)?,
                ))
            })
            .unwrap()
    };
    let (by_track, walked) = both(tracks.clone());
    assert_eq!(by_track.len(), 4);
    assert_eq!(by_track, walked);
    let (by_track, walked) = both(vec![tracks[1], tracks[3]]);
    assert_eq!(by_track.len(), 2);
    assert_eq!(by_track, walked);
}

#[test]
fn a_real_title_keeps_a_dotted_tail_and_only_a_file_name_standing_in_for_a_title_loses_its_extension(
) {
    let lib = Lib::new();
    // The title is the tag's, however much it looks like a file name.
    let (dotted, _) = lib.track(Some("Quiet.v2"), None, "Quiet.mp3");
    let (looks_like_a_file, _) = lib.track(Some("Lantern.mp3"), None, "Lantern.flac");
    // No title: the file's name stands in for it, without its extension.
    let (nameless, _) = lib.track(None, None, "Ember.wav");
    assert_eq!(lib.finds("v2"), vec![dotted]);
    assert_eq!(lib.finds("mp3"), vec![looks_like_a_file]);
    assert_eq!(lib.finds("ember"), vec![nameless]);
    assert_eq!(lib.finds("wav"), Vec::<i64>::new());
}

#[test]
fn a_track_whose_best_file_is_hidden_is_not_shown_by_an_unhidden_extra_copy() {
    // An extra copy is hidden by being an extra, whatever its own state,
    // so a track whose best file the user hid has no shown file: the lists
    // have no file to show the track through.
    let lib = Lib::new();
    let (track, best) = lib.track(Some("Hidden best"), None, "best.flac");
    let extra = lib.file(track, "best (copy).mp3", "extra");
    lib.insert(
        "UPDATE file SET hidden_reason = 'user' WHERE id = ?1",
        (best,),
    );
    let found = lib.found("hidden").unwrap();
    assert_eq!(found.tracks, vec![track]);
    let flags: Vec<_> = found
        .hidden
        .iter()
        .map(|h| (h.file_id, h.why, h.track_has_shown_file))
        .collect();
    assert_eq!(
        flags,
        vec![(best, Why::UserHid, false), (extra, Why::ExtraFile, false)]
    );
}

#[test]
fn an_accented_search_finds_text_without_accents() {
    let lib = Lib::new();
    let (plain, _) = lib.track(Some("Cafe Lumen"), Some("Renee"), "a.mp3");
    lib.track(Some("Other"), None, "b.mp3");
    for search in [
        "caf\u{e9}",
        "CAF\u{c9}",
        "cafe\u{301}",
        "ren\u{e9}e",
        "Ren\u{c9}E",
    ] {
        assert_eq!(lib.finds(search), vec![plain], "{search}");
    }
}

#[test]
fn words_around_punctuation_match_by_the_start_of_each_part() {
    let lib = Lib::new();
    let (slash, _) = lib.track(Some("Aqua/Brio"), None, "a.mp3");
    let (apostrophe, _) = lib.track(Some("Marlo's Dream"), None, "b.mp3");
    let (hyphen, _) = lib.track(Some("beta-test"), None, "c.mp3");
    for (search, expected) in [
        ("brio", slash),
        ("aqua", slash),
        ("aqua/brio", slash),
        ("aqua/br", slash),
        ("dream", apostrophe),
        ("marlo", apostrophe),
        ("marlo's", apostrophe),
        ("marlo's dre", apostrophe),
        ("test", hyphen),
        ("beta", hyphen),
        ("beta-te", hyphen),
    ] {
        assert_eq!(lib.finds(search), vec![expected], "{search}");
    }
    // Not from the middle of a part.
    assert_eq!(lib.finds("rio"), Vec::<i64>::new());
}
