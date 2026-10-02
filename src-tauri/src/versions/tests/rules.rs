//! One test per hard rule of ROADMAP §1.5, and per promise in the module
//! docs.

use super::super::{
    compare, parse_file_name, parse_title, JunkKind, MarkerForm, MarkerKind, Outcome, Reason, Side,
    VersionClass,
};

// ---- Length decides nothing -------------------------------------------

#[test]
fn length_decides_nothing_a_running_time_in_the_name_is_reported_as_junk_and_ignored() {
    let short = parse_title("Glasswing (2:58)");
    let long = parse_title("Glasswing (9:41)");
    assert_eq!(short.junk[0].kind, JunkKind::Duration);
    assert_eq!(short.junk[0].text, "(2:58)");
    assert!(short.markers.is_empty());
    assert_eq!(compare(&short, &long).outcome, Outcome::SameVersion);
}

#[test]
fn length_decides_nothing_short_and_extended_are_just_two_different_cuts() {
    let short = parse_title("Glasswing (Short Edit)");
    let extended = parse_title("Glasswing (Extended Mix)");
    let one_way = compare(&short, &extended);
    let other_way = compare(&extended, &short);
    assert_eq!(one_way.outcome, Outcome::DifferentCut);
    assert_eq!(other_way.outcome, Outcome::DifferentCut);
    // The reason names the kinds; nothing ranks one above the other.
    assert_eq!(
        one_way.reasons[1],
        Reason::CutDiffers {
            only_a: vec![MarkerKind::Short],
            only_b: vec![MarkerKind::Extended],
        }
    );
}

#[test]
fn length_decides_nothing_a_long_junk_laden_name_equals_its_short_clean_one() {
    let long = parse_file_name(
        "Downloads/10000001_Halden_Rook_-_Orbit_Line_&#40;Original Mix&#41; [ example-ripper ] (320kbps) - RipSite.example.mp3",
    );
    let short = parse_title("Orbit Line");
    assert_eq!(long.base_title, "Orbit Line");
    assert_eq!(long.credits[0].name, "Halden Rook");
    assert_eq!(long.junk.len(), 9);
    assert_eq!(compare(&long, &short).outcome, Outcome::SameVersion);
}

// ---- A version word can be the real title -----------------------------

#[test]
fn a_bracketed_label_is_never_removed_from_the_original() {
    let name = parse_title("Velo (dirty)");
    assert_eq!(name.original, "Velo (dirty)");
    assert_eq!(name.markers[0].text, "(dirty)");
}

#[test]
fn a_title_ending_in_a_label_offers_the_reading_where_the_label_is_the_title() {
    let name = parse_title("Velo (dirty)");
    let readings = name.readings();
    assert_eq!(readings.len(), 2);
    assert_eq!(readings[0].base_title, "Velo");
    assert_eq!(readings[1].base_title, "Velo (dirty)");
    assert!(readings[1].markers.is_empty());
    assert_eq!(readings[1].demoted, ["(dirty)"]);
}

#[test]
fn a_label_written_twice_keeps_its_first_copy_in_the_title_and_says_it_is_doubtful() {
    let name = parse_title("Velo (dirty) (dirty)");
    assert_eq!(name.base_title, "Velo (dirty)");
    assert_eq!(name.markers.len(), 1);
    assert!(!name.markers[0].ambiguous);
    assert_eq!(name.title_markers.len(), 1);
    assert!(name.title_markers[0].ambiguous);
}

#[test]
fn the_other_name_settles_whether_a_label_is_the_title() {
    let titled = parse_title("Velo (dirty)");
    // Beside the plain title, "(dirty)" is a marker…
    let plain = compare(&titled, &parse_title("Velo"));
    assert_eq!(plain.outcome, Outcome::DifferentCut);
    // …and beside the name that carries it twice, it is the title.
    let twice = compare(&titled, &parse_title("Velo (dirty) (dirty)"));
    assert_eq!(twice.outcome, Outcome::DifferentCut);
    assert!(twice.reasons.contains(&Reason::MarkerReadAsTitle {
        side: Side::A,
        text: "(dirty)".to_string(),
    }));
    assert!(twice.reasons.contains(&Reason::CutDiffers {
        only_a: vec![],
        only_b: vec![MarkerKind::Dirty],
    }));
}

