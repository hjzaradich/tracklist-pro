//! "Fill the gap" (ROADMAP 3.5): for the place between two playlist tracks A
//! and B, the Library tracks X that fit both A → X and X → B, each with the
//! reasons that name the gap it fills. Pure: no database, no clock.
//!
//! - **Same rules as "what's next".** A leg's key, BPM and energy fitness are
//!   the very functions [`super::score`] uses (Camelot relation, half and
//!   double time, the tight or wide tolerance), so a pair is a clash here
//!   exactly when "what's next" gives it no credit. The settings' weights and
//!   tolerance are the same too.
//! - **A signal is as good as its worse leg.** Key and BPM: X's fitness is the
//!   lower of A → X and X → B, so a true bridge (8A → 9A → 10A) beats a track
//!   that fits one side only. A signal needs a value at one end at least;
//!   a candidate that lacks it scores 0 on it. The score is the weighted
//!   average of the signals that apply.
//! - **Energy** aims at the playlist's target at this place when it has one,
//!   else halfway between A's and B's energy (or the one end's, if the other
//!   has none). Energy is only what tracks show: from tags or the user, never
//!   estimated here. A candidate with no energy, while energy counts, goes
//!   last.
//! - **Reasons** say what the track does for this place: `keyBridge` and
//!   `bpmStep` when A → B itself clashes and X fixes it, `energyTarget` or
//!   `energyStep` for energy, `keyFits` and `bpmFits` when both sides were
//!   fine already. No reason, no suggestion.
//! - **Order.** Energy known first, then the higher score, then the lower id.

use std::collections::HashSet;

use serde::Serialize;
use specta::Type;

use super::score::{
    bpm_fitness, energy_fitness, key_fitness, key_relation, Candidate, Facts, MOST_REASONS,
};
use super::{Direction, Settings, Tolerance};
use crate::crates::fit::{tempo_gap, Camelot};

/// How the key of `to` follows `from`: 0 to 1 (0 is a clash), or `None` when
/// either has no key.
pub fn key_fit(from: Option<Camelot>, to: Option<Camelot>, tolerance: Tolerance) -> Option<f64> {
    Some(key_fitness(key_relation(from?, to?), tolerance))
}

/// How the tempo of `to` follows `from`'s, half and double time counted:
/// 0 to 1 (0 is a jump), or `None` when either has no BPM.
pub fn bpm_fit(from: Option<f64>, to: Option<f64>, tolerance: Tolerance) -> Option<f64> {
    Some(bpm_fitness(tempo_gap(from?, to?).0, tolerance))
}

/// Whether an energy step (`to` minus `from`) is bigger than any direction
/// "what's next" gives credit for.
pub fn energy_jump(step: i32, tolerance: Tolerance) -> bool {
    [Direction::Up, Direction::Same, Direction::Down]
        .into_iter()
        .all(|d| energy_fitness(step, d, tolerance) == 0.0)
}

/// What the gap is.
pub struct Gap<'a> {
    /// A, the track before it; none at the start of the playlist.
    pub before: Option<Facts>,
    /// B, the track after it; none at the end.
    pub after: Option<Facts>,
    /// The playlist's target energy at this place, if it has a target.
    pub target: Option<f64>,
    pub settings: &'a Settings,
    /// Tracks never offered (the playlist's own, and A and B).
    pub exclude: &'a HashSet<i64>,
}

/// Why a track fills the gap, as data for the screen to word
/// (`playlists:gap.<code>`). Keys are Camelot (`8A`); the screen shows them
/// in the user's notation. `from` and `to` are A's and B's values, `via` the
/// suggested track's; at an end of the playlist one of them is missing.
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(tag = "code", rename_all = "camelCase")]
pub enum BridgeReason {
    /// A → B clashes on key; X is a key between: `8A → 9A → 10A`.
    KeyBridge {
        from: String,
        via: String,
        to: String,
    },
    /// The key fits on every side that has one.
    KeyFits {
        from: Option<String>,
        via: String,
        to: Option<String>,
    },
    /// A → B jumps in tempo; X's tempo sits between (half and double time
    /// read as the closest).
    BpmStep { from: f64, via: f64, to: f64 },
    /// The tempo fits on every side that has one.
    BpmFits {
        from: Option<f64>,
        via: f64,
        to: Option<f64>,
    },
    /// X's energy is near the playlist's target at this place.
    EnergyTarget { via: u8, target: f64 },
    /// X's energy, between A's and B's: `5 → 6 → 7`.
    EnergyStep {
        from: Option<u8>,
        via: u8,
        to: Option<u8>,
    },
}

