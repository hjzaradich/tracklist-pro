//! Pairs of names, what they say about each other, and why (the reason
//! codes, in order).

use super::super::{compare, Outcome};
use super::{parsed, Src};
use Outcome::{CantTell, DifferentBase, DifferentCut, DifferentRework, SameVersion};
use Src::{F, T};

type Pair = (Src, &'static str, Src, &'static str, Outcome, &'static str);

// One pair per row.
#[rustfmt::skip]
const PAIRS: &[Pair] = &[
    // ---- Same base, same version ------------------------------------------
    (T, "Paper Harbor", T, "Paper Harbor", SameVersion, "base_titles_match,same_markers"),
    (T, "PAPER HARBOR", T, "paper harbor", SameVersion, "base_titles_match,same_markers"),
    (T, "Paper Harbor", T, "Paper  Harbor!", SameVersion, "base_titles_match,same_markers"),
    (T, "Nox & Vey", T, "Nox and Vey", SameVersion, "base_titles_match,same_markers"),
    (T, "Cafe\u{301} Vireo", T, "Caf\u{e9} Vireo", SameVersion, "base_titles_match,same_markers"),
    // Accents and spelled-out letters: stores and file names drop them.
    (T, "Caf\u{e9} Vireo", T, "Cafe Vireo", SameVersion, "base_titles_match,same_markers"),
    (T, "Stra\u{df}e", T, "STRASSE", SameVersion, "base_titles_match,same_markers"),
    (T, "\u{130}stanbul", T, "istanbul", SameVersion, "base_titles_match,same_markers"),
    (T, "S\u{f8}lvane \u{c6}ther", T, "Solvane Aether", SameVersion, "base_titles_match,same_markers"),
    // A Japanese voicing mark makes a different letter: not folded.
    (T, "\u{304B}", T, "\u{304C}", DifferentBase, "base_titles_differ"),
    (T, "Can't Wait", T, "Can\u{2019}t Wait", SameVersion, "base_titles_match,same_markers"),
    (T, "Paper Harbor (Original Mix)", T, "Paper Harbor", SameVersion, "base_titles_match,same_markers"),
    (T, "Paper Harbor (Extended Mix)", T, "Paper Harbor [Extended Version]", SameVersion, "base_titles_match,same_markers"),
    (T, "Paper Harbor (Extended Mix)", T, "Paper Harbor - Extended Mix", SameVersion, "base_titles_match,same_markers"),
    (T, "Paper Harbor (Quill Ashby Remix)", T, "Paper Harbor (quill ashby rmx)", SameVersion, "base_titles_match,same_markers"),
    (T, "Paper Harbor (Remix by Quill Ashby)", T, "Paper Harbor (Quill Ashby's Remix)", SameVersion, "base_titles_match,same_markers"),
    (T, "Paper Harbor (Intro) (Outro)", T, "Paper Harbor (Intro-Outro)", SameVersion, "base_titles_match,same_markers"),
    (T, "Paper Harbor (Clean) (Quill Ashby Remix)", T, "Paper Harbor (Quill Ashby Remix) [Clean]", SameVersion, "base_titles_match,same_markers"),
    (T, "Neon Moth (Explicit)", T, "Neon Moth (Dirty)", SameVersion, "base_titles_match,same_markers"),
    (T, "Paper Harbor (@lumenlark edit)", T, "Paper Harbor [@Lumenlark Edit]", SameVersion, "base_titles_match,same_markers"),
    (T, "Paper Harbor feat. Vey Sun", T, "Paper Harbor", SameVersion, "base_titles_match,same_markers"),
    (T, "(Intro)", T, "Intro", SameVersion, "base_titles_match,same_markers"),
    // Junk around a name changes nothing.
    (F, "10000001_Orbit_Line_&#40;Original Mix&#41;.mp3", F, "Halden Rook/Orbit Line (Original Mix).flac", SameVersion, "base_titles_match,same_markers"),
    (F, "Downloads/RipSite.example - Nemora Vale - Glasswing.mp3", F, "Nemora Vale/Lanterns EP/01 Glasswing.flac", SameVersion, "base_titles_match,same_markers"),
    (F, "Downloads/Nemora Vale - Glasswing (320).mp3", F, "AIFF/Nemora Vale - Glasswing.aiff", SameVersion, "base_titles_match,same_markers"),
    (F, "Downloads/Orbit Line [ ripper.example ].mp3", T, "Orbit Line (Original Mix)", SameVersion, "base_titles_match,same_markers"),
    (T, "Glasswing (Free Download)", T, "Glasswing", SameVersion, "base_titles_match,same_markers"),
    (F, "Kestrel Nine - Paper Harbor.flac", T, "Paper Harbor", SameVersion, "base_titles_match,same_markers"),
    // Length decides nothing: a running time is junk.
    (T, "Glasswing (3:45)", T, "Glasswing (7:10)", SameVersion, "base_titles_match,same_markers"),
    // ---- Same base, different cut -----------------------------------------
    (T, "Paper Harbor", T, "Paper Harbor (Extended Mix)", DifferentCut, "base_titles_match,cut_differs"),
    (T, "Paper Harbor (Original Mix)", T, "Paper Harbor (Extended Mix)", DifferentCut, "base_titles_match,cut_differs"),
    (T, "Paper Harbor (Extended Mix)", T, "Paper Harbor (Radio Edit)", DifferentCut, "base_titles_match,cut_differs"),
    (T, "Paper Harbor (Short Edit)", T, "Paper Harbor (Extended Mix)", DifferentCut, "base_titles_match,cut_differs"),
    (T, "Neon Moth (Clean)", T, "Neon Moth (Dirty)", DifferentCut, "base_titles_match,cut_differs"),
    (T, "Neon Moth (Clean)", T, "Neon Moth (Clean) (Intro)", DifferentCut, "base_titles_match,cut_differs"),
    (T, "Neon Moth (Intro)", T, "Neon Moth (Intro-Outro)", DifferentCut, "base_titles_match,cut_differs"),
    (T, "Neon Moth (Quick Hit)", T, "Neon Moth", DifferentCut, "base_titles_match,cut_differs"),
    (T, "Neon Moth (Club Mix)", T, "Neon Moth (Radio Mix)", DifferentCut, "base_titles_match,cut_differs"),
    (T, "Velo (dirty)", T, "Velo", DifferentCut, "base_titles_match,cut_differs"),
    (T, "Paper Harbor (Quill Ashby Remix)", T, "Paper Harbor (Quill Ashby Remix) (Extended)", DifferentCut, "base_titles_match,cut_differs"),
    (F, "Solvane/Neon Moth (Dirty).mp3", F, "Solvane/Neon Moth (Clean).mp3", DifferentCut, "base_titles_match,cut_differs"),
    (T, "Glasswing (Edit)", T, "Glasswing", DifferentCut, "base_titles_match,cut_differs"),
    (T, "Glasswing (Edit)", T, "Glasswing (Radio Edit)", DifferentCut, "base_titles_match,cut_differs"),
    (T, "Glasswing (2019 Remaster)", T, "Glasswing", DifferentCut, "base_titles_match,cut_differs"),
    (T, "Glasswing (2019 Remaster)", T, "Glasswing (Remastered)", SameVersion, "base_titles_match,same_markers"),
    // A version word can be the title: the other name settles it.
    (T, "Velo (dirty)", T, "Velo (dirty) (dirty)", DifferentCut, "base_titles_match,marker_read_as_title,cut_differs"),
    (T, "Velo (dirty) (dirty)", T, "Velo (dirty)", DifferentCut, "base_titles_match,marker_read_as_title,cut_differs"),
    (T, "Come Clean", T, "Come Clean (Clean)", DifferentCut, "base_titles_match,cut_differs"),
    (T, "Paper Harbor Extended Mix", T, "Paper Harbor (Extended Mix)", SameVersion, "base_titles_match,marker_read_as_title,same_markers"),
    (T, "Paper Harbor Extended Mix", T, "Paper Harbor Extended Mix", SameVersion, "base_titles_match,same_markers"),
    (T, "Come Clean", T, "Come (Clean)", SameVersion, "base_titles_match,marker_read_as_title,same_markers"),
    // ---- Same base, different rework --------------------------------------
    (T, "Paper Harbor (Quill Ashby Remix)", T, "Paper Harbor", DifferentRework, "base_titles_match,rework_differs"),
    (T, "Paper Harbor (Quill Ashby Remix)", T, "Paper Harbor (Extended Mix)", DifferentRework, "base_titles_match,rework_differs,cut_differs"),
    (T, "Korvex (Rivetta Remix)", T, "Korvex (Flint Remix)", DifferentRework, "base_titles_match,rework_differs"),
    (T, "Korvex (Rivetta Remix)", T, "Korvex (Flint Bootleg)", DifferentRework, "base_titles_match,rework_differs"),
    (T, "Korvex (Rivetta Remix) (Clean)", T, "Korvex (Flint Remix) (Dirty)", DifferentRework, "base_titles_match,rework_differs,cut_differs"),
    (T, "Paper Harbor (VIP)", T, "Paper Harbor", DifferentRework, "base_titles_match,rework_differs"),
    (T, "Paper Harbor (Cover)", T, "Paper Harbor", DifferentRework, "base_titles_match,rework_differs"),
    (T, "Paper Harbor (Live)", T, "Paper Harbor (Dub)", DifferentRework, "base_titles_match,rework_differs"),
    (T, "Paper Harbor (Alternate Mix)", T, "Paper Harbor (Original Mix)", DifferentRework, "base_titles_match,rework_differs"),
    (T, "Paper Harbor [Vey Sun Flip]", T, "Paper Harbor (@lumenlark edit)", DifferentRework, "base_titles_match,rework_differs"),
    (T, "Paper Harbor (Rivetta Remix) (Flint Remix)", T, "Paper Harbor (Rivetta Remix)", DifferentRework, "base_titles_match,rework_differs"),
    (T, "Paper Harbor (Quill Ashby Remix) (Extended)", T, "Paper Harbor (Extended Mix)", DifferentRework, "base_titles_match,rework_differs"),
    // Owner's decisions (2026-10-02): edits, named mixes, remasters,
    // instrumentals and acapellas.
    (T, "Glasswing (Instrumental)", T, "Glasswing", DifferentRework, "base_titles_match,rework_differs"),
    (T, "Glasswing (Acapella)", T, "Glasswing", DifferentRework, "base_titles_match,rework_differs"),
    (T, "Glasswing (Acapella)", T, "Glasswing (Instrumental)", DifferentRework, "base_titles_match,rework_differs"),
    (T, "Glasswing (Vey Sun Edit)", T, "Glasswing", DifferentRework, "base_titles_match,rework_differs"),
    (T, "Glasswing (Vey Sun Edit)", T, "Glasswing (@lumenlark edit)", DifferentRework, "base_titles_match,rework_differs"),
    (T, "Glasswing (Re-Edit)", T, "Glasswing", DifferentRework, "base_titles_match,rework_differs"),
    (T, "Glasswing (Vey Sun Mix)", T, "Glasswing (Vey Sun Remix)", SameVersion, "base_titles_match,same_markers"),
    (T, "Glasswing (Vey Sun Mix)", T, "Glasswing (Extended Mix)", DifferentRework, "base_titles_match,rework_differs,cut_differs"),
    // A remix filed under the remixer alone.
    (F, "Quill Ashby - Paper Harbor (Quill Ashby Remix).mp3", F, "Kestrel Nine - Paper Harbor (Quill Ashby Remix).flac", SameVersion, "base_titles_match,credited_to_remixer,same_markers"),
    (F, "Quill Ashby - Paper Harbor (Quill Ashby Remix).mp3", F, "Kestrel Nine - Paper Harbor.flac", DifferentRework, "base_titles_match,rework_differs,credited_to_remixer"),
    (F, "Kestrel Nine - Paper Harbor.flac", F, "Vey - Paper Harbor (Nox & Vey Remix).mp3", DifferentRework, "base_titles_match,rework_differs,credited_to_remixer"),
    // ---- Different base ----------------------------------------------------
    (T, "Paper Harbor", T, "Neon Moth", DifferentBase, "base_titles_differ"),
    (T, "Paper Harbor (Extended Mix)", T, "Neon Moth (Extended Mix)", DifferentBase, "base_titles_differ"),
    (T, "Paper Harbor", T, "Paper Harbour", DifferentBase, "base_titles_differ"),
    (T, "Intro", T, "Paper Harbor (Intro)", DifferentBase, "base_titles_differ"),
    (T, "Duskline x Emberlow", T, "Korvex", DifferentBase, "base_titles_differ"),
    (T, "Glasswing", T, "Glasswing - Reprise", DifferentBase, "base_titles_differ"),
    // ---- Can't tell --------------------------------------------------------
    (T, "", T, "Paper Harbor", CantTell, "empty_title"),
    (T, "", T, "", CantTell, "empty_title,empty_title"),
    // Bare label words, and nothing in the other name to back them.
    (T, "Paper Harbor Extended Mix", T, "Paper Harbor", CantTell, "base_titles_match,title_read_as_marker,cut_differs"),
    (T, "Come Clean", T, "Come", CantTell, "base_titles_match,title_read_as_marker,cut_differs"),
    (T, "Paper Harbor Remix", T, "Paper Harbor", CantTell, "base_titles_match,title_read_as_marker,rework_differs"),
    // A rework, but whose?
    (T, "Paper Harbor (Remix)", T, "Paper Harbor (Quill Ashby Remix)", CantTell, "base_titles_match,rework_detail_missing"),
    (T, "Paper Harbor (Live)", T, "Paper Harbor (Live at Harbor Hall)", CantTell, "base_titles_match,rework_detail_missing"),
    (T, "Paper Harbor (Kestrel Nine VIP)", T, "Paper Harbor (VIP)", CantTell, "base_titles_match,rework_detail_missing"),
    // Ordinary words in front of "Mix" or "Edit" are no one's name.
    (T, "Velo (Main Mix)", T, "Velo", CantTell, "base_titles_match,unrecognized_differs"),
    (T, "Velo (Album Mix)", T, "Velo", CantTell, "base_titles_match,unrecognized_differs"),
    (T, "Velo (Vocal Mix)", T, "Velo (Original Mix)", CantTell, "base_titles_match,unrecognized_differs"),
    (T, "Velo (12\" Mix)", T, "Velo", CantTell, "base_titles_match,unrecognized_differs"),
    (T, "Velo (DJ Edit)", T, "Velo (Edit)", CantTell, "base_titles_match,cut_differs,unrecognized_differs"),
    (T, "Velo (Single Edit)", T, "Velo (Radio Edit)", CantTell, "base_titles_match,cut_differs,unrecognized_differs"),
    (T, "Velo (Hype Edit)", T, "Velo", CantTell, "base_titles_match,unrecognized_differs"),
    (T, "Velo (Extended DJ Edit)", T, "Velo (Extended)", CantTell, "base_titles_match,cut_differs,unrecognized_differs"),
    // More label groups than can be read every way.
    (T, "Velo (Clean) (Intro) (Extended) (Live) (Dub) (VIP) (Cover) (Flip) (Bootleg)", T, "Velo", CantTell, "too_many_labels"),
    // Brackets nobody understands.
    (T, "Glasswing (Part 2)", T, "Glasswing", CantTell, "base_titles_match,unrecognized_differs"),
    (T, "Glasswing (Part 2)", T, "Glasswing (Part 3)", CantTell, "base_titles_match,unrecognized_differs"),
    (T, "Glasswing (Skyline Session)", T, "Glasswing (Radio Edit)", CantTell, "base_titles_match,cut_differs,unrecognized_differs"),
    (T, "Glasswing (Part 2)", T, "Glasswing [part 2]", SameVersion, "base_titles_match,same_markers"),
    // The same title under different artists can be a different song.
    (F, "Kestrel Nine - Paper Harbor.flac", F, "The Marrow Choir - Paper Harbor.m4a", CantTell, "base_titles_match,artists_differ"),
    (F, "Rivetta - Korvex (Extended Mix).mp3", F, "Flint - Korvex (Extended Mix).mp3", CantTell, "base_titles_match,artists_differ"),
    (F, "Kestrel Nine - Paper Harbor (Quill Ashby Remix).flac", F, "The Marrow Choir - Paper Harbor (Quill Ashby Remix).mp3", CantTell, "base_titles_match,artists_differ"),
    (F, "Kestrel Nine & Solvane - Paper Harbor.flac", F, "Solvane - Paper Harbor.mp3", SameVersion, "base_titles_match,same_markers"),
    // A mashup's part is a hint, never a link.
    (T, "Paper Harbor x Neon Moth (Vey Sun Mashup)", T, "Paper Harbor", CantTell, "base_titles_differ,mashup_part_match"),
    (T, "Neon Moth (Dirty)", T, "Paper Harbor x Neon Moth (Vey Sun Mashup)", CantTell, "base_titles_differ,mashup_part_match"),
    (T, "Duskline x Emberlow", T, "Emberlow", CantTell, "base_titles_differ,mashup_part_match"),
    (T, "Duskline x Emberlow", T, "Duskline x Korvex", CantTell, "base_titles_differ,mashup_part_match,mashup_part_match"),
    // A mashup named with and without its label.
    (T, "Duskline x Emberlow", T, "Duskline x Emberlow (Mashup)", SameVersion, "base_titles_match,same_markers"),
    (T, "Duskline x Emberlow", T, "Duskline x Emberlow (Vey Sun Mashup)", CantTell, "base_titles_match,rework_detail_missing"),
];

fn codes(reasons: &[super::super::Reason]) -> String {
    reasons
        .iter()
        .map(|r| r.code())
        .collect::<Vec<_>>()
        .join(",")
}

#[test]
fn every_pair_compares_as_the_table_says() {
    let mut wrong = Vec::new();
    for &(src_a, a, src_b, b, outcome, reasons) in PAIRS {
        let got = compare(&parsed(src_a, a), &parsed(src_b, b));
        if got.outcome != outcome || codes(&got.reasons) != reasons {
            wrong.push(format!(
                "{a:?} / {b:?}: expected {outcome:?} [{reasons}], got {:?} [{}]",
                got.outcome,
                codes(&got.reasons)
            ));
        }
    }
    assert!(
        wrong.is_empty(),
        "{} wrong:\n{}",
        wrong.len(),
        wrong.join("\n")
    );
}

#[test]
fn the_outcome_is_the_same_whichever_name_comes_first() {
    for &(src_a, a, src_b, b, _, _) in PAIRS {
        let (a, b) = (parsed(src_a, a), parsed(src_b, b));
        assert_eq!(
            compare(&a, &b).outcome,
            compare(&b, &a).outcome,
            "{:?} / {:?}",
            a.original,
            b.original
        );
    }
}

#[test]
fn a_name_compared_with_itself_is_the_same_version() {
    for case in super::corpus::CASES {
        let name = parsed(case.src, case.input);
        if name.base_title.is_empty() {
            continue;
        }
        let got = compare(&name, &name);
        assert_eq!(
            got.outcome, SameVersion,
            "{:?}: {:?}",
            case.input, got.reasons
        );
    }
}

#[test]
fn every_outcome_and_every_reason_code_is_covered_by_the_table() {
    for outcome in [
        SameVersion,
        DifferentCut,
        DifferentRework,
        DifferentBase,
        CantTell,
    ] {
        assert!(PAIRS.iter().any(|p| p.4 == outcome), "{outcome:?}");
    }
    for code in [
        "base_titles_match",
        "base_titles_differ",
        "empty_title",
        "too_many_labels",
        "title_read_as_marker",
        "marker_read_as_title",
        "same_markers",
        "cut_differs",
        "rework_differs",
        "rework_detail_missing",
        "unrecognized_differs",
        "artists_differ",
        "credited_to_remixer",
        "mashup_part_match",
    ] {
        assert!(
            PAIRS.iter().any(|p| p.5.split(',').any(|c| c == code)),
            "{code}"
        );
    }
}
