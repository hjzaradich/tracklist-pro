//! "Sort by fit after the selected track" (1cB-16; ROADMAP 1.14): how well
//! one track follows another, by key and by tempo, and the order a crate's
//! tracks take when sorted by it. Pure: no database, no clock, no files.
//!
//! **Key.** Keys are Camelot (`8A`). The same key, or a neighbor on the
//! wheel, fits: the same letter with the number one up or one down (12 and
//! 1 are neighbors), or the same number with the other letter (8A and 8B).
//! Any other key does not. A track with no key, or a selected track with
//! none, has no key fit, and says so.
//!
//! **Tempo.** The difference in BPM, track minus selected track. Half and
//! double time count: a 70 BPM track follows a 140 BPM one, and the reason
//! says "half time" (or "double time") so the DJ sees it was counted. The
//! reading with the smallest difference is the one used; the plain tempo wins
//! a tie.
//!
//! **Points** (0 to 6) put the two together, and the dots are read off them:
//! the same key gives 3 and a neighbor key 2; a tempo within 2 BPM gives 3,
//! within 4 BPM 2 and within 8 BPM 1. 5 or 6 points is 3 dots, 4 is 2 dots,
//! 2 or 3 is 1 dot, else none. So three dots need both a fitting key and a
//! close tempo; a fitting key alone, or a close tempo alone, is one dot.
//!
//! **Order.** Most points first; among equal points, the smaller BPM
//! difference first (a track with no BPM difference to give comes last);
//! among those, the order the tracks came in (a stable sort). The selected
//! track itself is always first.

use serde::{Deserialize, Serialize};
use specta::Type;

#[cfg(test)]
mod tests;

/// Within this many BPM, a tempo scores these points.
const TEMPO_STEPS: [(f64, u8); 3] = [(2.0, 3), (4.0, 2), (8.0, 1)];

/// A key on the Camelot wheel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Camelot {
    /// 1 to 12.
    pub number: u8,
    /// `true` for B (major), `false` for A (minor).
    pub major: bool,
}

impl Camelot {
    /// Parses `8A`, `08A` or `12B`, in either letter case; anything else
    /// is no key.
    pub fn parse(text: &str) -> Option<Camelot> {
        let text = text.trim();
        let letter = text.chars().last()?;
        let major = match letter.to_ascii_uppercase() {
            'A' => false,
            'B' => true,
            _ => return None,
        };
        let digits = &text[..text.len() - letter.len_utf8()];
        // `parse` would take a leading "+".
        if !digits.chars().all(|c| c.is_ascii_digit()) {
            return None;
        }
        let number: u8 = digits.parse().ok()?;
        (1..=12)
            .contains(&number)
            .then_some(Camelot { number, major })
    }

    /// A key from its place on the wheel as the track table stores it
    /// (`search::camelot_order`: 1A is 2, 1B is 3, up to 12B at 25).
    pub fn from_order(order: i64) -> Option<Camelot> {
        if !(2..=25).contains(&order) {
            return None;
        }
        Some(Camelot {
            number: u8::try_from(order / 2).ok()?,
            major: order % 2 == 1,
        })
    }

    /// The key as written: `8A`.
    pub fn name(self) -> String {
        format!("{}{}", self.number, if self.major { 'B' } else { 'A' })
    }

    /// The number one step up (12 is followed by 1) or down (1 by 12).
    fn step(self, up: bool) -> u8 {
        match (up, self.number) {
            (true, 12) => 1,
            (true, n) => n + 1,
            (false, 1) => 12,
            (false, n) => n - 1,
        }
    }

    /// Whether `other` is a neighbor: not the same key, but next to it on
    /// the wheel.
    pub fn neighbors(self, other: Camelot) -> bool {
        if self == other {
            return false;
        }
        if self.major == other.major {
            other.number == self.step(true) || other.number == self.step(false)
        } else {
            other.number == self.number
        }
    }
}

/// What a track is compared by.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Facts {
    pub bpm: Option<f64>,
    pub key: Option<Camelot>,
}

/// How a track's key relates to the selected track's.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum KeyFit {
    /// The same key.
    Same { key: String },
    /// Next to it on the wheel.
    Neighbor { from: String, to: String },
    /// Any other key.
    Far { from: String, to: String },
    /// The track has no key.
    NoKey,
    /// The selected track has no key.
    NoTargetKey,
}

/// Which way the tempo was read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum TimeFeel {
    /// As it is.
    Straight,
    /// The track is at half the selected track's tempo.
    Half,
    /// The track is at double the selected track's tempo.
    Double,
}