#[test]
fn label_words_without_brackets_are_never_stripped_from_the_title() {
    for title in [
        "Come Clean",
        "Long Live",
        "Stay Dirty",
        "Paper Harbor Extended Mix",
    ] {
        let name = parse_title(title);
        assert_eq!(name.base_title, title);
        assert!(name.markers.is_empty(), "{title}");
        assert_eq!(name.title_markers.len(), 1, "{title}");
        assert!(name.title_markers[0].ambiguous, "{title}");
        assert_eq!(name.title_markers[0].form, MarkerForm::Bare, "{title}");
    }
}

#[test]
fn label_words_without_brackets_against_the_plain_title_cant_be_told() {
    let got = compare(&parse_title("Come Clean"), &parse_title("Come"));
    assert_eq!(got.outcome, Outcome::CantTell);
    assert!(got.reasons.contains(&Reason::TitleReadAsMarker {
        side: Side::A,
        text: "Clean".to_string(),
    }));
}

#[test]
fn a_title_that_is_only_a_label_word_stays_a_title() {
    for title in [
        "Intro",
        "Original",
        "Dub",
        "Live",
        "(Intro)",
        "Extended Mix",
    ] {
        let name = parse_title(title);
        assert_eq!(name.base_title, title);
        assert!(name.markers.is_empty(), "{title}");
        assert!(name.title_markers[0].ambiguous, "{title}");
    }
}

#[test]
fn a_label_after_a_dash_is_a_marker_flagged_as_doubtful() {
    let name = parse_title("Glasswing - Extended Mix");
    assert_eq!(name.base_title, "Glasswing");
    assert_eq!(name.markers[0].kind, MarkerKind::Extended);
    assert_eq!(name.markers[0].form, MarkerForm::DashSegment);
    assert!(name.markers[0].ambiguous);
}

#[test]
fn a_bracketed_label_in_its_usual_place_is_not_flagged() {
    let name = parse_title("Glasswing (Extended Mix)");
    assert_eq!(name.markers[0].form, MarkerForm::Bracketed);
    assert!(!name.markers[0].ambiguous);
}

// ---- A remix may be credited only to the remixer ----------------------

#[test]
fn a_remix_keeps_its_remixer_as_the_markers_detail() {
    let name = parse_file_name("Quill Ashby/Paper Harbor (Quill Ashby Remix).mp3");
    assert_eq!(name.markers[0].kind, MarkerKind::Remix);
    assert_eq!(name.markers[0].detail.as_deref(), Some("Quill Ashby"));
    assert_eq!(name.markers[0].kind.class(), VersionClass::Rework);
}

#[test]
fn a_remix_filed_under_the_remixer_alone_matches_the_one_filed_under_the_artist() {
    let by_remixer = parse_file_name("Quill Ashby - Paper Harbor (Quill Ashby Remix).mp3");
    let by_artist = parse_file_name("Kestrel Nine - Paper Harbor (Quill Ashby Remix).flac");
    let got = compare(&by_remixer, &by_artist);
    assert_eq!(got.outcome, Outcome::SameVersion);
    assert!(got
        .reasons
        .contains(&Reason::CreditedToRemixer { side: Side::A }));
}

#[test]
fn a_remix_filed_under_the_remixer_alone_is_a_rework_of_the_artists_original() {
    let remix = parse_file_name("Quill Ashby - Paper Harbor (Quill Ashby Remix).mp3");
    let original = parse_file_name("Kestrel Nine - Paper Harbor (Original Mix).flac");
    let got = compare(&remix, &original);
    assert_eq!(got.outcome, Outcome::DifferentRework);
    assert!(!got.reasons.contains(&Reason::ArtistsDiffer));
}

// ---- The same title can be different songs ----------------------------

