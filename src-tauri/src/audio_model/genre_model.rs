//! Your own genre model (ROADMAP 3.8, 5B-B3a): what each of your genres
//! sounds like, learned from your tracks' fingerprints, so the audio model
//! can name your genres and not only AudioSet's.
//!
//! - **What it learns from.** Library tracks whose shown file has a current
//!   fingerprint (the current model, matching `audio_hash`) and a canonical
//!   genre node (`track_row.genre_node`, the exact node, no roll-up to its
//!   parent). Only genres with at least [`MIN_TRACKS`] such tracks count;
//!   with fewer than [`MIN_GENRES`] of them there is no model, and that
//!   answer is stored too.
//! - **The method.** One logistic regression per genre against all the other
//!   learned tracks (one-vs-rest) on the 192-number fingerprint, as the
//!   4C-M1 measurement did (57 % right on 19 genres with CED-tiny). Each
//!   number is standardised first: the training tracks' mean taken off and
//!   divided by their standard deviation (a constant number is dropped). The
//!   mean and the scale are stored with the weights, because a track is
//!   scored the same way. This is not the mean-centring and unit length
//!   "Sounds like" uses: a distance wants every direction to count alike, a
//!   regression wants every number on one scale.
//! - **Regularisation and weighting.** L2, `C = 0.1` as in scikit-learn
//!   (loss = half the squared weights + `C` times the summed log-loss), and
//!   the bias is not regularised. Each genre is fitted with balanced weights:
//!   its tracks together count as much as all the others together, or a
//!   genre of 10 tracks among 5,000 would never reach the threshold. So the
//!   percent shown is a confidence of the model, not how often the genre
//!   occurs.
//! - **Reproducible.** No randomness: zero start, a fixed order, Nesterov
//!   gradient steps of one over a bound on the curvature, a fixed number of
//!   iterations at most and a stop when the gradient is small. The same
//!   tracks give the same bytes.
//! - **When.** Only while the audio model is switched on, by a job of its own
//!   at the lowest priority ([`learn_job`]): after the embed job ends, after
//!   the index job ends (which writes `genre_node`, so changes to a track's
//!   genre and to the genre tree land there) and at startup. It learns again
//!   only when the [signature](signature) of what it would learn from
//!   changed: the model, the method, and every track and genre used with the
//!   identity of its fingerprint. Switching the model off stops the job and
//!   keeps what was learned, like the embeddings.
//! - **Reading.** [`your_genres`] scores a track's current fingerprint with
//!   the stored weights: at most [`MOST_GENRES`] genres whose sigmoid is at
//!   least [`THRESHOLD`], best first. It is empty with the switch off, with no
//!   model, with a model of another embedding model, or with no fingerprint.
//!   Reads only; the model is never loaded.

use std::collections::BTreeMap;

use rusqlite::{Connection, OptionalExtension};
use serde::Serialize;
use sha2::{Digest, Sha256};
use specta::Type;
use tauri::State;

use super::guess::{MOST_GENRES, THRESHOLD};
use super::panel::{fingerprint_of, shown, standing, PanelState, Standing};
use super::subset;
use super::EMBEDDING_LEN;
use crate::db::{DbError, ReadPool, Writer};
use crate::ipc::IpcError;
use crate::jobs::{JobContext, JobError, JobHandler, JobId, JobKind, NewJob, Priority};
use crate::scan::chain;
use crate::settings::{read_feature_enabled, Feature};

/// The fewest tracks a genre needs to be learned.
pub const MIN_TRACKS: usize = 10;

/// The fewest genres a model needs: one genre alone has nothing to be told
/// apart from.
pub const MIN_GENRES: usize = 2;

/// The job's priority: just under the embed job's, the lowest there is.
pub const PRIORITY: Priority = Priority(super::PRIORITY.0 - 1);

/// Changing the method (the preprocessing, the weighting, the settings
/// below) means everything is learned again: this goes into the signature.
const METHOD_VERSION: u8 = 1;

/// The regularisation: scikit-learn's `C`, as 4C-M1 used.
const C: f64 = 0.1;

/// The most gradient steps a genre takes.
const MOST_STEPS: usize = 300;