/// How a track's tempo relates to the selected track's.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum BpmFit {
    /// `delta` BPM above (positive) or below the selected track's, read the
    /// way `time` says.
    Delta { delta: f64, time: TimeFeel },
    /// The track has no BPM.
    NoBpm,
    /// The selected track has no BPM.
    NoTargetBpm,
}

/// The reason a track fits, as facts for the screen to word.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct FitReason {
    pub key: KeyFit,
    pub bpm: BpmFit,
}

/// What the Fit column shows for a row.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RowFit {
    /// 0 to 3.
    pub dots: u8,
    /// Why; `None` for the selected track itself, which has no fit to
    /// itself.
    pub reason: Option<FitReason>,
}

/// A track's fit to the selected track.
#[derive(Debug, Clone, PartialEq)]
pub struct Fit {
    /// 0 to 6.
    pub points: u8,
    pub reason: FitReason,
    /// The BPM difference the points were given for, without its sign.
    spread: Option<f64>,
}

impl Fit {
    /// 0 to 3.
    pub fn dots(&self) -> u8 {
        match self.points {
            5.. => 3,
            4 => 2,
            2 | 3 => 1,
            _ => 0,
        }
    }

    pub fn row(&self) -> RowFit {
        RowFit {
            dots: self.dots(),
            reason: Some(self.reason.clone()),
        }
    }
}

fn key_fit(target: Option<Camelot>, track: Option<Camelot>) -> (u8, KeyFit) {
    let Some(target) = target else {
        return (0, KeyFit::NoTargetKey);
    };
    let Some(track) = track else {
        return (0, KeyFit::NoKey);
    };
    if track == target {
        (3, KeyFit::Same { key: track.name() })
    } else if target.neighbors(track) {
        (
            2,
            KeyFit::Neighbor {
                from: target.name(),
                to: track.name(),
            },
        )
    } else {
        (
            0,
            KeyFit::Far {
                from: target.name(),
                to: track.name(),
            },
        )
    }
}

fn bpm_fit(target: Option<f64>, track: Option<f64>) -> (u8, Option<f64>, BpmFit) {
    let Some(target) = target else {
        return (0, None, BpmFit::NoTargetBpm);
    };
    let Some(track) = track else {
        return (0, None, BpmFit::NoBpm);
    };
    // The straight reading first, so it wins a tie.
    let (delta, time) = [
        (track - target, TimeFeel::Straight),
        (track - target * 2.0, TimeFeel::Double),
        (track - target / 2.0, TimeFeel::Half),
    ]
    .into_iter()
    .reduce(|best, next| {
        if next.0.abs() < best.0.abs() {
            next
        } else {
            best
        }
    })
    .expect("three readings");
    let points = TEMPO_STEPS
        .iter()
        .find(|(within, _)| delta.abs() <= *within)
        .map_or(0, |(_, points)| *points);
    (points, Some(delta.abs()), BpmFit::Delta { delta, time })
}

/// How well `track` follows `target`.
pub fn fit(target: Facts, track: Facts) -> Fit {
    let (key_points, key) = key_fit(target.key, track.key);
    let (bpm_points, spread, bpm) = bpm_fit(target.bpm, track.bpm);
    Fit {
        points: key_points + bpm_points,
        reason: FitReason { key, bpm },
        spread,
    }
}

/// A track to place: its id and what it is compared by.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Candidate {
    pub id: i64,
    pub facts: Facts,
}

/// `tracks`, in the order they came in, arranged by fit to `target` (the
/// facts of the track with id `after`): the selected track first if it is
/// among them, then the rest, best fit first. Equal fits keep the order they
/// came in. Each track comes with its fit; the selected track's has no
/// reason.
pub fn arrange(after: i64, target: Facts, tracks: &[Candidate]) -> Vec<(i64, RowFit)> {
    let mut rest: Vec<(i64, Fit)> = tracks
        .iter()
        .filter(|t| t.id != after)
        .map(|t| (t.id, fit(target, t.facts)))
        .collect();
    // `sort_by` is stable: equal fits stay as they came in.
    rest.sort_by(|(_, a), (_, b)| {
        b.points
            .cmp(&a.points)
            .then_with(|| match (a.spread, b.spread) {
                (Some(x), Some(y)) => x.total_cmp(&y),
                // A track with no BPM difference to give is after the rest.
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => std::cmp::Ordering::Equal,
            })
    });
    let mut out = Vec::with_capacity(tracks.len());
    if tracks.iter().any(|t| t.id == after) {
        out.push((
            after,
            RowFit {
                dots: 0,
                reason: None,
            },
        ));
    }
    out.extend(rest.into_iter().map(|(id, fit)| (id, fit.row())));
    out
}