#[test]
fn the_same_title_under_different_artists_cant_be_told() {
    let one = parse_file_name("Kestrel Nine - Paper Harbor.flac");
    let other = parse_file_name("The Marrow Choir - Paper Harbor.m4a");
    let got = compare(&one, &other);
    assert_eq!(got.outcome, Outcome::CantTell);
    assert!(got.reasons.contains(&Reason::ArtistsDiffer));
}

#[test]
fn the_same_title_as_two_remixes_is_described_not_linked() {
    // Two songs called "Korvex", each known only by a remix. The names
    // can only say "different rework"; whether they share a source is for
    // the fingerprint and the user. Nothing here says "same".
    let one = parse_title("Korvex (Rivetta Remix)");
    let other = parse_title("Korvex (Flint Bootleg)");
    assert_eq!(compare(&one, &other).outcome, Outcome::DifferentRework);
}

#[test]
fn identical_names_say_same_version_and_nothing_about_the_audio() {
    // Identical tags can hide a different recording (§1.5): the answer is
    // about the names, and its reasons say only that.
    let got = compare(&parse_title("Paper Harbor"), &parse_title("Paper Harbor"));
    assert_eq!(got.outcome, Outcome::SameVersion);
    assert_eq!(got.reasons, [Reason::BaseTitlesMatch, Reason::SameMarkers]);
}

// ---- Mashup part names can match unrelated tracks ---------------------

#[test]
fn a_mashup_lists_its_parts_as_possible_source_titles() {
    let name = parse_title("Paper Harbor x Neon Moth (Vey Sun Mashup)");
    assert_eq!(name.mashup_parts, ["Paper Harbor", "Neon Moth"]);
    assert_eq!(name.base_title, "Paper Harbor x Neon Moth");
    assert_eq!(name.markers[0].kind, MarkerKind::Mashup);
    assert_eq!(name.markers[0].detail.as_deref(), Some("Vey Sun"));
}

#[test]
fn a_mashup_part_matching_another_title_is_only_a_hint() {
    let mashup = parse_title("Duskline x Emberlow (Mashup)");
    let namesake = parse_title("Duskline");
    let got = compare(&mashup, &namesake);
    assert_eq!(got.outcome, Outcome::CantTell);
    assert_eq!(
        got.reasons,
        [
            Reason::BaseTitlesDiffer,
            Reason::MashupPartMatch {
                side: Side::A,
                part: "Duskline".to_string(),
            },
        ]
    );
}

#[test]
fn a_mashup_without_its_label_is_flagged_as_doubtful() {
    let name = parse_title("Duskline x Emberlow");
    assert_eq!(name.markers.len(), 1);
    assert_eq!(name.markers[0].kind, MarkerKind::Mashup);
    assert_eq!(name.markers[0].form, MarkerForm::Separator);
    assert!(name.markers[0].ambiguous);
    assert_eq!(name.base_title, "Duskline x Emberlow");
}

// ---- Junk is reported, the original kept ------------------------------

#[test]
fn everything_stripped_is_listed_as_junk_as_it_was_written() {
    let name = parse_file_name("Beatless/10000001_Orbit_Line_&#40;Original Mix&#41;.mp3");
    let kinds: Vec<JunkKind> = name.junk.iter().map(|j| j.kind).collect();
    assert_eq!(
        kinds,
        [
            JunkKind::FolderPath,
            JunkKind::FileExtension,
            JunkKind::HtmlEntity,
            JunkKind::HtmlEntity,
            JunkKind::StoreId,
            JunkKind::Underscores,
        ]
    );
    for junk in &name.junk {
        assert!(name.original.contains(&junk.text), "{:?}", junk.text);
    }
    assert_eq!(
        name.original,
        "Beatless/10000001_Orbit_Line_&#40;Original Mix&#41;.mp3"
    );
}

#[test]
fn a_name_that_is_nothing_but_junk_keeps_itself_as_its_title() {
    for title in ["ripsite.example", "[ example-ripper ]", "(320)"] {
        let name = parse_title(title);
        assert_eq!(name.base_title, title);
    }
}

// ---- When unsure, can't tell -------------------------------------------