impl BridgeReason {
    /// Gap-fixing reasons first.
    fn rank(&self) -> u8 {
        match self {
            BridgeReason::KeyBridge { .. } => 0,
            BridgeReason::BpmStep { .. } => 1,
            BridgeReason::EnergyTarget { .. } => 2,
            BridgeReason::EnergyStep { .. } => 3,
            BridgeReason::KeyFits { .. } => 4,
            BridgeReason::BpmFits { .. } => 5,
        }
    }
}

/// A suggested track.
#[derive(Debug, Clone, PartialEq)]
pub struct Bridged {
    pub id: i64,
    /// 0 to 100.
    pub score: u8,
    pub reasons: Vec<BridgeReason>,
}

/// A signal's fitness for X: the worse of the legs A → X and X → B that
/// exist; 0 when X lacks the value; `None` when neither end has one (the
/// signal doesn't apply).
fn through<T: Copy>(
    a: Option<T>,
    x: Option<T>,
    b: Option<T>,
    fit: impl Fn(T, T) -> f64,
) -> Option<f64> {
    if a.is_none() && b.is_none() {
        return None;
    }
    let Some(x) = x else { return Some(0.0) };
    let legs = [a.map(|a| fit(a, x)), b.map(|b| fit(x, b))];
    Some(legs.into_iter().flatten().fold(1.0, f64::min))
}

/// The energy the place calls for.
fn energy_wanted(gap: &Gap) -> Option<f64> {
    gap.target.or_else(|| {
        let energy = |f: Option<Facts>| f.and_then(|f| f.energy).map(f64::from);
        match (energy(gap.before), energy(gap.after)) {
            (Some(a), Some(b)) => Some((a + b) / 2.0),
            (Some(e), None) | (None, Some(e)) => Some(e),
            (None, None) => None,
        }
    })
}

fn energy_fit(want: Option<f64>, x: Option<u8>, tolerance: Tolerance) -> Option<f64> {
    let want = want?;
    Some(x.map_or(0.0, |x| {
        let step = (f64::from(x) - want).round() as i32;
        energy_fitness(step, Direction::Same, tolerance)
    }))
}

/// (fitness, goes last for lacking energy) for X, or `None` if no signal
/// applies to the gap.
fn score_of(gap: &Gap, x: &Facts) -> Option<(f64, bool)> {
    let w = &gap.settings.weights;
    let tol = gap.settings.tolerance;
    let (a, b) = (gap.before, gap.after);
    let key = through(
        a.and_then(|f| f.key),
        x.key,
        b.and_then(|f| f.key),
        |p, q| key_fitness(key_relation(p, q), tol),
    );
    let bpm = through(
        a.and_then(|f| f.bpm),
        x.bpm,
        b.and_then(|f| f.bpm),
        |p, q| bpm_fitness(tempo_gap(p, q).0, tol),
    );
    let want = energy_wanted(gap);
    let energy = energy_fit(want, x.energy, tol);
    let mut weights = 0.0;
    let mut total = 0.0;
    for (weight, fitness) in [(w.key, key), (w.bpm, bpm), (w.energy, energy)] {
        if let (true, Some(fitness)) = (weight > 0, fitness) {
            weights += f64::from(weight);
            total += f64::from(weight) * fitness;
        }
    }
    if weights == 0.0 {
        return None;
    }
    let lacks_energy = w.energy > 0 && want.is_some() && x.energy.is_none();
    Some((total / weights, lacks_energy))
}

