//! Every label spelling (1bA-7, 1bA-8), read in every bracket style, after
//! a " - ", in a file name, and in upper and lower case.
//!
//! The list is written out by hand, apart from the module's tables, so a
//! wrong table row can't agree with itself. A test checks that every table
//! spelling has a row here.

use super::super::tables::{CUT_LABELS, REWORK_LABELS};
use super::{parsed, show_markers, Src};

/// A spelling as it would be written, and the markers it must give.
const SPELLINGS: &[(&str, &str)] = &[
    // Original
    ("Original", "original"),
    ("Original Mix", "original"),
    ("Original Version", "original"),
    ("Original Edit", "original"),
    ("Orig", "original"),
    ("Orig Mix", "original"),
    ("Orig. Mix", "original"),
    ("OG", "original"),
    ("OG Mix", "original"),
    ("OG Version", "original"),
    // Extended
    ("Extended", "extended"),
    ("Extended Mix", "extended"),
    ("Extended Version", "extended"),
    ("Extended Edit", "extended"),
    ("Extended Cut", "extended"),
    ("Ext", "extended"),
    ("Ext.", "extended"),
    ("Ext Mix", "extended"),
    ("Ext. Mix", "extended"),
    ("Ext Version", "extended"),
    ("Extd", "extended"),
    ("Xtd", "extended"),
    // Radio Edit
    ("Radio Edit", "radio_edit"),
    ("Radio Mix", "radio_edit"),
    ("Radio Version", "radio_edit"),
    ("Radio Cut", "radio_edit"),
    ("Radio", "radio_edit"),
    // Club Edit
    ("Club Edit", "club_edit"),
    ("Club Mix", "club_edit"),
    ("Club Version", "club_edit"),
    ("Club Cut", "club_edit"),
    ("Club", "club_edit"),
    // Short
    ("Short", "short"),
    ("Short Edit", "short"),
    ("Short Mix", "short"),
    ("Short Version", "short"),
    ("Short Cut", "short"),
    // Clean
    ("Clean", "clean"),
    ("Clean Edit", "clean"),
    ("Clean Mix", "clean"),
    ("Clean Version", "clean"),
    ("Clean Cut", "clean"),
    ("Cln", "clean"),
    // Dirty
    ("Dirty", "dirty"),
    ("Dirty Edit", "dirty"),
    ("Dirty Mix", "dirty"),
    ("Dirty Version", "dirty"),
    ("Dirty Cut", "dirty"),
    ("Explicit", "dirty"),
    ("Explicit Version", "dirty"),
    ("Drt", "dirty"),
    // Intro
    ("Intro", "intro"),
    ("Intro Edit", "intro"),
    ("Intro Mix", "intro"),
    ("Intro Version", "intro"),
    ("DJ Intro", "intro"),
    ("With Intro", "intro"),
    // Outro
    ("Outro", "outro"),
    ("Outro Edit", "outro"),
    ("Outro Mix", "outro"),
    ("Outro Version", "outro"),
    ("DJ Outro", "outro"),
    ("With Outro", "outro"),
    // Intro-Outro
    ("Intro-Outro", "intro_outro"),
    ("Intro Outro", "intro_outro"),
    ("Intro-Outro Edit", "intro_outro"),
    ("Intro Outro Edit", "intro_outro"),
    ("In-Out", "intro_outro"),
    ("In Out", "intro_outro"),
    ("DJ Intro Outro", "intro_outro"),
    ("Intro / Outro", "intro_outro"),
    ("Intro + Outro", "intro_outro"),
    // Quick Hit
    ("Quick Hit", "quick_hit"),
    ("Quick-Hit", "quick_hit"),
    ("QuickHit", "quick_hit"),
    ("Quick Hitter", "quick_hit"),
    ("QH", "quick_hit"),
    // Edit (plain)
    ("Edit", "edit"),
    // Remaster
    ("Remaster", "remaster"),
    ("Remastered", "remaster"),
    ("Remastered Version", "remaster"),
    ("Remaster Version", "remaster"),
    ("2019 Remaster", "remaster"),
    ("Remastered 2019", "remaster"),
    // Record-pool combinations
    ("Intro - Clean", "intro | clean"),
    ("Intro - Dirty", "intro | dirty"),
    ("Clean Intro", "clean | intro"),
    ("Dirty Intro", "dirty | intro"),
    ("Clean Extended", "clean | extended"),
    ("Dirty Extended Mix", "dirty | extended"),
    ("Clean - Short Edit", "clean | short"),
    ("Quick Hit Clean", "quick_hit | clean"),
    ("Quick Hit - Dirty", "quick_hit | dirty"),
    ("Clean / Radio Edit", "clean | radio_edit"),
    ("Intro Outro Clean", "intro_outro | clean"),
    ("Club Mix Clean", "club_edit | clean"),
    // Remix
    ("Remix", "remix"),
    ("RMX", "remix"),
    ("Remixed", "remix"),
    ("Re-Mix", "remix"),
    ("Remix Edit", "remix"),
    ("Quill Ashby Remix", "remix=Quill Ashby"),
    ("Quill Ashby Rmx", "remix=Quill Ashby"),
    ("Quill Ashby Re-Mix", "remix=Quill Ashby"),
    ("Quill Ashby Remix Edit", "remix=Quill Ashby"),
    ("Remix by Quill Ashby", "remix=Quill Ashby"),
    ("Remixed by Quill Ashby", "remix=Quill Ashby"),
    ("Vey Sun Mix", "remix=Vey Sun"),
    ("Quill Ashby Radio Remix", "radio_edit | remix=Quill Ashby"),
    ("Quill Ashby Club Remix", "club_edit | remix=Quill Ashby"),
    // VIP
    ("VIP", "vip"),
    ("V.I.P.", "vip"),
    ("VIP Mix", "vip"),
    ("VIP Edit", "vip"),
    ("VIP Version", "vip"),
    ("Kestrel Nine VIP", "vip=Kestrel Nine"),
    ("Kestrel Nine VIP Mix", "vip=Kestrel Nine"),
    // Flip
    ("Flip", "flip"),
    ("Flipped", "flip"),
    ("Vey Sun Flip", "flip=Vey Sun"),
    ("Flipped by Vey Sun", "flip=Vey Sun"),
    // Bootleg
    ("Bootleg", "bootleg"),
    ("Bootleg Mix", "bootleg"),
    ("Bootleg Edit", "bootleg"),
    ("Boot", "bootleg"),
    ("Booty", "bootleg"),
    ("Flint Bootleg", "bootleg=Flint"),
    ("Flint Bootleg Mix", "bootleg=Flint"),
    ("Bootleg by Flint", "bootleg=Flint"),
    ("Re-Edit", "bootleg"),
    ("Vey Sun Edit", "bootleg=Vey Sun"),
    ("Vey Sun Re-Edit", "bootleg=Vey Sun"),
    ("@lumenlark edit", "bootleg=@lumenlark"),
    ("@lumenlark re-edit", "bootleg=@lumenlark"),
    ("@lumenlark reedit", "bootleg=@lumenlark"),
    ("@lumenlark bootleg", "bootleg=@lumenlark"),
    // Rework, refix, reboot
    ("Rework", "rework"),
    ("Reworked", "rework"),
    ("Refix", "rework"),
    ("Re-Fix", "rework"),
    ("Reboot", "rework"),
    ("Re-Boot", "rework"),
    ("Flint Rework", "rework=Flint"),
    ("Flint Refix", "rework=Flint"),
    ("Flint Reboot", "rework=Flint"),
    ("Reworked by Flint", "rework=Flint"),
    // Dub
    ("Dub", "dub"),
    ("Dub Mix", "dub"),
    ("Dub Version", "dub"),
    ("Dub Edit", "dub"),
    ("Flint Dub", "dub=Flint"),
    ("Flint Dub Mix", "dub=Flint"),
    // Cover
    ("Cover", "cover"),
    ("Cover Version", "cover"),
    ("Covered", "cover"),
    ("The Marrow Choir Cover", "cover=The Marrow Choir"),
    ("Cover by The Marrow Choir", "cover=The Marrow Choir"),
    ("Covered by The Marrow Choir", "cover=The Marrow Choir"),
    // Live
    ("Live", "live"),
    ("Live Version", "live"),
    ("Live Mix", "live"),
    ("Live Edit", "live"),
    ("Live Recording", "live"),
    ("Live at Harbor Hall", "live=Harbor Hall"),
    ("Live from Harbor Hall", "live=Harbor Hall"),
    ("Live in Port Ellery", "live=Port Ellery"),
    // Alternate
    ("Alternate", "alternate"),
    ("Alternate Mix", "alternate"),
    ("Alternate Version", "alternate"),
    ("Alternate Take", "alternate"),
    ("Alternate Arrangement", "alternate"),
    ("Alternative", "alternate"),
    ("Alternative Mix", "alternate"),
    ("Alternative Version", "alternate"),
    ("Alt", "alternate"),
    ("Alt Mix", "alternate"),
    ("Alt. Mix", "alternate"),
    ("Alt Version", "alternate"),
    ("Alt Take", "alternate"),
    // Instrumental
    ("Instrumental", "instrumental"),
    ("Instrumental Mix", "instrumental"),
    ("Instrumental Version", "instrumental"),
    ("Inst", "instrumental"),
    ("Instr", "instrumental"),
    (
        "Quill Ashby Remix Instrumental",
        "remix=Quill Ashby | instrumental=Quill Ashby",
    ),
    // Acapella
    ("Acapella", "acapella"),
    ("Acappella", "acapella"),
    ("Accapella", "acapella"),
    ("A Cappella", "acapella"),
    ("Acapella Version", "acapella"),
    ("Acap", "acapella"),
    ("Acapella - Clean", "acapella | clean"),
    // Mashup
    ("Mashup", "mashup"),
    ("Mash-Up", "mashup"),
    ("Mash Up", "mashup"),
    ("Vey Sun Mashup", "mashup=Vey Sun"),
    ("Vey Sun Mash-Up", "mashup=Vey Sun"),
];