#[test]
fn a_bracket_that_isnt_understood_is_kept_and_makes_the_pair_cant_tell() {
    let name = parse_title("Glasswing (Skyline Session)");
    assert_eq!(name.base_title, "Glasswing");
    assert_eq!(name.unrecognized, ["(Skyline Session)"]);
    assert!(name.markers.is_empty());
    let got = compare(&name, &parse_title("Glasswing"));
    assert_eq!(got.outcome, Outcome::CantTell);
    assert_eq!(
        got.reasons[1],
        Reason::UnrecognizedDiffers {
            only_a: vec!["(Skyline Session)".to_string()],
            only_b: vec![],
        }
    );
}

#[test]
fn a_rework_without_a_name_beside_one_with_a_name_cant_be_told() {
    let got = compare(
        &parse_title("Paper Harbor (Remix)"),
        &parse_title("Paper Harbor (Quill Ashby Remix)"),
    );
    assert_eq!(got.outcome, Outcome::CantTell);
    assert_eq!(
        got.reasons[1],
        Reason::ReworkDetailMissing {
            kind: MarkerKind::Remix,
            side: Side::A,
        }
    );
}

// ---- The owner's label decisions (2026-10-02) --------------------------

#[test]
fn a_plain_edit_is_a_cut() {
    let name = parse_title("Glasswing (Edit)");
    assert_eq!(name.markers[0].kind, MarkerKind::Edit);
    assert_eq!(name.markers[0].kind.class(), VersionClass::Cut);
    assert_eq!(name.markers[0].detail, None);
}

#[test]
fn a_named_edit_is_a_bootleg_by_that_name_like_the_handle_form() {
    for (title, whose) in [
        ("Glasswing (Vey Sun Edit)", "Vey Sun"),
        ("Glasswing (@lumenlark edit)", "@lumenlark"),
        ("Glasswing (Vey Sun Re-Edit)", "Vey Sun"),
    ] {
        let name = parse_title(title);
        assert_eq!(name.markers[0].kind, MarkerKind::Bootleg, "{title}");
        assert_eq!(name.markers[0].detail.as_deref(), Some(whose), "{title}");
        assert_eq!(name.markers[0].kind.class(), VersionClass::Rework);
    }
}

#[test]
fn a_re_edit_without_a_name_is_a_bootleg() {
    let name = parse_title("Glasswing (Re-Edit)");
    assert_eq!(name.markers[0].kind, MarkerKind::Bootleg);
    assert_eq!(name.markers[0].detail, None);
}

#[test]
fn a_named_mix_is_a_remix_by_that_name() {
    let name = parse_title("Glasswing (Vey Sun Mix)");
    assert_eq!(name.markers[0].kind, MarkerKind::Remix);
    assert_eq!(name.markers[0].detail.as_deref(), Some("Vey Sun"));
}

#[test]
fn a_mix_named_after_a_cut_is_that_cut_not_a_remix() {
    for (title, kind) in [
        ("Glasswing (Original Mix)", MarkerKind::Original),
        ("Glasswing (Extended Mix)", MarkerKind::Extended),
        ("Glasswing (Radio Mix)", MarkerKind::RadioEdit),
        ("Glasswing (Club Mix)", MarkerKind::ClubEdit),
        ("Glasswing (Clean Mix)", MarkerKind::Clean),
    ] {
        let name = parse_title(title);
        assert_eq!(name.markers.len(), 1, "{title}");
        assert_eq!(name.markers[0].kind, kind, "{title}");
        assert_eq!(name.markers[0].detail, None, "{title}");
    }
}

#[test]
fn a_remaster_is_a_cut_with_or_without_its_year() {
    for title in [
        "Glasswing (Remaster)",
        "Glasswing (2019 Remaster)",
        "Glasswing (Remastered 2019)",
    ] {
        let name = parse_title(title);
        assert_eq!(name.markers.len(), 1, "{title}");
        assert_eq!(name.markers[0].kind, MarkerKind::Remaster, "{title}");
        assert_eq!(name.markers[0].kind.class(), VersionClass::Cut);
        assert!(name.unrecognized.is_empty(), "{title}");
    }
}

