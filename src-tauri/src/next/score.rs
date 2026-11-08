//! "What's next" scoring (ROADMAP 3.4): how well each candidate follows the
//! chosen track. Pure: no database, no clock, no files.
//!
//! Each signal gives a fitness from 0 to 1; the score is the weighted
//! average of the signals that apply, as a percent. A signal does not apply,
//! and is left out of the average, when its weight is 0 or when the chosen
//! track has nothing to compare (no key, no BPM, no energy, no played
//! transitions at all); those are reported as `ignored`. A candidate that
//! lacks what the chosen track has scores 0 on that signal and says so.
//!
//! **Energy.** The step is the candidate's shown energy minus the chosen
//! track's. The ideal step is +1 going up, 0 staying, -1 going down; the
//! fitness falls with the distance from it. A candidate with no energy, while
//! energy is weighed and the chosen track has one, comes after every
//! candidate that has one, and `energyNone` is its first reason.
//!
//! **Order.** Energy known first, then the higher score, then the lower id.
//! Reasons (at most [`MOST_REASONS`]) are the signals that contributed most.
//!
//! **Genre** (3.2). The same canonical node is a full fit; a node that shares
//! a parent with the chosen track's (or is its parent or child) is half. A
//! track with no mapped genre gives the candidate nothing, and the chosen
//! track having none leaves the signal unused. **Tags** (3.1) count the tags
//! both have, against the chosen track's own (at most three): sharing all of
//! up to three is a full fit. None on the chosen track leaves it unused.
//! Neither says anything when it counts for nothing.
//!
//! **Sounds similar** (3.8). The audio model's opinion of how alike the two
//! sound: the cosine of the two fingerprints after the Library's average
//! fingerprint is taken off (the number "Sounds like" shows), with a negative
//! cosine counting as 0, so the fitness is `max(cosine, 0)` from 0 to 1 and
//! its reason says that as a whole percent. It is unused (and said so) when
//! its weight is above 0 but the model is off or the chosen track has no
//! current fingerprint; a candidate without one just gets no similarity part.
//! **Never the only reason:** a candidate's similarity part exists only if
//! at least one of its fact parts (key, BPM, energy, played, genre, tags) is
//! above 0. A candidate that sounds alike and has nothing else in common is
//! therefore scored and shown exactly as if there were no similarity signal,
//! and the reason is never shown without a fact reason beside it.

use std::cmp::Ordering;
use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use specta::Type;

use super::{Direction, Settings, Signal, Tolerance, Weights, MOST_WEIGHT};
use crate::audio_model::similar::{likeness_of, Resemblance};
use crate::crates::fit::{tempo_gap, Camelot, TimeFeel, TEMPO_STEPS};

/// The most reasons a result carries.
pub const MOST_REASONS: usize = 3;

/// What a track is compared by.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Facts {
    pub bpm: Option<f64>,
    pub key: Option<Camelot>,
    /// The energy the track shows, 1 to 10.
    pub energy: Option<u8>,
    /// The track's canonical genre, if it maps to one.
    pub genre: Option<GenreRef>,
}

/// A canonical genre node and its parent (3.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GenreRef {
    pub node: i64,
    pub parent: Option<i64>,
}

/// The names and shared tags the reasons need; filled only for the signals
/// that count.
#[derive(Debug, Clone, Default)]
pub struct Vocab {
    /// Genre node names.
    pub genre_names: HashMap<i64, String>,
    /// The chosen track's tags by name, in a fixed order.
    pub target_tags: Vec<String>,
    /// For each track sharing a tag with the chosen one, which of
    /// `target_tags` (by position, ascending) it shares.
    pub shared_tags: HashMap<i64, Vec<usize>>,
    /// Whether any track has a tag at all. A user with none has nothing to
    /// be told is missing, so tags are then left out of the "not used" list.
    /// Read only when the chosen track has no tags.
    pub any_tags: bool,
}

/// A track to rank: its id and what it is compared by.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Candidate {
    pub id: i64,
    pub facts: Facts,
}