/// A genre is done when the gradient (of the mean loss) is shorter than this.
const SMALL_GRADIENT: f64 = 1e-4;

/// A feature whose standard deviation is under this is the same in every
/// track and is dropped (its scale is 0).
const FLAT: f64 = 1e-6;

/// A genre learning job.
pub fn learn_job() -> NewJob {
    NewJob::new(JobKind::GenreModel).priority(PRIORITY)
}

// ---- The data ---------------------------------------------------------------------

/// One track the model could learn from.
struct Example {
    recording: i64,
    node: i64,
    file: i64,
    audio_hash: Vec<u8>,
    /// The fingerprint as stored; empty when only the signature is wanted.
    fingerprint: Vec<u8>,
}

/// Library tracks with a current fingerprint and a canonical genre node,
/// by track id. With `blobs` false the fingerprints are left unread: the
/// signature needs only their identity.
fn candidates(conn: &Connection, blobs: bool) -> rusqlite::Result<Vec<Example>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT lt.recording_id, tr.genre_node, f.id, f.audio_hash, {fingerprint}
         FROM library_track lt
         JOIN track_row tr ON tr.recording_id = lt.recording_id
         JOIN genre g ON g.id = tr.genre_node
         JOIN file f ON f.id = {shown}
         JOIN embedding e ON e.file_id = f.id AND e.audio_hash = f.audio_hash
                         AND e.model_id = ?1
         ORDER BY lt.recording_id",
        fingerprint = if blobs { "e.fingerprint" } else { "x''" },
        shown = shown()
    ))?;
    let rows = stmt.query_map([subset::model_id()], |r| {
        Ok(Example {
            recording: r.get(0)?,
            node: r.get(1)?,
            file: r.get(2)?,
            audio_hash: r.get(3)?,
            fingerprint: r.get(4)?,
        })
    })?;
    rows.collect()
}

/// The genres with at least [`MIN_TRACKS`] of `rows`, and how many each has;
/// empty if there are fewer than [`MIN_GENRES`].
fn enough(rows: &[Example]) -> BTreeMap<i64, usize> {
    let mut counts: BTreeMap<i64, usize> = BTreeMap::new();
    for row in rows {
        *counts.entry(row.node).or_default() += 1;
    }
    counts.retain(|_, n| *n >= MIN_TRACKS);
    if counts.len() < MIN_GENRES {
        counts.clear();
    }
    counts
}

/// Names what would be learned from `rows`: the model, the method, and every
/// track and genre node that counts, with the identity of its fingerprint.
/// Equal signatures learn equal models. When there is too little to learn
/// from, only the model and the method count.
fn signature(model_id: &str, rows: &[Example]) -> String {
    let counts = enough(rows);
    let mut hash = Sha256::new();
    hash.update(b"genre-model");
    hash.update([METHOD_VERSION]);
    hash.update((model_id.len() as u64).to_le_bytes());
    hash.update(model_id.as_bytes());
    for row in rows.iter().filter(|r| counts.contains_key(&r.node)) {
        hash.update(row.recording.to_le_bytes());
        hash.update(row.node.to_le_bytes());
        hash.update(row.file.to_le_bytes());
        hash.update((row.audio_hash.len() as u64).to_le_bytes());
        hash.update(&row.audio_hash);
    }
    hash.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// The signature of what the database holds now.
fn current_signature(conn: &Connection) -> rusqlite::Result<String> {
    Ok(signature(&subset::model_id(), &candidates(conn, false)?))
}

/// The signature of the model stored, if there is one.
fn stored_signature(conn: &Connection) -> rusqlite::Result<Option<String>> {
    conn.query_row("SELECT signature FROM genre_model WHERE id = 1", [], |r| {
        r.get(0)
    })
    .optional()
}

/// Whether the model should be learned now: the switch is on and what it
/// was learned from is not what is there. False while the switch is off.
pub(crate) fn due(conn: &Connection) -> rusqlite::Result<bool> {
    if !read_feature_enabled(conn, Feature::AudioModel)? {
        return Ok(false);
    }
    Ok(stored_signature(conn)?.as_deref() != Some(current_signature(conn)?.as_str()))
}

/// Queues the learning job if the switch is on and the model is out of date,
/// unless one is queued already (a running one is asked to run once more).
pub(crate) fn request<E: From<DbError>>(
    writer: &Writer,
    enqueue: impl FnOnce(NewJob) -> Result<JobId, E>,
) -> Result<(), E> {
    if writer.call(|c| due(c))? {
        chain::queue_once(writer, learn_job(), enqueue)?;
    }
    Ok(())
}

// ---- Learning ---------------------------------------------------------------------

/// A track and the genre it is in, for [`fit`].
pub(crate) struct Sample {
    pub node: i64,
    /// [`EMBEDDING_LEN`] finite numbers.
    pub fingerprint: Vec<f32>,
}

/// What was learned for one genre.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct NodeFit {
    pub node: i64,
    pub weights: Vec<f32>,
    pub bias: f32,
    /// How many tracks of the genre it was learned from.
    pub trained_on: usize,
}