/// How a marker can be wrapped. `%` is the spelling.
const BRACKET_STYLES: &[&str] = &[
    "Paper Harbor (%)",
    "Paper Harbor [%]",
    "Paper Harbor {%}",
    "Paper Harbor \u{FF08}%\u{FF09}",
    "Paper Harbor \u{3010}%\u{3011}",
    "Paper Harbor \u{FF3B}%\u{FF3D}",
    "Paper Harbor (%",
    "Paper Harbor ( % )",
    "Paper Harbor (feat. Vey Sun) (%)",
    "Paper Harbor (%) (Free Download)",
];

fn wrap(style: &str, spelling: &str) -> String {
    style.replace('%', spelling)
}

/// Collects what's wrong instead of stopping at the first, so one run
/// shows everything.
fn check(wrong: &mut Vec<String>, src: Src, input: &str, base: &str, markers: &str) {
    let parsed = parsed(src, input);
    let got = show_markers(&parsed.markers);
    if parsed.base_title != base || got != markers {
        wrong.push(format!(
            "{input:?}: expected {base:?} [{markers}], got {:?} [{got}]",
            parsed.base_title
        ));
    }
    if !parsed.unrecognized.is_empty() {
        wrong.push(format!(
            "{input:?}: not understood: {:?}",
            parsed.unrecognized
        ));
    }
}