/// Which reason, as the key the screen words it by: `next:reason.<code>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum ReasonCode {
    /// The same key (`from`).
    KeySame,
    /// One number up or down, the same letter (`from`, `to`).
    KeyAdjacent,
    /// The same number, the other letter: relative major or minor.
    KeyRelative,
    /// Two numbers up or down, the same letter. Counted only when wide.
    KeyTwoSteps,
    /// One number up or down, the other letter. Counted only when wide.
    KeyDiagonal,
    /// Any other key.
    KeyFar,
    /// The candidate has no key.
    KeyNone,
    /// `delta` BPM above or below, read as it is.
    Bpm,
    /// `delta` BPM, the candidate at half the chosen track's tempo.
    BpmHalf,
    /// `delta` BPM, the candidate at double the chosen track's tempo.
    BpmDouble,
    /// The candidate has no BPM.
    BpmNone,
    /// The shown energy is `step` above (positive) or below the chosen
    /// track's.
    EnergyStep,
    /// The candidate has no energy, which is why it is last.
    EnergyNone,
    /// Played right after the chosen track `count` times.
    PlayedAfter,
    /// The same genre (`from`).
    GenreSame,
    /// A genre next to the chosen track's: a sibling, parent or child
    /// (`from`, `to`).
    GenreRelated,
    /// `tags` in common (`count` of them).
    TagsShared,
    /// The audio model hears the two as `percent` alike. Always shown as the
    /// audio model's opinion, and never without a fact reason beside it.
    Similar,
}

/// One reason a candidate ranks where it does, as data for the screen to
/// word. Only the fields its `code` names are set.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
#[serde(rename = "NextReason")]
pub struct Reason {
    pub code: ReasonCode,
    /// Counts for the candidate (a fitness of at least a half).
    pub good: bool,
    /// Keys as Camelot (`8A`); the screen shows them in the user's notation.
    pub from: Option<String>,
    pub to: Option<String>,
    pub delta: Option<f64>,
    pub step: Option<i32>,
    pub count: Option<u32>,
    /// The tags in common, for `TagsShared`.
    pub tags: Option<Vec<String>>,
    /// How alike the two sound, 0 to 100, for `Similar`.
    pub percent: Option<u8>,
}

impl Reason {
    fn new(code: ReasonCode, fitness: f64) -> Reason {
        Reason {
            code,
            good: fitness >= 0.5,
            from: None,
            to: None,
            delta: None,
            step: None,
            count: None,
            tags: None,
            percent: None,
        }
    }
}

/// A signal beyond the built-in ones, to be weighed with them (a hook for a
/// later one). Genre, tags and the audio model's similarity are built in,
/// because they are reported as not used and need a weight setting.
pub trait ExtraSignal {
    /// How the candidate fits the chosen track on this signal, or `None`
    /// when it does not apply to the pair.
    fn part(&self, target: i64, candidate: i64) -> Option<ExtraPart>;
}

/// What an [`ExtraSignal`] says about one candidate.
#[derive(Debug, Clone, PartialEq)]
pub struct ExtraPart {
    /// Counts like the weights, 0 to 10.
    pub weight: u8,
    /// 0 to 1.
    pub fitness: f64,
    pub reason: Reason,
}

/// What to rank for.
pub struct Ask<'a> {
    pub target_id: i64,
    pub target: Facts,
    pub direction: Direction,
    pub settings: &'a Settings,
    /// How often each track was played right after the chosen one.
    pub played_after: &'a HashMap<i64, u32>,
    pub extras: &'a [&'a dyn ExtraSignal],
    pub vocab: &'a Vocab,
    /// How each track sounds against the chosen one.
    pub resemblance: &'a Resemblance,
}

/// A ranked candidate.
#[derive(Debug, Clone, PartialEq)]
pub struct Scored {
    pub id: i64,
    /// 0 to 100.
    pub score: u8,
    pub reasons: Vec<Reason>,
}

/// The best candidates, and what could not be compared.
#[derive(Debug, Clone, PartialEq)]
pub struct Ranking {
    pub results: Vec<Scored>,
    pub ignored: Vec<Signal>,
}