/// A learned model: the preprocessing and one fit per genre, by node id.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Fit {
    pub mean: Vec<f32>,
    pub scale: Vec<f32>,
    pub nodes: Vec<NodeFit>,
    /// How many tracks were learned from, in all.
    pub tracks: usize,
}

/// Learning was stopped (cancelled) before it ended.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Stopped;

/// The logit of `fingerprint` for a genre: standardise, then the weights.
pub(crate) fn logit(
    mean: &[f32],
    scale: &[f32],
    weights: &[f32],
    bias: f32,
    fingerprint: &[f32],
) -> f64 {
    let mut sum = f64::from(bias);
    for i in 0..EMBEDDING_LEN {
        sum += f64::from(weights[i]) * f64::from((fingerprint[i] - mean[i]) * scale[i]);
    }
    sum
}

fn sigmoid(logit: f64) -> f64 {
    1.0 / (1.0 + (-logit).exp())
}

/// Eight sums at once so the compiler can use vector instructions; the order
/// of the additions is fixed, so the result is the same on every run.
fn dot(a: &[f32], b: &[f32]) -> f32 {
    let mut lanes = [0f32; 8];
    let ((a8, a_tail), (b8, b_tail)) = (a.as_chunks::<8>(), b.as_chunks::<8>());
    let tail: f32 = a_tail.iter().zip(b_tail).map(|(x, y)| x * y).sum();
    for (x, y) in a8.iter().zip(b8) {
        for k in 0..8 {
            lanes[k] += x[k] * y[k];
        }
    }
    lanes.iter().sum::<f32>() + tail
}

/// What one genre's fit works on: the standardised tracks (`rows` of
/// [`EMBEDDING_LEN`] numbers, flat) and which are in the genre.
struct Problem<'a> {
    x: &'a [f32],
    in_genre: &'a [bool],
}

impl Problem<'_> {
    fn rows(&self) -> usize {
        self.in_genre.len()
    }

    fn row(&self, i: usize) -> &[f32] {
        &self.x[i * EMBEDDING_LEN..(i + 1) * EMBEDDING_LEN]
    }
}

/// The weights of `problem`, balanced: the genre's tracks count as much as
/// the others together, `n` in all.
fn sample_weights(problem: &Problem<'_>) -> (f64, f64) {
    let n = problem.rows() as f64;
    let inside = problem.in_genre.iter().filter(|b| **b).count() as f64;
    (n / (2.0 * inside), n / (2.0 * (n - inside)))
}