/// `Some(true)` if X fits every leg that exists and A → B itself clashes,
/// `Some(false)` if it fits and A → B was fine (or an end is missing),
/// `None` if X doesn't fit.
fn fills<T: Copy>(a: Option<T>, x: T, b: Option<T>, fit: impl Fn(T, T) -> f64) -> Option<bool> {
    if a.is_none() && b.is_none() {
        return None;
    }
    let legs = [a.map(|a| fit(a, x)), b.map(|b| fit(x, b))];
    if !legs.into_iter().flatten().all(|f| f > 0.0) {
        return None;
    }
    Some(matches!((a, b), (Some(a), Some(b)) if fit(a, b) == 0.0))
}

fn reasons_of(gap: &Gap, x: &Facts) -> Vec<BridgeReason> {
    let w = &gap.settings.weights;
    let tol = gap.settings.tolerance;
    let (a, b) = (gap.before, gap.after);
    let mut out = Vec::new();
    let (ka, kb) = (a.and_then(|f| f.key), b.and_then(|f| f.key));
    if let (true, Some(kx)) = (w.key > 0, x.key) {
        match fills(ka, kx, kb, |p, q| key_fitness(key_relation(p, q), tol)) {
            Some(true) => {
                if let (Some(ka), Some(kb)) = (ka, kb) {
                    out.push(BridgeReason::KeyBridge {
                        from: ka.name(),
                        via: kx.name(),
                        to: kb.name(),
                    });
                }
            }
            Some(false) => out.push(BridgeReason::KeyFits {
                from: ka.map(Camelot::name),
                via: kx.name(),
                to: kb.map(Camelot::name),
            }),
            None => {}
        }
    }
    let (pa, pb) = (a.and_then(|f| f.bpm), b.and_then(|f| f.bpm));
    if let (true, Some(px)) = (w.bpm > 0, x.bpm) {
        match fills(pa, px, pb, |p, q| bpm_fitness(tempo_gap(p, q).0, tol)) {
            Some(true) => {
                if let (Some(pa), Some(pb)) = (pa, pb) {
                    out.push(BridgeReason::BpmStep {
                        from: pa,
                        via: px,
                        to: pb,
                    });
                }
            }
            Some(false) => out.push(BridgeReason::BpmFits {
                from: pa,
                via: px,
                to: pb,
            }),
            None => {}
        }
    }
    if let (true, Some(ex)) = (w.energy > 0, x.energy) {
        let near = energy_fit(energy_wanted(gap), Some(ex), tol).is_some_and(|f| f > 0.0);
        if near {
            out.push(match gap.target {
                Some(target) => BridgeReason::EnergyTarget { via: ex, target },
                None => BridgeReason::EnergyStep {
                    from: a.and_then(|f| f.energy),
                    via: ex,
                    to: b.and_then(|f| f.energy),
                },
            });
        }
    }
    // Stable: equal ranks keep the signals' order.
    out.sort_by_key(BridgeReason::rank);
    out.truncate(MOST_REASONS);
    out
}

/// The `most` candidates that best fill the gap, best first, each with at
/// least one reason.
pub fn bridge(gap: &Gap, candidates: &[Candidate], most: usize) -> Vec<Bridged> {
    // The order needs only a number per candidate; reasons are worked out
    // while walking the best first.
    let mut order: Vec<(bool, f64, usize)> = candidates
        .iter()
        .enumerate()
        .filter(|(_, c)| !gap.exclude.contains(&c.id))
        .filter_map(|(n, c)| {
            score_of(gap, &c.facts)
                .filter(|(score, _)| *score > 0.0)
                .map(|(score, lacks)| (lacks, score, n))
        })
        .collect();
    order.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then_with(|| b.1.total_cmp(&a.1))
            .then_with(|| candidates[a.2].id.cmp(&candidates[b.2].id))
    });
    let mut out = Vec::with_capacity(most.min(order.len()));
    for (_, score, n) in order {
        if out.len() >= most {
            break;
        }
        let c = &candidates[n];
        let reasons = reasons_of(gap, &c.facts);
        if reasons.is_empty() {
            continue;
        }
        out.push(Bridged {
            id: c.id,
            score: (score * 100.0).round().clamp(0.0, 100.0) as u8,
            reasons,
        });
    }
    out
}