#[test]
fn instrumental_and_acapella_are_reworks_with_their_own_labels() {
    let instrumental = parse_title("Glasswing (Instrumental)");
    let acapella = parse_title("Glasswing (Acapella)");
    assert_eq!(instrumental.markers[0].kind, MarkerKind::Instrumental);
    assert_eq!(acapella.markers[0].kind, MarkerKind::Acapella);
    for name in [&instrumental, &acapella] {
        assert_eq!(name.markers[0].kind.class(), VersionClass::Rework);
        // Neither stands in for the full track.
        let got = compare(name, &parse_title("Glasswing"));
        assert_eq!(got.outcome, Outcome::DifferentRework);
    }
    assert_eq!(
        compare(&instrumental, &acapella).outcome,
        Outcome::DifferentRework
    );
}

#[test]
fn a_mix_or_edit_named_only_with_ordinary_words_is_not_recognized() {
    for title in [
        "Velo (Main Mix)",
        "Velo (Album Mix)",
        "Velo (Vocal Mix)",
        "Velo (12\" Mix)",
        "Velo (DJ Edit)",
        "Velo (Single Edit)",
        "Velo (Hype Edit)",
    ] {
        let name = parse_title(title);
        assert!(name.markers.is_empty(), "{title}: {:?}", name.markers);
        assert_eq!(name.unrecognized.len(), 1, "{title}");
        assert_eq!(name.base_title, "Velo", "{title}");
    }
}

#[test]
fn a_pair_that_differs_only_in_an_ordinary_word_mix_cant_be_told() {
    for (a, b) in [
        ("Velo (Main Mix)", "Velo"),
        ("Velo (DJ Edit)", "Velo (Edit)"),
        ("Velo (Single Edit)", "Velo (Radio Edit)"),
    ] {
        let got = compare(&parse_title(a), &parse_title(b));
        assert_eq!(got.outcome, Outcome::CantTell, "{a} / {b}");
    }
}

#[test]
fn two_different_remixers_are_two_different_reworks() {
    let got = compare(
        &parse_title("Korvex (Rivetta Remix)"),
        &parse_title("Korvex (Flint Remix)"),
    );
    assert_eq!(got.outcome, Outcome::DifferentRework);
}

// ---- Accents fold in the comparison only --------------------------------

#[test]
fn accents_and_sharp_s_dont_make_a_different_title() {
    for (a, b) in [
        ("Caf\u{e9} Vireo", "Cafe Vireo"),
        ("Stra\u{df}e", "STRASSE"),
        ("\u{130}stanbul", "istanbul"),
    ] {
        let got = compare(&parse_title(a), &parse_title(b));
        assert_eq!(got.outcome, Outcome::SameVersion, "{a} / {b}");
    }
}

#[test]
fn folding_accents_leaves_the_parsed_name_as_written() {
    let name = parse_title("Caf\u{e9} Stra\u{df}e (Extended Mix)");
    assert_eq!(name.original, "Caf\u{e9} Stra\u{df}e (Extended Mix)");
    assert_eq!(name.base_title, "Caf\u{e9} Stra\u{df}e");
}

// ---- The tables are the one place for a label ---------------------------

#[test]
fn every_kind_sits_in_exactly_one_label_table_and_takes_its_class_from_it() {
    use super::super::tables::{CUT_LABELS, REWORK_LABELS};
    for kind in MarkerKind::ALL {
        let in_cuts = CUT_LABELS.iter().filter(|(k, _)| *k == kind).count();
        let in_reworks = REWORK_LABELS.iter().filter(|(k, _)| *k == kind).count();
        assert_eq!(in_cuts + in_reworks, 1, "{}", kind.as_str());
        let expected = match in_cuts {
            1 => VersionClass::Cut,
            _ => VersionClass::Rework,
        };
        assert_eq!(kind.class(), expected, "{}", kind.as_str());
    }
}

// ---- A name with very many labels ----------------------------------------