/// A bound on how sharply the loss curves: a quarter of the largest
/// eigenvalue of the weighted second moment of `[x, 1]`, found by power
/// iteration from a fixed start, plus the regularisation. Taken a fifth too
/// large, since the iteration only approaches the eigenvalue from below.
fn curvature(problem: &Problem<'_>, weight_in: f64, weight_out: f64, lambda: f64) -> f64 {
    let n = problem.rows();
    let d = EMBEDDING_LEN;
    let mut v = vec![1.0 / ((d + 1) as f64).sqrt(); d + 1];
    let mut largest = 0.0;
    for _ in 0..30 {
        let v32: Vec<f32> = v[..d].iter().map(|x| *x as f32).collect();
        let mut next = vec![0f64; d + 1];
        for i in 0..n {
            let s = if problem.in_genre[i] {
                weight_in
            } else {
                weight_out
            };
            let row = problem.row(i);
            let u = f64::from(dot(row, &v32)) + v[d];
            let c = s * u / n as f64;
            for k in 0..d {
                next[k] += c * f64::from(row[k]);
            }
            next[d] += c;
        }
        let length = next.iter().map(|x| x * x).sum::<f64>().sqrt();
        if length <= 0.0 {
            break;
        }
        largest = length;
        v = next.iter().map(|x| x / length).collect();
    }
    0.25 * largest * 1.2 + lambda
}

/// Fits one genre: weights and bias in the standardised space.
fn fit_genre(
    problem: &Problem<'_>,
    report: &mut dyn FnMut(f64) -> Result<(), Stopped>,
    (from, span): (f64, f64),
) -> Result<(Vec<f32>, f32), Stopped> {
    let n = problem.rows();
    let d = EMBEDDING_LEN;
    let (weight_in, weight_out) = sample_weights(problem);
    // sklearn's loss, divided by C and the summed weights (n): the mean
    // loss plus lambda / 2 times the squared weights.
    let lambda = 1.0 / (C * n as f64);
    let step = 1.0 / curvature(problem, weight_in, weight_out, lambda);
    let (mut w, mut b) = (vec![0f64; d], 0f64);
    let (mut last_w, mut last_b) = (w.clone(), b);
    for k in 0..MOST_STEPS {
        report(from + span * k as f64 / MOST_STEPS as f64)?;
        let momentum = k as f64 / (k as f64 + 3.0);
        let ahead_w: Vec<f64> = w
            .iter()
            .zip(&last_w)
            .map(|(a, p)| a + momentum * (a - p))
            .collect();
        let ahead_b = b + momentum * (b - last_b);
        let ahead32: Vec<f32> = ahead_w.iter().map(|x| *x as f32).collect();
        let mut gradient = vec![0f64; d];
        let mut gradient_b = 0f64;
        for i in 0..n {
            let row = problem.row(i);
            let (target, s) = if problem.in_genre[i] {
                (1.0, weight_in)
            } else {
                (0.0, weight_out)
            };
            let z = f64::from(dot(row, &ahead32)) + ahead_b;
            let c = s * (sigmoid(z) - target) / n as f64;
            for j in 0..d {
                gradient[j] += c * f64::from(row[j]);
            }
            gradient_b += c;
        }
        for j in 0..d {
            gradient[j] += lambda * ahead_w[j];
        }
        let length = (gradient.iter().map(|g| g * g).sum::<f64>() + gradient_b * gradient_b).sqrt();
        if length < SMALL_GRADIENT {
            w = ahead_w;
            b = ahead_b;
            break;
        }
        last_w = std::mem::take(&mut w);
        last_b = b;
        w = ahead_w
            .iter()
            .zip(&gradient)
            .map(|(a, g)| a - step * g)
            .collect();
        b = ahead_b - step * gradient_b;
    }
    Ok((w.iter().map(|x| *x as f32).collect(), b as f32))
}