fn assert_none(wrong: Vec<String>) {
    assert!(
        wrong.is_empty(),
        "{} wrong:\n{}",
        wrong.len(),
        wrong.join("\n")
    );
}

#[test]
fn every_spelling_is_read_in_every_bracket_style() {
    let mut wrong = Vec::new();
    for (spelling, markers) in SPELLINGS {
        for style in BRACKET_STYLES {
            check(
                &mut wrong,
                Src::T,
                &wrap(style, spelling),
                "Paper Harbor",
                markers,
            );
        }
    }
    assert_none(wrong);
}

#[test]
fn every_spelling_is_read_in_a_file_name_with_an_artist() {
    let mut wrong = Vec::new();
    for (spelling, markers) in SPELLINGS {
        // A file name can't hold a slash: it would be a folder.
        if spelling.contains('/') {
            continue;
        }
        let input = format!("Kestrel Nine - Paper Harbor ({spelling}).mp3");
        check(&mut wrong, Src::F, &input, "Paper Harbor", markers);
        let underscored = input.replace(' ', "_");
        // A name's spaces become underscores too, and come back as spaces.
        check(&mut wrong, Src::F, &underscored, "Paper Harbor", markers);
    }
    assert_none(wrong);
}

#[test]
fn every_spelling_is_read_in_upper_and_lower_case() {
    let mut wrong = Vec::new();
    for (spelling, markers) in SPELLINGS {
        // A name in the spelling changes case with it; compare kinds only.
        let kinds_only = |shown: &str| -> String {
            shown
                .split(" | ")
                .map(|m| m.split('=').next().unwrap_or(m))
                .collect::<Vec<_>>()
                .join(" | ")
        };
        for cased in [spelling.to_uppercase(), spelling.to_lowercase()] {
            let input = format!("Paper Harbor ({cased})");
            let parsed = parsed(Src::T, &input);
            let got = kinds_only(&show_markers(&parsed.markers));
            if got != kinds_only(markers) || parsed.base_title != "Paper Harbor" {
                wrong.push(format!("{input:?}: got {:?} [{got}]", parsed.base_title));
            }
        }
    }
    assert_none(wrong);
}