/// The relation of a candidate's key to the chosen track's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum KeyRelation {
    Same,
    Adjacent,
    Relative,
    TwoSteps,
    Diagonal,
    Far,
}

pub(crate) fn key_relation(target: Camelot, key: Camelot) -> KeyRelation {
    match target.distance(key) {
        (0, true) => KeyRelation::Same,
        (1, true) => KeyRelation::Adjacent,
        (0, false) => KeyRelation::Relative,
        (2, true) => KeyRelation::TwoSteps,
        (1, false) => KeyRelation::Diagonal,
        _ => KeyRelation::Far,
    }
}

pub(crate) fn key_fitness(relation: KeyRelation, tolerance: Tolerance) -> f64 {
    use KeyRelation::*;
    match (tolerance, relation) {
        (_, Same) => 1.0,
        // As Sort by fit: a neighbor is two points of three.
        (Tolerance::Tight, Adjacent | Relative) => 2.0 / 3.0,
        (Tolerance::Wide, Adjacent | Relative) => 0.8,
        (Tolerance::Wide, TwoSteps) => 0.5,
        (Tolerance::Wide, Diagonal) => 0.4,
        _ => 0.0,
    }
}

/// Within this many BPM, a tempo has this fitness, when wide.
const WIDE_TEMPO_STEPS: [(f64, f64); 4] = [(3.0, 1.0), (6.0, 0.75), (12.0, 0.5), (20.0, 0.25)];

pub(crate) fn bpm_fitness(gap: f64, tolerance: Tolerance) -> f64 {
    let gap = gap.abs();
    match tolerance {
        // As Sort by fit: 3, 2 and 1 points of 3.
        Tolerance::Tight => TEMPO_STEPS
            .iter()
            .find(|(within, _)| gap <= *within)
            .map_or(0.0, |(_, points)| f64::from(*points) / 3.0),
        Tolerance::Wide => WIDE_TEMPO_STEPS
            .iter()
            .find(|(within, _)| gap <= *within)
            .map_or(0.0, |(_, fitness)| *fitness),
    }
}

pub(crate) fn energy_fitness(step: i32, direction: Direction, tolerance: Tolerance) -> f64 {
    let ideal = match direction {
        Direction::Up => 1,
        Direction::Same => 0,
        Direction::Down => -1,
    };
    let falloff: &[f64] = match tolerance {
        Tolerance::Tight => &[1.0, 0.5],
        Tolerance::Wide => &[1.0, 0.75, 0.5, 0.25],
    };
    falloff
        .get((step - ideal).unsigned_abs() as usize)
        .copied()
        .unwrap_or(0.0)
}

/// How a played count scores: once is half, twice three quarters, and so on.
fn played_fitness(count: u32) -> f64 {
    1.0 - 0.5_f64.powi(i32::try_from(count.min(30)).unwrap_or(30))
}

/// One signal's weight, fitness and reason for a candidate; the reason is
/// text fields are filled only for the results that are shown.
struct Part {
    signal: Option<Signal>,
    weight: u8,
    fitness: f64,
    reason: Reason,
}

/// Which signals apply, from the chosen track and the settings.
struct Plan<'a> {
    key: Option<Camelot>,
    bpm: Option<f64>,
    energy: Option<u8>,
    played: bool,
    genre: Option<GenreRef>,
    tags: bool,
    /// The cosines, when sounds similar counts.
    similar: Option<&'a HashMap<i64, f32>>,
    ignored: Vec<Signal>,
}