/// Learns a model from `samples`, or `None` when no [`MIN_GENRES`] genres
/// have [`MIN_TRACKS`] samples each. `report` is told how far along it is,
/// from 0 to 1, between steps, and may stop it. Same samples in the same
/// order, same model.
pub(crate) fn fit(
    samples: &[Sample],
    report: &mut dyn FnMut(f64) -> Result<(), Stopped>,
) -> Result<Option<Fit>, Stopped> {
    let mut counts: BTreeMap<i64, usize> = BTreeMap::new();
    for s in samples {
        *counts.entry(s.node).or_default() += 1;
    }
    counts.retain(|_, n| *n >= MIN_TRACKS);
    if counts.len() < MIN_GENRES {
        return Ok(None);
    }
    let kept: Vec<&Sample> = samples
        .iter()
        .filter(|s| counts.contains_key(&s.node))
        .collect();
    let n = kept.len();
    let d = EMBEDDING_LEN;
    // The preprocessing, from these tracks only.
    let mut mean = vec![0f64; d];
    for s in &kept {
        for (m, x) in mean.iter_mut().zip(&s.fingerprint) {
            *m += f64::from(*x);
        }
    }
    mean.iter_mut().for_each(|m| *m /= n as f64);
    let mut variance = vec![0f64; d];
    for s in &kept {
        for ((v, x), m) in variance.iter_mut().zip(&s.fingerprint).zip(&mean) {
            *v += (f64::from(*x) - m).powi(2);
        }
    }
    let mean: Vec<f32> = mean.iter().map(|m| *m as f32).collect();
    let scale: Vec<f32> = variance
        .iter()
        .map(|v| {
            let deviation = (v / n as f64).sqrt();
            if deviation > FLAT {
                (1.0 / deviation) as f32
            } else {
                0.0
            }
        })
        .collect();
    let mut x = Vec::with_capacity(n * d);
    for s in &kept {
        for i in 0..d {
            x.push((s.fingerprint[i] - mean[i]) * scale[i]);
        }
    }
    let genres = counts.len();
    let mut nodes = Vec::with_capacity(genres);
    for (g, (&node, &trained_on)) in counts.iter().enumerate() {
        let in_genre: Vec<bool> = kept.iter().map(|s| s.node == node).collect();
        let problem = Problem {
            x: &x,
            in_genre: &in_genre,
        };
        let span = 1.0 / genres as f64;
        let (weights, bias) = fit_genre(&problem, report, (g as f64 * span, span))?;
        nodes.push(NodeFit {
            node,
            weights,
            bias,
            trained_on,
        });
    }
    Ok(Some(Fit {
        mean,
        scale,
        nodes,
        tracks: n,
    }))
}

fn blob_of(values: &[f32]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_le_bytes()).collect()
}

/// Replaces the stored model with `fit` (or with "too little to learn from"),
/// as one step, and says it was learned from `signature`. A genre deleted
/// since the tracks were read is left out. Does nothing if the switch has
/// been turned off meanwhile.
fn store(
    conn: &mut Connection,
    model_id: &str,
    signature: &str,
    fit: Option<&Fit>,
) -> rusqlite::Result<bool> {
    let tx = conn.transaction()?;
    if !read_feature_enabled(&tx, Feature::AudioModel)? {
        return Ok(false);
    }
    tx.execute("DELETE FROM genre_model_node", [])?;
    tx.execute("DELETE FROM genre_model", [])?;
    let (mean, scale, genres, tracks) = match fit {
        Some(fit) => (
            blob_of(&fit.mean),
            blob_of(&fit.scale),
            fit.nodes.len(),
            fit.tracks,
        ),
        None => (Vec::new(), Vec::new(), 0, 0),
    };
    tx.execute(
        "INSERT INTO genre_model (id, model_id, signature, mean, scale, genres, tracks)
         VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6)",
        (
            model_id,
            signature,
            mean,
            scale,
            genres as i64,
            tracks as i64,
        ),
    )?;
    for node in fit.map_or(&[][..], |f| &f.nodes[..]) {
        tx.execute(
            "INSERT INTO genre_model_node (genre_id, weights, bias, trained_on, model_id)
             SELECT g.id, ?2, ?3, ?4, ?5 FROM genre g WHERE g.id = ?1",
            (
                node.node,
                blob_of(&node.weights),
                f64::from(node.bias),
                node.trained_on as i64,
                model_id,
            ),
        )?;
    }
    tx.commit()?;
    Ok(true)
}

/// What [`learn`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Learned {
    /// The switch is off: nothing was done.
    Off,
    /// What it would learn from is what it learned from.
    Unchanged,
    /// Fewer than [`MIN_GENRES`] genres have [`MIN_TRACKS`] tracks: no model.
    TooLittle,
    /// A model of this many genres was stored.
    Genres(usize),
}

/// Why [`learn`] ended without a result.
#[derive(Debug)]
pub(crate) enum Interrupted {
    Stopped,
    Db(DbError),
}

