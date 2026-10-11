//! The final guard: on a full (not identical) audio match, two files merge
//! silently only if their names carry the same version words (module docs,
//! "The final guard").

use super::*;
use crate::versions::label_kind;

/// A file with a title tag and a file name.
fn file(title: &str, file_name: &str) -> FileFacts {
    tagged_and_named(title, file_name)
}

/// The version words of a file, as `kind × count`.
fn label_words(file: &FileFacts) -> Vec<(MarkerKind, usize)> {
    file.label_words.iter().map(|(k, n)| (*k, *n)).collect()
}

fn differ(only_a: &[MarkerKind], only_b: &[MarkerKind]) -> Reason {
    Reason::LabelWordsDiffer {
        only_a: only_a.to_vec(),
        only_b: only_b.to_vec(),
    }
}

// ---- The cases the guard was built for ---------------------------------

#[test]
fn two_title_first_file_names_with_different_label_words_never_merge_on_full_audio() {
    for (name_a, name_b, only_a, only_b) in [
        (
            "Velo Clean - Halden Rook.mp3",
            "Velo Dirty - Halden Rook.mp3",
            MarkerKind::Clean,
            MarkerKind::Dirty,
        ),
        (
            "Velo (Clean) - Halden Rook.mp3",
            "Velo (Radio Edit) - Halden Rook.mp3",
            MarkerKind::Clean,
            MarkerKind::RadioEdit,
        ),
        (
            "Velo Radio Edit - Halden Rook.mp3",
            "Velo Extended Mix - Halden Rook.mp3",
            MarkerKind::RadioEdit,
            MarkerKind::Extended,
        ),
        (
            "Clean - Velo Halden Rook.mp3",
            "Dirty - Velo Halden Rook.mp3",
            MarkerKind::Clean,
            MarkerKind::Dirty,
        ),
    ] {
        let (a, b) = (file("Velo", name_a), file("Velo", name_b));
        let got = decided(full(), &a, &b);
        assert_eq!(got.outcome, Outcome::NeedsReview, "{name_a}");
        assert_eq!(got.reasons[0], differ(&[only_a], &[only_b]), "{name_a}");
        let mirrored = decided(full(), &b, &a);
        assert_eq!(mirrored.outcome, Outcome::NeedsReview, "{name_a}");
        assert_eq!(
            mirrored.reasons[0],
            differ(&[only_b], &[only_a]),
            "{name_a}"
        );
    }
}

#[test]
fn two_untagged_files_with_different_bare_label_words_never_merge_on_full_audio() {
    let (clean, dirty, plain) = (
        named("Velo Clean 128.mp3"),
        named("Velo Dirty 128.mp3"),
        named("Velo.mp3"),
    );
    let got = decided(full(), &clean, &dirty);
    assert_eq!(got.outcome, Outcome::NeedsReview);
    assert_eq!(
        got.reasons,
        [
            differ(&[MarkerKind::Clean], &[MarkerKind::Dirty]),
            Reason::SameAudio {
                coverage_a: 0.98,
                coverage_b: 0.97,
                score: 1.5,
            },
            Reason::NamesDiffer,
        ]
    );
    for (a, b, only_a, only_b) in [
        (&dirty, &plain, vec![MarkerKind::Dirty], vec![]),
        (&plain, &dirty, vec![], vec![MarkerKind::Dirty]),
    ] {
        let got = decided(full(), a, b);
        assert_eq!(got.outcome, Outcome::NeedsReview);
        assert_eq!(got.reasons[0], differ(&only_a, &only_b));
    }
}

#[test]
fn an_artists_label_word_the_name_never_spells_out_as_the_artist_still_counts() {
    // By "Dub Kestrel", named "Velo Dub 128.mp3": the name doesn't say
    // "Dub Kestrel", so its "Dub" is the file's.
    let dub = FileFacts::new(Some("Velo"), Some("Dub Kestrel"), "Velo Dub 128.mp3");
    let plain = FileFacts::new(Some("Velo"), Some("Dub Kestrel"), "nightdrive promo.mp3");
    for (a, b, only_a, only_b) in [
        (&dub, &plain, vec![MarkerKind::Dub], vec![]),
        (&plain, &dub, vec![], vec![MarkerKind::Dub]),
    ] {
        let got = decided(full(), a, b);
        assert_eq!(got.outcome, Outcome::NeedsReview);
        assert_eq!(got.reasons[0], differ(&only_a, &only_b));
    }
}

#[test]
fn a_label_word_in_a_tag_whose_file_name_is_unrelated_never_merges_with_the_plain_title() {
    let got = decided(
        full(),
        &file("Velo Dirty 128", "nightdrive promo.mp3"),
        &file("Velo", "other promo.mp3"),
    );
    assert_eq!(got.outcome, Outcome::NeedsReview);
    assert_eq!(got.reasons[0], differ(&[MarkerKind::Dirty], &[]));
}