#[test]
fn a_name_with_a_hundred_label_brackets_compares_as_cant_tell() {
    let long = format!("Velo{}", " (Remix)".repeat(100));
    let name = parse_title(&long);
    assert_eq!(name.original, long);
    let got = compare(&name, &name);
    assert_eq!(got.outcome, Outcome::CantTell);
    assert_eq!(
        got.reasons,
        [
            Reason::TooManyLabels { side: Side::A },
            Reason::TooManyLabels { side: Side::B },
        ]
    );
    let against_plain = compare(&parse_title("Velo"), &name);
    assert_eq!(against_plain.outcome, Outcome::CantTell);
    assert_eq!(
        against_plain.reasons,
        [Reason::TooManyLabels { side: Side::B }]
    );
}

#[test]
fn a_name_at_the_label_limit_is_still_compared() {
    let name = parse_title("Velo (Clean) (Intro) (Extended) (Live) (Dub) (VIP) (Cover) (Flip)");
    assert_eq!(compare(&name, &name).outcome, Outcome::SameVersion);
}

// ---- Reasons are data ---------------------------------------------------

#[test]
fn every_kind_is_a_cut_or_a_rework_as_section_1_5_lists_them() {
    let cuts = [
        MarkerKind::Original,
        MarkerKind::Extended,
        MarkerKind::RadioEdit,
        MarkerKind::ClubEdit,
        MarkerKind::Short,
        MarkerKind::Clean,
        MarkerKind::Dirty,
        MarkerKind::Intro,
        MarkerKind::Outro,
        MarkerKind::IntroOutro,
        MarkerKind::QuickHit,
        MarkerKind::Edit,
        MarkerKind::Remaster,
    ];
    for kind in MarkerKind::ALL {
        let expected = match cuts.contains(&kind) {
            true => VersionClass::Cut,
            false => VersionClass::Rework,
        };
        assert_eq!(kind.class(), expected, "{}", kind.as_str());
    }
}

#[test]
fn reason_codes_and_kind_names_are_plain_snake_case_keys() {
    let plain = |s: &str| s.chars().all(|c| c.is_ascii_lowercase() || c == '_');
    for kind in MarkerKind::ALL {
        assert!(plain(kind.as_str()), "{}", kind.as_str());
    }
    let reasons = [
        Reason::BaseTitlesMatch,
        Reason::BaseTitlesDiffer,
        Reason::EmptyTitle { side: Side::A },
        Reason::TooManyLabels { side: Side::A },
        Reason::TitleReadAsMarker {
            side: Side::A,
            text: String::new(),
        },
        Reason::MarkerReadAsTitle {
            side: Side::A,
            text: String::new(),
        },
        Reason::SameMarkers,
        Reason::CutDiffers {
            only_a: vec![],
            only_b: vec![],
        },
        Reason::ReworkDiffers {
            only_a: vec![],
            only_b: vec![],
        },
        Reason::ReworkDetailMissing {
            kind: MarkerKind::Remix,
            side: Side::A,
        },
        Reason::UnrecognizedDiffers {
            only_a: vec![],
            only_b: vec![],
        },
        Reason::ArtistsDiffer,
        Reason::CreditedToRemixer { side: Side::A },
        Reason::MashupPartMatch {
            side: Side::A,
            part: String::new(),
        },
    ];
    let mut codes: Vec<&str> = reasons.iter().map(Reason::code).collect();
    assert!(codes.iter().all(|c| plain(c)));
    codes.sort_unstable();
    codes.dedup();
    assert_eq!(codes.len(), reasons.len(), "codes are distinct");
}

#[test]
fn a_rework_reason_names_whose_rework_each_side_has() {
    let got = compare(
        &parse_title("Korvex (Rivetta Remix)"),
        &parse_title("Korvex (Flint Bootleg)"),
    );
    let Reason::ReworkDiffers { only_a, only_b } = &got.reasons[1] else {
        panic!("{:?}", got.reasons);
    };
    assert_eq!(only_a[0].kind, MarkerKind::Remix);
    assert_eq!(only_a[0].detail.as_deref(), Some("Rivetta"));
    assert_eq!(only_b[0].kind, MarkerKind::Bootleg);
    assert_eq!(only_b[0].detail.as_deref(), Some("Flint"));
}