impl From<DbError> for Interrupted {
    fn from(e: DbError) -> Self {
        Interrupted::Db(e)
    }
}

impl From<Stopped> for Interrupted {
    fn from(_: Stopped) -> Self {
        Interrupted::Stopped
    }
}

/// Learns the model if it is out of date, as the job does: the database is
/// read and written through `writer` in short steps, and the fitting runs on
/// this thread.
pub(crate) fn learn(
    writer: &Writer,
    report: &mut dyn FnMut(f64) -> Result<(), Stopped>,
) -> Result<Learned, Interrupted> {
    let enabled = writer.call(|c| read_feature_enabled(c, Feature::AudioModel))?;
    if !enabled {
        return Ok(Learned::Off);
    }
    let unchanged = writer.call(|c| Ok(stored_signature(c)? == Some(current_signature(c)?)))?;
    if unchanged {
        return Ok(Learned::Unchanged);
    }
    report(0.0)?;
    // The signature is of the rows that are read, so the model says exactly
    // what it was learned from even if tracks changed meanwhile.
    let model_id = subset::model_id();
    let rows = writer.call(|c| candidates(c, true))?;
    let signature = signature(&model_id, &rows);
    // A fingerprint that is not whole and finite is left out, not fatal.
    let samples: Vec<Sample> = rows
        .iter()
        .filter_map(|row| {
            Some(Sample {
                node: row.node,
                fingerprint: fingerprint_of(&row.fingerprint)?,
            })
        })
        .collect();
    drop(rows);
    let fitted = fit(&samples, report)?;
    let genres = fitted.as_ref().map(|f| f.nodes.len());
    let stored = writer.call({
        let signature = signature.clone();
        move |c| store(c, &model_id, &signature, fitted.as_ref())
    })?;
    Ok(match (stored, genres) {
        (false, _) => Learned::Off,
        (true, None) => Learned::TooLittle,
        (true, Some(n)) => Learned::Genres(n),
    })
}

/// The learning job's handler.
#[derive(Debug, Clone, Copy, Default)]
pub struct GenreLearner;

impl JobHandler for GenreLearner {
    fn run(&self, job: &JobContext) -> Result<(), JobError> {
        let mut failure: Option<JobError> = None;
        let result = learn(job.writer(), &mut |fraction| {
            job.progress(fraction).map_err(|e| {
                failure = Some(e);
                Stopped
            })
        });
        match result {
            Ok(_) => job.progress(1.0),
            Err(Interrupted::Stopped) => Err(failure.unwrap_or(JobError::Cancelled)),
            Err(Interrupted::Db(e)) => Err(e.into()),
        }
    }
}

// ---- Reading ----------------------------------------------------------------------

/// One of your genres the model hears in a track.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct YourGenre {
    /// The genre node (`genre.id`).
    pub node: i64,
    /// Its name in your genre tree.
    pub name: String,
    /// 0 to 100.
    pub percent: u8,
}

/// Why [`YourGenres`] is what it is: `Found` when there is something to show,
/// otherwise the one thing the empty line should say.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum YourGenresReason {
    /// There are genres to show.
    Found,
    /// The audio model switch is off.
    SwitchOff,
    /// Only Library tracks are listened to.
    NotInLibrary,
    /// The model has not learned your genres yet (or learned them for another
    /// embedding model).
    NoModelYet,
    /// The track has no current fingerprint yet.
    NoFingerprintYet,
    /// The model heard the track, and no genre of yours is at least
    /// [`THRESHOLD`] sure.
    NothingStrongEnough,
    /// The track's result or the learned model can't be read.
    CouldntBeRead,
}

/// Your genres the model hears in a track, with the reason when there are none.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct YourGenres {
    /// At most [`MOST_GENRES`], best first.
    pub genres: Vec<YourGenre>,
    /// `Found` exactly when `genres` is not empty.
    pub reason: YourGenresReason,
}

impl YourGenres {
    fn none(reason: YourGenresReason) -> Self {
        Self {
            genres: Vec::new(),
            reason,
        }
    }
}

fn floats(bytes: &[u8]) -> Option<Vec<f32>> {
    subset::from_blob(bytes).filter(|v| v.len() == EMBEDDING_LEN && v.iter().all(|x| x.is_finite()))
}