#[test]
fn every_spelling_after_a_dash_is_a_marker_flagged_as_doubtful() {
    let mut wrong = Vec::new();
    for (spelling, markers) in SPELLINGS {
        // Spellings with their own " - " would split into more segments.
        if spelling.contains(" - ") {
            continue;
        }
        let doubtful = markers
            .split(" | ")
            .map(|m| format!("{m}?"))
            .collect::<Vec<_>>()
            .join(" | ");
        let input = format!("Paper Harbor - {spelling}");
        check(&mut wrong, Src::T, &input, "Paper Harbor", &doubtful);
    }
    assert_none(wrong);
}

#[test]
fn every_spelling_without_a_name_and_without_brackets_stays_in_the_title() {
    let mut wrong = Vec::new();
    for (spelling, markers) in SPELLINGS {
        // A name in front can't be told from the title; joining words
        // ("by", "at") need a bracket or a dash.
        if markers.contains('=') || spelling.contains(" - ") || spelling.contains("2019") {
            continue;
        }
        let input = format!("Paper Harbor {spelling}");
        let parsed = parsed(Src::T, &input);
        let doubtful = markers
            .split(" | ")
            .map(|m| format!("{m}?"))
            .collect::<Vec<_>>()
            .join(" | ");
        let got = show_markers(&parsed.title_markers);
        if parsed.base_title != input || !parsed.markers.is_empty() || got != doubtful {
            wrong.push(format!(
                "{input:?}: got {:?}, markers [{}], title markers [{got}]",
                parsed.base_title,
                show_markers(&parsed.markers)
            ));
        }
    }
    assert_none(wrong);
}

#[test]
fn every_table_spelling_has_a_row_in_this_list() {
    let listed: Vec<String> = SPELLINGS.iter().map(|(s, _)| s.to_lowercase()).collect();
    let mut missing = Vec::new();
    for (kind, spellings) in CUT_LABELS.iter().chain(REWORK_LABELS) {
        for spelling in *spellings {
            match SPELLINGS
                .iter()
                .find(|(listed, _)| listed.to_lowercase() == *spelling)
            {
                Some((_, markers)) if *markers == kind.as_str() => {}
                Some((_, markers)) => missing.push(format!("{spelling}: listed as {markers}")),
                None => missing.push(format!("{spelling}: not listed")),
            }
        }
    }
    assert!(listed.len() >= 150, "only {} spellings", listed.len());
    assert!(missing.is_empty(), "{}", missing.join("\n"));
}

#[test]
fn every_kind_of_section_1_5_has_a_spelling() {
    use super::super::MarkerKind;
    for kind in MarkerKind::ALL {
        assert!(
            SPELLINGS
                .iter()
                .any(|(_, markers)| *markers == kind.as_str()),
            "no spelling for {}",
            kind.as_str()
        );
    }
}