// ---- Parser nits (1bA-15) ---------------------------------------------

#[test]
fn a_cut_name_followed_by_edit_is_one_cut() {
    let name = parse_title("Glasswing (Quick Hit Edit)");
    let kinds: Vec<_> = name.markers.iter().map(|m| m.kind).collect();
    assert_eq!(kinds, [MarkerKind::QuickHit]);
    assert_eq!(name.base_title, "Glasswing");
}

#[test]
fn a_cut_name_followed_by_edit_is_one_cut_in_a_dash_segment_and_a_bare_tail() {
    for input in ["Glasswing - Quick Hit Edit", "Glasswing Quick Hit Edit"] {
        let name = parse_title(input);
        let kinds: Vec<_> = name
            .markers
            .iter()
            .chain(&name.title_markers)
            .map(|m| m.kind)
            .collect();
        assert_eq!(kinds, [MarkerKind::QuickHit], "{input}");
    }
}

#[test]
fn an_edit_on_its_own_or_after_a_rework_still_counts_as_its_own_label() {
    let plain = parse_title("Glasswing (Edit)");
    assert_eq!(plain.markers[0].kind, MarkerKind::Edit);
    let both = parse_title("Glasswing (Instrumental Edit)");
    let kinds: Vec<_> = both.markers.iter().map(|m| m.kind).collect();
    assert_eq!(kinds, [MarkerKind::Instrumental, MarkerKind::Edit]);
}

#[test]
fn an_artist_ending_in_a_lone_x_stays_one_artist() {
    let name = parse_file_name("Odalys X - Glasswing.mp3");
    let credits: Vec<_> = name.credits.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(credits, ["Odalys X"]);
    assert_eq!(name.base_title, "Glasswing");
}

#[test]
fn an_x_between_two_artists_still_splits_them() {
    let name = parse_file_name("Odalys X Nemora Vale - Glasswing.mp3");
    let credits: Vec<_> = name.credits.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(credits, ["Odalys", "Nemora Vale"]);
}

#[test]
fn a_bare_bitrate_at_the_end_is_junk_like_the_bracketed_one() {
    for (input, junk) in [
        ("Glasswing 320kbps", "320kbps"),
        ("Glasswing 192 kbps", "192 kbps"),
        ("Glasswing 128k", "128k"),
        ("Glasswing 256KBPS", "256KBPS"),
        ("Glasswing (Extended Mix) 320kbps", "320kbps"),
        ("Odalys Vane - Glasswing - 320kbps", "320kbps"),
    ] {
        let name = parse_title(input);
        assert_eq!(name.junk.len(), 1, "{input}");
        assert_eq!(name.junk[0].kind, JunkKind::RipTag, "{input}");
        assert_eq!(name.junk[0].text, junk, "{input}");
        assert!(!name.base_title.to_lowercase().contains("kbps"), "{input}");
    }
    let file = parse_file_name("Odalys Vane - Glasswing 320kbps.mp3");
    assert_eq!(file.base_title, "Glasswing");
    assert_eq!(file.junk.len(), 2);
}

#[test]
fn a_bare_number_or_a_name_that_is_only_a_bitrate_is_not_junk() {
    assert!(parse_title("Glasswing 320").junk.is_empty());
    assert_eq!(parse_title("Glasswing 320").base_title, "Glasswing 320");
    let alone = parse_title("320kbps");
    assert!(alone.junk.is_empty());
    assert_eq!(alone.base_title, "320kbps");
}

#[test]
fn a_file_name_that_is_an_artist_and_a_bitrate_leaves_the_bitrate_as_the_title_not_the_artist() {
    let name = parse_file_name("Odalys Vane - 320kbps.mp3");
    assert!(name.junk.iter().all(|j| j.kind != JunkKind::RipTag));
    assert_eq!(name.credits[0].name, "Odalys Vane");
    assert_eq!(
        name.base_title, "320kbps",
        "the artist is not made the title"
    );
}