#[test]
fn a_label_word_in_a_title_counts_under_two_titles_too() {
    // "Come Clean" beside an unrelated title: the names don't carry the
    // same version words, so the pair isn't merged without asking.
    let got = decided(full(), &titled("Come Clean"), &titled("Nightdrive"));
    assert_eq!(got.outcome, Outcome::NeedsReview);
    assert_eq!(
        codes(&got),
        ["labelWordsDiffer", "sameAudio", "namesDiffer"]
    );
}

// ---- Where the guard keeps out -----------------------------------------

#[test]
fn the_same_label_words_on_both_files_merge_as_the_table_says() {
    for (a, b) in [
        (named("Velo Dirty 128.mp3"), named("Velo Dirty 128.flac")),
        (
            file("Velo", "Velo Clean - Halden Rook.mp3"),
            file("Velo", "Velo Clean - Halden Rook.flac"),
        ),
        (
            file("Velo", "Velo (Dirty).mp3"),
            file("Velo (Dirty)", "nightdrive promo.mp3"),
        ),
        (titled("Come Clean"), titled("Come Clean")),
        (
            file("Come Clean", "Come Clean (Clean).mp3"),
            file("Come Clean", "Halden Rook - Come Clean (Clean).flac"),
        ),
        (titled("Velo (Extended Mix)"), titled("Velo - Extended")),
        (titled("Velo (Original Mix)"), titled("Velo")),
    ] {
        let got = decided(full(), &a, &b);
        assert_eq!(got.outcome, Outcome::Duplicate, "{got:?}");
        assert_eq!(codes(&got)[0], "sameAudio");
    }
}

#[test]
fn identical_audio_merges_whatever_the_label_words() {
    let got = decided(
        Audio::Identical,
        &named("Velo Clean 128.mp3"),
        &named("Velo Dirty 128.mp3"),
    );
    assert_eq!(got.outcome, Outcome::Duplicate);
    assert_eq!(got.reasons, [Reason::IdenticalAudio, Reason::NamesDiffer]);
}

#[test]
fn the_guard_leaves_every_row_but_a_merge_on_full_audio_as_the_table_has_it() {
    let (clean, dirty) = (named("Velo Clean 128.mp3"), named("Velo Dirty 128.mp3"));
    for (what, audio) in every_audio() {
        if what == "full" {
            continue;
        }
        let pair = Pair::new(audio, &clean, &dirty);
        assert_eq!(decide(&pair), by_table(&pair), "{what}");
    }
    // And on full audio, a pair the table already keeps apart.
    let (a, b) = (titled("Velo (Clean)"), titled("Velo (Dirty)"));
    let pair = Pair::new(full(), &a, &b);
    assert_eq!(decide(&pair), by_table(&pair));
    assert_eq!(codes(&decide(&pair)), ["versionVeto"]);
}

// ---- What counts as a file's version words -----------------------------

#[test]
fn a_files_version_words_are_counted_in_the_name_that_writes_each_most() {
    for (title, file_name, words) in [
        (
            "Come Clean",
            "Come Clean (Clean).mp3",
            vec![(MarkerKind::Clean, 2)],
        ),
        (
            "Come Clean",
            "nightdrive promo.mp3",
            vec![(MarkerKind::Clean, 1)],
        ),
        ("Come Clean", "Come Clean.mp3", vec![(MarkerKind::Clean, 1)]),
        (
            "Velo (Dirty)",
            "Velo (Dirty).mp3",
            vec![(MarkerKind::Dirty, 1)],
        ),
        (
            "Velo (Clean)",
            "Velo Dub 128.mp3",
            vec![(MarkerKind::Clean, 1), (MarkerKind::Dub, 1)],
        ),
        ("Velo", "Velo.mp3", vec![]),
    ] {
        assert_eq!(label_words(&file(title, file_name)), words, "{title}");
    }
}

#[test]
fn a_label_is_read_at_its_longest_and_original_is_no_version_word() {
    for (name, words) in [
        ("Velo Radio Edit 128.mp3", vec![(MarkerKind::RadioEdit, 1)]),
        ("Velo (Radio Mix).mp3", vec![(MarkerKind::RadioEdit, 1)]),
        ("Velo Extended Mix.mp3", vec![(MarkerKind::Extended, 1)]),
        ("Velo (Original Mix).mp3", vec![]),
        ("Velo original copy.mp3", vec![]),
        (
            "Velo [Clean] (Extended).mp3",
            vec![(MarkerKind::Extended, 1), (MarkerKind::Clean, 1)],
        ),
    ] {
        assert_eq!(label_words(&named(name)), words, "{name}");
    }
}

