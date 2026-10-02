//! Comparing one candidate pair, always on full-track fingerprints
//! (1bA-2; ROADMAP 1.4). The definitions are in the [module docs](super).

use rusty_chromaprint::match_fingerprints;

use crate::fingerprint::stored::config;
use crate::fingerprint::{CompareError, Fingerprint};

/// A secondary alignment is only looked for between stretches at least
/// this long on both sides (about 5 s), and only kept if at least this many
/// items match. Shorter than that, a chance match is too easy.
pub const MIN_SECONDARY_ITEMS: usize = 40;

/// At most this many alignments per pair, the main one included.
pub const MAX_ALIGNMENTS: usize = 8;

/// The score when nothing matched: every bit differs.
pub const NO_MATCH_SCORE: f64 = 32.0;

/// A stretch of A and a stretch of B, equally long, that hold the same
/// audio. Offsets and lengths are in fingerprint items
/// ([`item_seconds`] each).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Segment {
    /// Where it starts in A, in items from A's start.
    pub offset_a: usize,
    /// Where it starts in B, in items from B's start.
    pub offset_b: usize,
    /// How long it is, in items.
    pub items: usize,
    /// The average number of differing bits (of 32) per item over this
    /// stretch: 0 is identical. Always under 10, the matcher's limit.
    pub score: f64,
    /// Which alignment found it: 0 is the main one, the single best way to
    /// lay B over A. Higher numbers were found afterwards, in what the
    /// earlier ones left unmatched.
    pub alignment: u8,
}

impl Segment {
    /// How far B's audio sits after A's in this stretch, in items:
    /// positive when the shared audio starts later in A than in B.
    pub fn shift(&self) -> i64 {
        self.offset_a as i64 - self.offset_b as i64
    }
}

/// How two full-track fingerprints line up.
#[derive(Debug, Clone, PartialEq)]
pub struct Comparison {
    /// A's length in items.
    pub items_a: usize,
    /// B's length in items.
    pub items_b: usize,
    /// The share of A matched by the main alignment, 0 to 1.
    pub coverage_a: f32,
    /// The share of B matched by the main alignment, 0 to 1.
    pub coverage_b: f32,
    /// The difference score over the main alignment's matched part: the
    /// average number of differing bits (of 32) per item, weighted by
    /// length. 0 is identical; [`NO_MATCH_SCORE`] when nothing matched.
    pub score: f64,
    /// Every matched stretch, the main alignment's first, each alignment's
    /// in order of where they start in A. No item of A or of B is in two.
    pub segments: Vec<Segment>,
}

impl Comparison {
    /// The share of A found anywhere in B, over every alignment: what a
    /// cut is judged on. Never below [`coverage_a`](Self::coverage_a).
    pub fn found_a(&self) -> f32 {
        share(self.segments.iter().map(|s| s.items).sum(), self.items_a)
    }

    /// The share of B found anywhere in A, over every alignment.
    pub fn found_b(&self) -> f32 {
        share(self.segments.iter().map(|s| s.items).sum(), self.items_b)
    }

    /// The same comparison seen from B: sides swapped.
    pub fn swapped(&self) -> Comparison {
        let mut segments: Vec<Segment> = self
            .segments
            .iter()
            .map(|s| Segment {
                offset_a: s.offset_b,
                offset_b: s.offset_a,
                ..*s
            })
            .collect();
        segments.sort_by_key(|s| (s.alignment, s.offset_a));
        Comparison {
            items_a: self.items_b,
            items_b: self.items_a,
            coverage_a: self.coverage_b,
            coverage_b: self.coverage_a,
            score: self.score,
            segments,
        }
    }

    /// Two fingerprints with the same items: everything matches, exactly.
    pub(crate) fn identical(items: usize) -> Comparison {
        Comparison {
            items_a: items,
            items_b: items,
            coverage_a: 1.0,
            coverage_b: 1.0,
            score: 0.0,
            segments: vec![Segment {
                offset_a: 0,
                offset_b: 0,
                items,
                score: 0.0,
                alignment: 0,
            }],
        }
    }
}

/// How much audio one fingerprint item stands for, in seconds (~0.124).
pub fn item_seconds() -> f32 {
    config().item_duration_in_seconds()
}