fn plan<'a>(ask: &'a Ask) -> Plan<'a> {
    let w = &ask.settings.weights;
    let mut ignored = Vec::new();
    let mut want = |signal: Signal, weight: u8, has: bool| {
        if weight > 0 && !has {
            ignored.push(signal);
        }
        weight > 0 && has
    };
    let key = want(Signal::Key, w.key, ask.target.key.is_some())
        .then_some(ask.target.key)
        .flatten();
    let bpm = want(Signal::Bpm, w.bpm, ask.target.bpm.is_some())
        .then_some(ask.target.bpm)
        .flatten();
    let energy = want(Signal::Energy, w.energy, ask.target.energy.is_some())
        .then_some(ask.target.energy)
        .flatten();
    let played = want(Signal::Played, w.played, !ask.played_after.is_empty());
    let genre = want(Signal::Genre, w.genre, ask.target.genre.is_some())
        .then_some(ask.target.genre)
        .flatten();
    let tags = if w.tags == 0 {
        false
    } else if ask.vocab.target_tags.is_empty() {
        // Not used; said so only to a user who has tags somewhere.
        if ask.vocab.any_tags {
            ignored.push(Signal::Tags);
        }
        false
    } else {
        true
    };
    let similar = match (w.similar, ask.resemblance) {
        (0, _) | (_, Resemblance::NotWeighed) => None,
        (_, Resemblance::To(cosines)) => Some(cosines),
        (_, Resemblance::ModelOff | Resemblance::NoFingerprint) => {
            ignored.push(Signal::Similar);
            None
        }
    };
    Plan {
        key,
        bpm,
        energy,
        played,
        genre,
        tags,
        similar,
        ignored,
    }
}