#[test]
fn intro_and_outro_together_are_intro_outro_wherever_each_is_written() {
    let both = [(MarkerKind::IntroOutro, 1)];
    assert_eq!(label_words(&file("Velo (Intro)", "Velo (Outro).mp3")), both);
    assert_eq!(label_words(&titled("Velo (Intro Outro)")), both);
    assert_eq!(label_words(&titled("Velo (Intro - Outro)")), both);
    assert_eq!(
        label_words(&titled("Velo (Intro)")),
        [(MarkerKind::Intro, 1)]
    );
    let got = decided(
        full(),
        &file("Velo (Intro)", "Velo (Outro).mp3"),
        &titled("Velo (Intro Outro)"),
    );
    assert_eq!(got.outcome, Outcome::Duplicate);
}

#[test]
fn an_artist_is_set_aside_only_where_the_name_spells_the_whole_artist_out() {
    let by_dub_kestrel = |file_name: &str| {
        label_words(&FileFacts::new(
            Some("Velo"),
            Some("Dub Kestrel"),
            file_name,
        ))
    };
    assert_eq!(by_dub_kestrel("Dub Kestrel Velo copy.mp3"), []);
    assert_eq!(by_dub_kestrel("Velo - Dub Kestrel.mp3"), []);
    // Not the whole artist, or not in a row: the word stays.
    let dub = [(MarkerKind::Dub, 1)];
    assert_eq!(by_dub_kestrel("Velo Dub 128.mp3"), dub);
    assert_eq!(by_dub_kestrel("Dub Velo Kestrel.mp3"), dub);
    // Spelled out once, and a dub besides.
    assert_eq!(by_dub_kestrel("Dub Kestrel - Velo Dub.mp3"), dub);
}

#[test]
fn a_file_names_artist_part_is_set_aside_only_with_a_title_tag_that_is_not_inside_it() {
    // Tagged "Velo": "Dub Kestrel" in front of the dash is the artist.
    assert_eq!(label_words(&file("Velo", "Dub Kestrel - Velo.mp3")), []);
    // Tagged "Velo", and "Velo" is in front of the dash: title first.
    assert_eq!(
        label_words(&file("Velo", "Velo Dub - Halden Rook.mp3")),
        [(MarkerKind::Dub, 1)]
    );
    // No title tag: nothing says which side of the dash the artist is on.
    assert_eq!(
        label_words(&named("Dub Kestrel - Velo.mp3")),
        [(MarkerKind::Dub, 1)]
    );
}

#[test]
fn two_untagged_files_by_an_artist_with_a_label_word_still_merge() {
    // The same words on both files: no difference, whoever "Dub" is.
    let got = decided(
        full(),
        &named("Dub Kestrel - Velo.mp3"),
        &named("Dub Kestrel - Velo.flac"),
    );
    assert_eq!(got.outcome, Outcome::Duplicate);
    assert_eq!(codes(&got), ["sameAudio"]);
}

#[test]
fn folders_in_front_of_a_file_name_carry_no_version_words() {
    assert_eq!(label_words(&named("Live Sets/Dub/Velo.mp3")), []);
    assert_eq!(label_words(&named("D:\\Music\\Clean\\Velo.mp3")), []);
}

#[test]
fn a_label_word_fused_to_another_word_is_not_seen() {
    // The known limit: there is no word break to find it by.
    for name in ["VeloDirty.mp3", "Velo Dirty128.mp3"] {
        assert_eq!(label_words(&named(name)), [], "{name}");
    }
    let got = decided(
        full(),
        &file("Velo", "VeloDirty.mp3"),
        &file("Velo", "Velo.mp3"),
    );
    assert_eq!(got.outcome, Outcome::Duplicate);
}

// ---- Every way to write a label word -----------------------------------

const TITLES: [&str; 2] = ["Velo", "Paper Harbor"];

/// A few cuts and reworks, one of two words, and two spellings of one cut.
const LABELS: [&str; 7] = [
    "Clean",
    "Dirty",
    "Extended",
    "Extended Mix",
    "Radio Edit",
    "Dub",
    "Instrumental",
];

#[derive(Debug, Clone, Copy)]
enum Position {
    Start,
    Middle,
    End,
}