fn share(matched: usize, of: usize) -> f32 {
    if of == 0 {
        return 0.0;
    }
    (matched as f32 / of as f32).min(1.0)
}

/// A stretch of items, `start..end`.
type Span = (usize, usize);

/// Lines up `a[span_a]` and `b[span_b]` with rusty-chromaprint's matcher
/// and gives the matched stretches in whole-track offsets.
fn align(
    a: &[u32],
    b: &[u32],
    span_a: Span,
    span_b: Span,
    alignment: u8,
) -> Result<Vec<Segment>, CompareError> {
    let found = match_fingerprints(&a[span_a.0..span_a.1], &b[span_b.0..span_b.1], &config())
        .map_err(|_| CompareError::TooLong)?;
    Ok(found
        .into_iter()
        .filter(|s| s.items_count > 0)
        .map(|s| Segment {
            offset_a: span_a.0 + s.offset1,
            offset_b: span_b.0 + s.offset2,
            items: s.items_count,
            score: s.score,
            alignment,
        })
        .collect())
}

/// The stretches of `0..len` no segment covers, at least `min` long.
fn unmatched(len: usize, mut taken: Vec<Span>, min: usize) -> Vec<Span> {
    taken.sort_unstable();
    let mut free = Vec::new();
    let mut at = 0;
    for (start, end) in taken {
        if start >= at + min {
            free.push((at, start));
        }
        at = at.max(end);
    }
    if len >= at + min {
        free.push((at, len));
    }
    free
}

/// Compares two full-track fingerprints: the main alignment, which the
/// duplicate rule is judged on, then up to [`MAX_ALIGNMENTS`]` - 1` more
/// in what it left unmatched, for cuts with a part taken out.
pub fn compare(a: &Fingerprint, b: &Fingerprint) -> Result<Comparison, CompareError> {
    if !a.comparable_with(b) {
        return Err(CompareError::Incomparable);
    }
    let (a, b) = (a.items(), b.items());
    let mut segments = align(a, b, (0, a.len()), (0, b.len()), 0)?;
    let matched: usize = segments.iter().map(|s| s.items).sum();
    let score = if matched > 0 {
        segments
            .iter()
            .map(|s| s.score * s.items as f64)
            .sum::<f64>()
            / matched as f64
    } else {
        NO_MATCH_SCORE
    };
    let (coverage_a, coverage_b) = (share(matched, a.len()), share(matched, b.len()));

    // Secondary alignments: each round tries every unmatched stretch of A
    // against every unmatched stretch of B and keeps the pair that matches
    // most. A round's results stay valid until one of its stretches is cut.
    let mut tried: Vec<(Span, Span, Vec<Segment>)> = Vec::new();
    for alignment in 1..MAX_ALIGNMENTS as u8 {
        if segments.is_empty() {
            break;
        }
        let taken = |side: fn(&Segment) -> usize| -> Vec<Span> {
            segments
                .iter()
                .map(|s| (side(s), side(s) + s.items))
                .collect()
        };
        let free_a = unmatched(a.len(), taken(|s| s.offset_a), MIN_SECONDARY_ITEMS);
        let free_b = unmatched(b.len(), taken(|s| s.offset_b), MIN_SECONDARY_ITEMS);
        tried.retain(|(sa, sb, _)| free_a.contains(sa) && free_b.contains(sb));
        for &span_a in &free_a {
            for &span_b in &free_b {
                if tried
                    .iter()
                    .any(|(sa, sb, _)| *sa == span_a && *sb == span_b)
                {
                    continue;
                }
                let mut found = align(a, b, span_a, span_b, alignment)?;
                found.retain(|s| s.items >= MIN_SECONDARY_ITEMS);
                tried.push((span_a, span_b, found));
            }
        }
        let total = |found: &[Segment]| found.iter().map(|s| s.items).sum::<usize>();
        // The most matched; on a tie, the earliest in A, then in B.
        let Some(best) = tried
            .iter()
            .filter(|(_, _, found)| !found.is_empty())
            .max_by_key(|(sa, sb, found)| (total(found), std::cmp::Reverse((*sa, *sb))))
        else {
            break;
        };
        segments.extend(best.2.iter().map(|s| Segment { alignment, ..*s }));
    }

    Ok(Comparison {
        items_a: a.len(),
        items_b: b.len(),
        coverage_a,
        coverage_b,
        score,
        segments,
    })
}