#[cfg(test)]
thread_local! {
    /// How many key reasons had their names written (tests count these).
    pub(super) static NAMED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// How a candidate's genre fits the chosen track's: the same node, a node
/// beside it, or nothing.
fn genre_part(ask: &Ask, target: GenreRef, c: &Candidate, names: bool) -> Part {
    let w = ask.settings.weights.genre;
    let name = |node: i64| ask.vocab.genre_names.get(&node).cloned();
    let (code, fitness) = match c.facts.genre {
        None => (None, 0.0),
        Some(g) if g.node == target.node => (Some(ReasonCode::GenreSame), 1.0),
        Some(g)
            if (g.parent.is_some() && g.parent == target.parent)
                || g.parent == Some(target.node)
                || target.parent == Some(g.node) =>
        {
            (Some(ReasonCode::GenreRelated), 0.5)
        }
        Some(_) => (None, 0.0),
    };
    let mut reason = Reason::new(code.unwrap_or(ReasonCode::GenreRelated), fitness);
    if names && code.is_some() {
        reason.from = name(target.node);
        reason.to = c.facts.genre.and_then(|g| name(g.node));
    }
    Part {
        signal: Some(Signal::Genre),
        weight: w,
        fitness,
        reason,
    }
}

/// How many of the chosen track's tags the candidate has.
fn tags_part(ask: &Ask, c: &Candidate, names: bool) -> Part {
    let vocab = ask.vocab;
    let shared = vocab.shared_tags.get(&c.id).map_or(&[][..], Vec::as_slice);
    let most = vocab.target_tags.len().clamp(1, 3);
    let fitness = (shared.len() as f64 / most as f64).min(1.0);
    let mut reason = Reason::new(ReasonCode::TagsShared, fitness);
    reason.count = u32::try_from(shared.len()).ok();
    if names {
        reason.tags = Some(
            shared
                .iter()
                .filter_map(|&n| vocab.target_tags.get(n).cloned())
                .collect(),
        );
    }
    Part {
        signal: Some(Signal::Tags),
        weight: ask.settings.weights.tags,
        fitness,
        reason,
    }
}

/// The parts for one candidate. `names` says whether the reason text fields
/// (the key names) are filled; the order needs only the fitness.
fn parts(ask: &Ask, plan: &Plan<'_>, c: &Candidate, names: bool) -> Vec<Part> {
    let w: &Weights = &ask.settings.weights;
    let tolerance = ask.settings.tolerance;
    let mut out = Vec::with_capacity(6);
    if let Some(target) = plan.key {
        out.push(match c.facts.key {
            None => Part {
                signal: Some(Signal::Key),
                weight: w.key,
                fitness: 0.0,
                reason: Reason::new(ReasonCode::KeyNone, 0.0),
            },
            Some(key) => {
                let relation = key_relation(target, key);
                let fitness = key_fitness(relation, tolerance);
                let code = match relation {
                    KeyRelation::Same => ReasonCode::KeySame,
                    KeyRelation::Adjacent => ReasonCode::KeyAdjacent,
                    KeyRelation::Relative => ReasonCode::KeyRelative,
                    KeyRelation::TwoSteps => ReasonCode::KeyTwoSteps,
                    KeyRelation::Diagonal => ReasonCode::KeyDiagonal,
                    KeyRelation::Far => ReasonCode::KeyFar,
                };
                let mut reason = Reason::new(code, fitness);
                if names {
                    #[cfg(test)]
                    NAMED.with(|n| n.set(n.get() + 1));
                    reason.from = Some(target.name());
                    reason.to = Some(key.name());
                }
                Part {
                    signal: Some(Signal::Key),
                    weight: w.key,
                    fitness,
                    reason,
                }
            }
        });
    }
    if let Some(target) = plan.bpm {
        out.push(match c.facts.bpm {
            None => Part {
                signal: Some(Signal::Bpm),
                weight: w.bpm,
                fitness: 0.0,
                reason: Reason::new(ReasonCode::BpmNone, 0.0),
            },
            Some(bpm) => {
                let (delta, time) = tempo_gap(target, bpm);
                let fitness = bpm_fitness(delta, tolerance);
                let code = match time {
                    TimeFeel::Straight => ReasonCode::Bpm,
                    TimeFeel::Half => ReasonCode::BpmHalf,
                    TimeFeel::Double => ReasonCode::BpmDouble,
                };
                let mut reason = Reason::new(code, fitness);
                reason.delta = Some(delta);
                Part {
                    signal: Some(Signal::Bpm),
                    weight: w.bpm,
                    fitness,
                    reason,
                }
            }
        });
    }
    if let Some(target) = plan.energy {
        if let Some(energy) = c.facts.energy {
            let step = i32::from(energy) - i32::from(target);
            let fitness = energy_fitness(step, ask.direction, tolerance);
            let mut reason = Reason::new(ReasonCode::EnergyStep, fitness);
            reason.step = Some(step);
            out.push(Part {
                signal: Some(Signal::Energy),
                weight: w.energy,
                fitness,
                reason,
            });
        }
        // No energy: left out of the average, and the candidate goes last.
    }
    if plan.played {
        let count = ask.played_after.get(&c.id).copied().unwrap_or(0);
        if count > 0 {
            let fitness = played_fitness(count);
            let mut reason = Reason::new(ReasonCode::PlayedAfter, fitness);
            reason.count = Some(count);
            out.push(Part {
                signal: Some(Signal::Played),
                weight: w.played,
                fitness,
                reason,
            });
        } else {
            // Nothing played: counts for nothing, and has nothing to say.
            out.push(Part {
                signal: Some(Signal::Played),
                weight: w.played,
                fitness: 0.0,
                reason: Reason::new(ReasonCode::PlayedAfter, 0.0),
            });
        }
    }
    if let Some(target) = plan.genre {
        out.push(genre_part(ask, target, c, names));
    }
    if plan.tags {
        out.push(tags_part(ask, c, names));
    }
    if let Some(cosines) = plan.similar {
        // Never the only reason: only beside a fact that counts for something.
        let beside_a_fact = out.iter().any(|p| p.weight > 0 && p.fitness > 0.0);
        if let (true, Some(&cosine)) = (beside_a_fact, cosines.get(&c.id)) {
            let fitness = f64::from(likeness_of(cosine));
            let mut reason = Reason::new(ReasonCode::Similar, fitness);
            reason.percent = Some((fitness * 100.0).round() as u8);
            out.push(Part {
                signal: Some(Signal::Similar),
                weight: w.similar,
                fitness,
                reason,
            });
        }
    }
    for extra in ask.extras {
        if let Some(p) = extra.part(ask.target_id, c.id) {
            // A bad extra is "not used": NaN or a fitness outside 0..=1 is
            // skipped, and the weight is capped like the settings are.
            let usable = p.fitness.is_finite() && (0.0..=1.0).contains(&p.fitness);
            if p.weight > 0 && usable {
                out.push(Part {
                    signal: None,
                    weight: p.weight.min(MOST_WEIGHT),
                    fitness: p.fitness,
                    reason: p.reason,
                });
            }
        }
    }
    out
}

/// Whether the candidate goes last for lacking energy.
fn lacks_energy(plan: &Plan<'_>, c: &Candidate) -> bool {
    plan.energy.is_some() && c.facts.energy.is_none()
}

fn score_of(parts: &[Part]) -> f64 {
    let weights: f64 = parts.iter().map(|p| f64::from(p.weight)).sum();
    if weights <= 0.0 {
        return 0.0;
    }
    parts
        .iter()
        .map(|p| f64::from(p.weight) * p.fitness)
        .sum::<f64>()
        / weights
}

/// The reasons to show: energy missing first, then by contribution; what
/// counted for nothing (nothing played, no genre or tag in common) is never a
/// reason.
fn reasons_of(plan: &Plan<'_>, c: &Candidate, parts: Vec<Part>) -> Vec<Reason> {
    let mut out = Vec::with_capacity(MOST_REASONS);
    if lacks_energy(plan, c) {
        out.push(Reason::new(ReasonCode::EnergyNone, 0.0));
    }
    let mut rest: Vec<Part> = parts
        .into_iter()
        .filter(|p| match p.signal {
            Some(Signal::Played) => p.reason.count.is_some(),
            Some(Signal::Genre | Signal::Tags | Signal::Similar) => p.fitness > 0.0,
            _ => true,
        })
        .collect();
    // Stable: equal contributions keep the signals' order.
    rest.sort_by(|a, b| {
        (f64::from(b.weight) * b.fitness).total_cmp(&(f64::from(a.weight) * a.fitness))
    });
    let room = MOST_REASONS.saturating_sub(out.len());
    // A fact that counts for something must be among the reasons beside the
    // audio model's: if the best few are all other kinds, it takes the last
    // place (an extra signal could crowd it out).
    let is_fact = |p: &Part| p.fitness > 0.0 && matches!(p.signal, Some(s) if s != Signal::Similar);
    if room > 0 && room < rest.len() && !rest[..room].iter().any(is_fact) {
        if let Some(at) = rest.iter().position(is_fact) {
            rest.swap(room - 1, at);
        }
    }
    rest.truncate(room);
    if !rest.iter().any(is_fact) {
        // No fact to stand beside it: the audio model's opinion is left out.
        rest.retain(|p| p.signal != Some(Signal::Similar));
    }
    out.extend(rest.into_iter().map(|p| p.reason));
    out
}

/// The `most` candidates that best follow the chosen track, best first. The
/// chosen track itself is never among them.
pub fn rank(ask: &Ask, candidates: &[Candidate], most: usize) -> Ranking {
    let plan = plan(ask);
    // The order needs only a number per candidate; reasons are worked out for
    // the few that are kept.
    let mut order: Vec<(bool, f64, usize)> = candidates
        .iter()
        .enumerate()
        .filter(|(_, c)| c.id != ask.target_id)
        .map(|(n, c)| {
            let p = parts(ask, &plan, c, false);
            (lacks_energy(&plan, c), score_of(&p), n)
        })
        .collect();
    let by_rank = |a: &(bool, f64, usize), b: &(bool, f64, usize)| -> Ordering {
        a.0.cmp(&b.0)
            .then_with(|| b.1.total_cmp(&a.1))
            .then_with(|| candidates[a.2].id.cmp(&candidates[b.2].id))
    };
    if order.len() > most {
        order.select_nth_unstable_by(most, by_rank);
        order.truncate(most);
    }
    order.sort_by(by_rank);
    let results = order
        .into_iter()
        .map(|(_, score, n)| {
            let c = &candidates[n];
            Scored {
                id: c.id,
                score: (score * 100.0).round().clamp(0.0, 100.0) as u8,
                reasons: reasons_of(&plan, c, parts(ask, &plan, c, true)),
            }
        })
        .collect();
    Ranking {
        results,
        ignored: plan.ignored,
    }
}
