//! Whatever the input: no panic, the original kept, the same answer twice.

use proptest::prelude::*;

use super::super::tokenize::{normalize, tokenize};
use super::super::{compare, parse, NameSource, ParsedName};

/// Pieces that names are made of, to reach the parser's branches far more
/// often than random characters would.
const PIECES: &[&str] = &[
    "Paper Harbor",
    "Neon Moth",
    "Velo",
    "Quill Ashby",
    "Remix",
    "Extended Mix",
    "Clean",
    "dirty",
    "Intro",
    "VIP",
    "Mashup",
    "Live at",
    "by",
    "feat.",
    "ft",
    "x",
    "vs.",
    "@lumenlark edit",
    "(",
    ")",
    "[",
    "]",
    "{",
    "}",
    "\u{FF08}",
    "\u{FF09}",
    "\u{3010}",
    "\u{3011}",
    " - ",
    " \u{2013} ",
    "-",
    "_",
    " ",
    "  ",
    "/",
    "\\",
    "&",
    "&#40;",
    "&#41;",
    "&amp;",
    "&#x110000;",
    "&#;",
    "%20",
    "10000001_",
    "01 ",
    "01. ",
    "A1 - ",
    "3:45",
    "320",
    "kbps",
    "RipSite.example",
    "www.",
    ".mp3",
    ".",
    ",",
    "'s",
    "\u{2019}",
    "e\u{301}",
    "\u{1F525}",
    "\u{200D}",
    "\u{200B}",
    "\u{9752}",
];

fn name_like() -> impl Strategy<Value = String> {
    proptest::collection::vec(proptest::sample::select(PIECES), 0..14)
        .prop_map(|pieces| pieces.concat())
}

fn both(input: &str) -> [ParsedName; 2] {
    [
        parse(input, NameSource::Title),
        parse(input, NameSource::FileName),
    ]
}

proptest! {
    #[test]
    fn parsing_never_panics_on_arbitrary_unicode(input in any::<String>()) {
        both(&input);
    }

    #[test]
    fn parsing_never_panics_on_name_like_strings(input in name_like()) {
        both(&input);
    }

    #[test]
    fn the_original_string_is_always_recoverable(input in any::<String>(), like in name_like()) {
        for input in [input, like] {
            for parsed in both(&input) {
                prop_assert_eq!(&parsed.original, &input);
            }
        }
    }

    #[test]
    fn parsing_a_string_twice_gives_the_same_result(input in any::<String>(), like in name_like()) {
        for input in [input, like] {
            prop_assert_eq!(both(&input), both(&input));
        }
    }

    #[test]
    fn comparing_never_panics_and_gives_the_same_answer_twice(a in name_like(), b in name_like()) {
        for a in both(&a) {
            for b in both(&b) {
                let first = compare(&a, &b);
                prop_assert_eq!(&first, &compare(&a, &b));
                prop_assert!(!first.reasons.is_empty());
            }
        }
    }

    #[test]
    fn comparing_never_panics_on_arbitrary_unicode(a in any::<String>(), b in any::<String>()) {
        compare(&parse(&a, NameSource::Title), &parse(&b, NameSource::FileName));
    }

    #[test]
    fn a_names_own_reading_is_always_its_first(input in name_like()) {
        for parsed in both(&input) {
            let readings = parsed.readings();
            prop_assert!(!readings.is_empty());
            prop_assert_eq!(&readings[0].base_title, &parsed.base_title);
            prop_assert_eq!(&readings[0].markers, &parsed.markers);
        }
    }

    #[test]
    fn every_marker_keeps_the_text_it_was_read_from(input in name_like()) {
        for parsed in both(&input) {
            for marker in parsed.markers.iter().chain(&parsed.title_markers) {
                prop_assert!(!marker.text.is_empty());
            }
        }
    }

    #[test]
    fn normalizing_twice_changes_nothing_more(input in any::<String>()) {
        let once = normalize(&input);
        prop_assert_eq!(normalize(&once), once);
    }

    #[test]
    fn tokenizing_never_panics_and_never_loses_a_letter(input in any::<String>()) {
        let plain = normalize(&input);
        let tokens = tokenize(&plain);
        let letters = |text: &str| text.chars().filter(|c| c.is_alphanumeric()).count();
        let kept: usize = tokens
            .iter()
            .map(|token| match token {
                super::super::tokenize::Token::Text(text) => letters(text),
                super::super::tokenize::Token::Group(group) => letters(&group.raw),
                super::super::tokenize::Token::Dash => 0,
            })
            .sum();
        prop_assert_eq!(kept, letters(&plain));
    }
}
