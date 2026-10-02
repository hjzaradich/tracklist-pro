//! The tokenizer (1bA-5) by itself.

use super::super::tokenize::{flatten, normalize, tokenize, BracketKind, Group, Token, MAX_DEPTH};

fn text(s: &str) -> Token {
    Token::Text(s.to_string())
}

fn groups(tokens: &[Token]) -> Vec<&Group> {
    tokens
        .iter()
        .filter_map(|t| match t {
            Token::Group(g) => Some(g),
            _ => None,
        })
        .collect()
}

#[test]
fn plain_text_is_one_token() {
    assert_eq!(tokenize("Glasswing"), vec![text("Glasswing")]);
}

#[test]
fn an_empty_name_has_no_tokens() {
    assert_eq!(tokenize(""), vec![]);
}

#[test]
fn every_bracket_kind_makes_a_group_of_its_kind() {
    for (name, kind) in [
        ("Glasswing (Extended Mix)", BracketKind::Round),
        ("Glasswing [Extended Mix]", BracketKind::Square),
        ("Glasswing {Extended Mix}", BracketKind::Curly),
        ("Glasswing \u{3008}Extended Mix\u{3009}", BracketKind::Angle),
    ] {
        let tokens = tokenize(name);
        let found = groups(&tokens);
        assert_eq!(found.len(), 1, "{name}");
        assert_eq!(found[0].kind, kind, "{name}");
        assert!(found[0].closed, "{name}");
        assert_eq!(found[0].own_text(), "Extended Mix", "{name}");
    }
}

#[test]
fn a_group_keeps_how_it_was_written() {
    let tokens = tokenize("Glasswing [ Extended  Mix ]");
    assert_eq!(groups(&tokens)[0].raw, "[ Extended  Mix ]");
}

#[test]
fn full_width_and_cjk_brackets_become_plain_ones() {
    assert_eq!(normalize("\u{FF08}a\u{FF09}"), "(a)");
    assert_eq!(normalize("\u{FF3B}a\u{FF3D}"), "[a]");
    assert_eq!(normalize("\u{3010}a\u{3011}"), "[a]");
    assert_eq!(normalize("\u{3014}a\u{3015}"), "[a]");
    assert_eq!(normalize("\u{FF5B}a\u{FF5D}"), "{a}");
    assert_eq!(normalize("\u{300A}a\u{300B}"), "\u{3008}a\u{3009}");
}

#[test]
fn smart_quotes_become_straight_ones() {
    assert_eq!(
        normalize("\u{2018}a\u{2019} \u{201C}b\u{201D}"),
        "'a' \"b\""
    );
}

#[test]
fn decomposed_letters_are_composed() {
    assert_eq!(normalize("Cafe\u{301}"), "Caf\u{e9}");
}

#[test]
fn every_kind_of_space_becomes_a_space_and_invisible_marks_go() {
    assert_eq!(normalize("a\u{A0}b\u{3000}c\td\ne"), "a b c d e");
    assert_eq!(normalize("a\u{200B}b\u{FEFF}"), "ab");
}

#[test]
fn nested_brackets_nest() {
    let tokens = tokenize("Glasswing (Quill Ashby Remix (Extended))");
    let outer = groups(&tokens);
    assert_eq!(outer.len(), 1);
    assert_eq!(outer[0].own_text(), "Quill Ashby Remix");
    let inner: Vec<&Group> = outer[0].nested().collect();
    assert_eq!(inner.len(), 1);
    assert_eq!(inner[0].own_text(), "Extended");
    assert_eq!(inner[0].raw, "(Extended)");
}

#[test]
fn a_bracket_left_open_runs_to_the_end() {
    let tokens = tokenize("Glasswing (Extended Mix");
    let found = groups(&tokens);
    assert_eq!(found.len(), 1);
    assert!(!found[0].closed);
    assert_eq!(found[0].own_text(), "Extended Mix");
    assert_eq!(found[0].raw, "(Extended Mix");
}

#[test]
fn a_closing_bracket_with_nothing_to_close_is_text() {
    assert_eq!(tokenize("Glasswing) Mix]"), vec![text("Glasswing) Mix]")]);
}

#[test]
fn a_closing_bracket_closes_its_own_kind_and_ends_brackets_opened_inside() {
    let tokens = tokenize("Glasswing [Extended (Mix] after");
    let outer = groups(&tokens);
    assert_eq!(outer.len(), 1);
    assert_eq!(outer[0].kind, BracketKind::Square);
    assert!(outer[0].closed);
    let inner: Vec<&Group> = outer[0].nested().collect();
    assert_eq!(inner.len(), 1);
    assert_eq!(inner[0].kind, BracketKind::Round);
    assert!(!inner[0].closed);
    assert_eq!(tokens.last(), Some(&text(" after")));
}

#[test]
fn a_closing_bracket_of_another_kind_is_text_inside_the_open_one() {
    let tokens = tokenize("Glasswing (Extended Mix]");
    let found = groups(&tokens);
    assert_eq!(found.len(), 1);
    assert!(!found[0].closed);
    assert_eq!(found[0].own_text(), "Extended Mix]");
}

#[test]
fn a_dash_with_a_space_each_side_separates_segments() {
    for dash in ['-', '\u{2013}', '\u{2014}', '\u{FF0D}'] {
        let tokens = tokenize(&format!("Nemora Vale {dash} Glasswing"));
        assert_eq!(
            tokens,
            vec![text("Nemora Vale "), Token::Dash, text(" Glasswing")],
            "{dash}"
        );
    }
}

#[test]
fn a_hyphen_inside_a_word_is_text() {
    assert_eq!(tokenize("Re-Fix"), vec![text("Re-Fix")]);
}

#[test]
fn a_dash_inside_a_bracket_is_not_a_segment_separator() {
    let tokens = tokenize("Glasswing (Intro - Clean)");
    assert!(!tokens.contains(&Token::Dash));
    assert_eq!(groups(&tokens)[0].own_text(), "Intro - Clean");
}

#[test]
fn brackets_nested_past_the_limit_are_text_not_a_crash() {
    let deep = "(".repeat(MAX_DEPTH * 50);
    let tokens = tokenize(&deep);
    assert!(!tokens.is_empty());
    let mut depth = 0;
    let mut level = groups(&tokens);
    while let Some(group) = level.first() {
        depth += 1;
        level = group.nested().collect();
    }
    assert_eq!(depth, MAX_DEPTH);
}

#[test]
fn flatten_gives_the_text_back_with_single_spaces() {
    let name = "Nemora Vale - Glasswing  (Extended Mix) [x]";
    assert_eq!(
        flatten(&tokenize(name)),
        "Nemora Vale - Glasswing (Extended Mix) [x]"
    );
}
