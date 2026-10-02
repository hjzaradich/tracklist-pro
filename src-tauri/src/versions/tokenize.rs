//! Splits a name into text, " - " separators and bracket groups (1bA-5).
//!
//! [`normalize`] first makes the characters plain: NFC, full-width and CJK
//! brackets as their ASCII kind, smart quotes as straight ones, every kind
//! of space as a space. [`tokenize`] then reads brackets of every kind,
//! nested and unbalanced:
//!
//! - A bracket left open runs to the end of its parent ("Title (Extended").
//! - A closing bracket closes the nearest open bracket of its own kind;
//!   brackets opened inside that one end there too.
//! - A closing bracket with nothing to close is plain text.

use unicode_normalization::UnicodeNormalization;

/// Brackets nested deeper than this are read as plain text.
pub const MAX_DEPTH: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BracketKind {
    /// `( )`
    Round,
    /// `[ ]`
    Square,
    /// `{ }`
    Curly,
    /// `〈 〉`
    Angle,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Token {
    Text(String),
    /// A " - " between two segments (any dash, with a space on each side).
    /// Only outside brackets.
    Dash,
    Group(Group),
}

/// A bracket and what's inside it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group {
    pub kind: BracketKind,
    /// False when the bracket was never closed.
    pub closed: bool,
    /// The group as written, brackets included.
    pub raw: String,
    pub children: Vec<Token>,
}

impl Group {
    /// The group's own text, without the text of brackets nested in it.
    pub fn own_text(&self) -> String {
        let parts: Vec<&str> = self
            .children
            .iter()
            .filter_map(|t| match t {
                Token::Text(s) => Some(s.as_str()),
                _ => None,
            })
            .collect();
        collapse_spaces(&parts.join(" "))
    }

    /// The brackets nested directly inside this one.
    pub fn nested(&self) -> impl Iterator<Item = &Group> {
        self.children.iter().filter_map(|t| match t {
            Token::Group(g) => Some(g),
            _ => None,
        })
    }
}

/// Makes a name's characters plain, without changing what it says.
pub fn normalize(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for c in input.chars() {
        match c {
            '\u{FF08}' => out.push('('),
            '\u{FF09}' => out.push(')'),
            '\u{FF3B}' | '\u{3010}' | '\u{3014}' | '\u{3016}' => out.push('['),
            '\u{FF3D}' | '\u{3011}' | '\u{3015}' | '\u{3017}' => out.push(']'),
            '\u{FF5B}' => out.push('{'),
            '\u{FF5D}' => out.push('}'),
            '\u{300A}' => out.push('\u{3008}'),
            '\u{300B}' => out.push('\u{3009}'),
            '\u{2018}' | '\u{2019}' | '\u{201A}' | '\u{201B}' | '\u{2032}' => out.push('\''),
            '\u{201C}' | '\u{201D}' | '\u{201E}' | '\u{201F}' | '\u{2033}' => out.push('"'),
            // Zero-width space, word joiner, byte-order mark.
            '\u{200B}' | '\u{2060}' | '\u{FEFF}' => {}
            c if c.is_whitespace() || c.is_control() => out.push(' '),
            c => out.push(c),
        }
    }
    // Composed last, so letters an invisible mark kept apart still join.
    out.nfc().collect()
}

/// Trims, and turns every run of spaces into one.
pub fn collapse_spaces(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Whether `c` is a hyphen or dash that can separate segments.
pub fn is_dash(c: char) -> bool {
    matches!(
        c,
        '-' | '\u{2010}' | '\u{2012}' | '\u{2013}' | '\u{2014}' | '\u{2015}' | '\u{FF0D}'
    )
}

fn opening(c: char) -> Option<BracketKind> {
    match c {
        '(' => Some(BracketKind::Round),
        '[' => Some(BracketKind::Square),
        '{' => Some(BracketKind::Curly),
        '\u{3008}' => Some(BracketKind::Angle),
        _ => None,
    }
}

fn closing(c: char) -> Option<BracketKind> {
    match c {
        ')' => Some(BracketKind::Round),
        ']' => Some(BracketKind::Square),
        '}' => Some(BracketKind::Curly),
        '\u{3009}' => Some(BracketKind::Angle),
        _ => None,
    }
}

/// A bracket being read. The first frame is the name itself.
struct Frame {
    kind: Option<BracketKind>,
    start: usize,
    tokens: Vec<Token>,
    text: String,
}

impl Frame {
    fn flush(&mut self) {
        if !self.text.is_empty() {
            self.tokens
                .push(Token::Text(std::mem::take(&mut self.text)));
        }
    }
}

/// Ends the innermost bracket at byte `end` and hands it to its parent.
fn close(stack: &mut Vec<Frame>, source: &str, end: usize, closed: bool) {
    let Some(mut frame) = stack.pop() else { return };
    frame.flush();
    let Some(kind) = frame.kind else {
        stack.push(frame);
        return;
    };
    let group = Group {
        kind,
        closed,
        raw: source[frame.start..end].to_string(),
        children: frame.tokens,
    };
    if let Some(parent) = stack.last_mut() {
        parent.tokens.push(Token::Group(group));
    }
}

/// Reads an already [`normalize`]d name into tokens.
pub fn tokenize(source: &str) -> Vec<Token> {
    let chars: Vec<(usize, char)> = source.char_indices().collect();
    let mut stack = vec![Frame {
        kind: None,
        start: 0,
        tokens: Vec::new(),
        text: String::new(),
    }];
    for (n, &(at, c)) in chars.iter().enumerate() {
        if let Some(kind) = opening(c) {
            if stack.len() <= MAX_DEPTH {
                if let Some(top) = stack.last_mut() {
                    top.flush();
                }
                stack.push(Frame {
                    kind: Some(kind),
                    start: at,
                    tokens: Vec::new(),
                    text: String::new(),
                });
                continue;
            }
        } else if let Some(kind) = closing(c) {
            if let Some(open) = stack.iter().rposition(|f| f.kind == Some(kind)) {
                while stack.len() > open + 1 {
                    close(&mut stack, source, at, false);
                }
                close(&mut stack, source, at + c.len_utf8(), true);
                continue;
            }
        } else if stack.len() == 1 && is_dash(c) {
            let space_before = n == 0 || chars[n - 1].1 == ' ';
            let space_after = n + 1 == chars.len() || chars[n + 1].1 == ' ';
            if space_before && space_after {
                let top = &mut stack[0];
                top.flush();
                top.tokens.push(Token::Dash);
                continue;
            }
        }
        if let Some(top) = stack.last_mut() {
            top.text.push(c);
        }
    }
    while stack.len() > 1 {
        close(&mut stack, source, source.len(), false);
    }
    let mut root = stack.pop().unwrap_or(Frame {
        kind: None,
        start: 0,
        tokens: Vec::new(),
        text: String::new(),
    });
    root.flush();
    root.tokens
}

/// Puts tokens back together as text, groups as written.
pub fn flatten(tokens: &[Token]) -> String {
    let mut out = String::new();
    for token in tokens {
        match token {
            Token::Text(s) => out.push_str(s),
            Token::Dash => out.push_str(" - "),
            Token::Group(g) => out.push_str(&g.raw),
        }
    }
    collapse_spaces(&out)
}