fn guesses_in(conn: &Connection, fingerprint: &[f32]) -> rusqlite::Result<YourGenres> {
    let model_id = subset::model_id();
    let state: Option<(Vec<u8>, Vec<u8>)> = conn
        .query_row(
            "SELECT mean, scale FROM genre_model WHERE id = 1 AND model_id = ?1 AND genres > 0",
            [&model_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let Some((mean, scale)) = state else {
        return Ok(YourGenres::none(YourGenresReason::NoModelYet));
    };
    let (Some(mean), Some(scale)) = (floats(&mean), floats(&scale)) else {
        return Ok(YourGenres::none(YourGenresReason::CouldntBeRead));
    };
    let mut stmt = conn.prepare_cached(
        "SELECT n.genre_id, g.name, n.weights, n.bias
         FROM genre_model_node n JOIN genre g ON g.id = n.genre_id
         WHERE n.model_id = ?1 ORDER BY n.genre_id",
    )?;
    let mut rows = stmt.query([&model_id])?;
    let mut passing: Vec<(i64, String, f64)> = Vec::new();
    while let Some(row) = rows.next()? {
        let Some(weights) = floats(&row.get::<_, Vec<u8>>(2)?) else {
            continue;
        };
        let bias: f64 = row.get(3)?;
        if !bias.is_finite() {
            continue;
        }
        let p = sigmoid(logit(&mean, &scale, &weights, bias as f32, fingerprint));
        if p.is_finite() && p >= THRESHOLD {
            passing.push((row.get(0)?, row.get(1)?, p));
        }
    }
    // Best first; the lower node id settles a tie.
    passing.sort_by(|a, b| b.2.total_cmp(&a.2).then(a.0.cmp(&b.0)));
    let genres: Vec<YourGenre> = passing
        .into_iter()
        .take(MOST_GENRES)
        .map(|(node, name, p)| YourGenre {
            node,
            name,
            percent: (p * 100.0).round().clamp(0.0, 100.0) as u8,
        })
        .collect();
    let reason = if genres.is_empty() {
        YourGenresReason::NothingStrongEnough
    } else {
        YourGenresReason::Found
    };
    Ok(YourGenres { genres, reason })
}

/// The genres of yours the model hears in track `recording`: at most
/// [`MOST_GENRES`], each at least [`THRESHOLD`] sure, best first. When there
/// are none, the reason says why: the switch is off, the track is not in the
/// Library, there is no model (or it is of another embedding model), the track
/// has no fingerprint yet, nothing is sure enough, or something stored can't be
/// read. A database that can't be read is `CouldntBeRead`: this only suggests.
pub(crate) fn your_genres(conn: &Connection, recording: i64) -> YourGenres {
    let fingerprint = match standing(conn, recording) {
        Ok(Standing::Heard(heard)) => match heard.fingerprint {
            Some(fingerprint) => fingerprint,
            None => return YourGenres::none(YourGenresReason::CouldntBeRead),
        },
        Ok(Standing::Not(state)) => {
            return YourGenres::none(match state {
                PanelState::Off => YourGenresReason::SwitchOff,
                PanelState::NotInLibrary => YourGenresReason::NotInLibrary,
                PanelState::CouldntBeRead => YourGenresReason::CouldntBeRead,
                PanelState::NotListenedYet | PanelState::Ready => {
                    YourGenresReason::NoFingerprintYet
                }
            })
        }
        Err(_) => return YourGenres::none(YourGenresReason::CouldntBeRead),
    };
    guesses_in(conn, &fingerprint)
        .unwrap_or_else(|_| YourGenres::none(YourGenresReason::CouldntBeRead))
}

/// The genres of yours the audio model hears in a track, from the model it
/// learned from your own tracks. Reads only; never loads the audio model.
#[tauri::command]
#[specta::specta]
pub async fn audio_model_your_genres(
    reads: State<'_, ReadPool>,
    recording_id: i64,
) -> Result<YourGenres, IpcError> {
    Ok(reads.read(move |c| Ok(your_genres(c, recording_id)))?)
}