#[derive(Debug, Clone, Copy)]
enum Separator {
    Space,
    Hyphen,
    Underscore,
    Dot,
    Round,
    Square,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Place {
    Tag,
    FileName,
}

/// "`title` copy" with `label` written into it.
fn labeled(title: &str, label: &str, position: Position, separator: Separator) -> String {
    let (label, joint) = match separator {
        Separator::Space => (label.to_string(), " "),
        Separator::Hyphen => (label.to_string(), "-"),
        Separator::Underscore => (label.to_string(), "_"),
        Separator::Dot => (label.to_string(), "."),
        Separator::Round => (format!("({label})"), " "),
        Separator::Square => (format!("[{label}]"), " "),
    };
    match position {
        Position::Start => format!("{label}{joint}{title} copy"),
        Position::Middle => format!("{title}{joint}{label}{joint}copy"),
        Position::End => format!("{title} copy{joint}{label}"),
    }
}

/// A file named "`title` copy" in its tag and its file name, with `label`,
/// if any, written into one of the two.
fn a_file(
    title: &str,
    label: Option<&str>,
    place: Place,
    position: Position,
    separator: Separator,
) -> FileFacts {
    let plain = format!("{title} copy");
    let written = match label {
        Some(label) => labeled(title, label, position, separator),
        None => plain.clone(),
    };
    let (tag, file_name) = match place {
        Place::Tag => (written, plain),
        Place::FileName => (plain, written),
    };
    FileFacts::new(Some(&tag), None, &format!("{file_name}.mp3"))
}

#[test]
fn however_a_label_word_is_written_full_audio_merges_only_files_with_the_same_version_words() {
    let positions = [Position::Start, Position::Middle, Position::End];
    let separators = [
        Separator::Space,
        Separator::Hyphen,
        Separator::Underscore,
        Separator::Dot,
        Separator::Round,
        Separator::Square,
    ];
    let places = [Place::Tag, Place::FileName];
    let labels: Vec<Option<&'static str>> = std::iter::once(None)
        .chain(LABELS.iter().copied().map(Some))
        .collect();

    // How often the guard was what kept a pair apart, and how often equal
    // words merged: neither may be zero, or the loops prove nothing.
    let (mut guarded, mut merged) = (0, 0);
    for title in TITLES {
        for position in positions {
            for separator in separators {
                // Each file of this title, position and separator, once.
                let key = |label: Option<&'static str>, place: Place| (label, place == Place::Tag);
                let files: std::collections::HashMap<_, FileFacts> = labels
                    .iter()
                    .flat_map(|&label| places.map(|place| (label, place)))
                    .map(|(label, place)| {
                        let file = a_file(title, label, place, position, separator);
                        (key(label, place), file)
                    })
                    .collect();
                for &label_a in &labels {
                    for &label_b in &labels {
                        for place_a in places {
                            for place_b in places {
                                let a = &files[&key(label_a, place_a)];
                                let b = &files[&key(label_b, place_b)];
                                let case = format!(
                                    "{title}: {label_a:?} in {place_a:?} / {label_b:?} in \
                                     {place_b:?}, {position:?}, {separator:?}"
                                );
                                let got = decided(full(), a, b);
                                let table = by_table(&Pair::new(full(), a, b));
                                let same_words =
                                    label_a.and_then(label_kind) == label_b.and_then(label_kind);
                                if same_words {
                                    assert_eq!(got, table, "{case}");
                                    merged += usize::from(got.outcome == Outcome::Duplicate);
                                } else {
                                    assert_ne!(got.outcome, Outcome::Duplicate, "{case}");
                                    guarded += usize::from(table.outcome == Outcome::Duplicate);
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    assert!(guarded > 0, "the guard never decided anything");
    assert!(merged > 0, "equal version words never merged");
}

#[test]
fn a_file_names_artist_part_is_not_the_artist_when_it_is_nothing_but_label_words() {
    // "Clean - Velo.mp3", "Radio Edit - Velo.mp3": a label in front, not
    // an artist called that.
    assert_eq!(
        label_words(&file("Velo", "Clean - Velo Halden Rook.mp3")),
        [(MarkerKind::Clean, 1)]
    );
    assert_eq!(
        label_words(&file("Velo", "Radio Edit - Velo.mp3")),
        [(MarkerKind::RadioEdit, 1)]
    );
    assert_eq!(
        label_words(&file("Velo", "Extended Mix - Velo.mp3")),
        [(MarkerKind::Extended, 1)]
    );
}

#[test]
fn a_file_names_artist_part_is_not_the_artist_when_the_tags_title_is_not_in_its_title_part() {
    // The file name is about something else than the tag: nothing says
    // "Nightdrive Clean" is who made it.
    assert_eq!(
        label_words(&file("Velo", "Nightdrive Clean - Halden Rook.mp3")),
        [(MarkerKind::Clean, 1)]
    );
}

#[test]
fn a_file_names_artist_part_is_not_the_artist_when_the_tags_title_is_in_it_too() {
    // The title on both sides of the dash: the front part isn't simply
    // the artist.
    assert_eq!(
        label_words(&file("Velo", "Velo Dub - Velo Halden.mp3")),
        [(MarkerKind::Dub, 1)]
    );
}
