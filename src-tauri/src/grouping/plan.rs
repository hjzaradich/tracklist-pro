//! Which files go to which track, as a pure function. It reads nothing and
//! writes nothing; [`super::store`] loads its input and applies its answer.
//!
//! # What it is given
//!
//! - Every file with a track, and every present file without one
//!   ([`Member`]): the audio it holds (its [`Key`]), the audio it held when
//!   it was put in its track, and whether a Library track points at it.
//! - What the grouping rules said about pairs of files ([`Pair`]), written
//!   by the deciding run ([`super::decide`]).
//! - What the user decided ([`Remembered`]), and the version links.
//!
//! # A track is files that are duplicates pair by pair
//!
//! 1. **Files with the very same audio** (the same [`Key`]) that sit alone,
//!    or in tracks holding nothing but that audio, share one track. No
//!    decided pair is needed for it.
//! 2. **A set joined by duplicate pairs** (tracks and single files, linked
//!    by a pair the rules call a duplicate, by the same audio, or by the
//!    user's "these are the same") merges only if *every* pair of files in
//!    it, between two of its tracks, is a duplicate. One pair that is a
//!    cut, a rework, a review or different, and nothing in the set merges.
//!    A pair nobody decided yet makes the set wait ([`Plan::missing`] lists
//!    those pairs for the deciding run). A set with more files than the cap
//!    is held as if it disagreed.
//!
//! A set never merges, whatever its pairs say, when:
//!
//! - two of its tracks are both in the Library (owner decision,
//!   ROADMAP 1.4: the user merges those, or keeps both);
//! - the user decided two of its tracks apart;
//! - two of its tracks are linked versions.
//!
//! # Merges are kept
//!
//! A track is never split because a file arrived or was retagged. Only a
//! file whose own audio changed since it was put in its track leaves it
//! (and when a Library track's own file holds other audio now, the other
//! files leave: the track is that file's). So a set whose pairs disagree
//! can group differently depending on the order its files arrived in; a
//! run over an unchanged database changes nothing.
//!
//! # What never moves
//!
//! - A **pinned** file, one a Library track points at.
//! - A file between two tracks that are linked versions: versions never
//!   merge. It's left where it is and counted.
//!
//! # A track from before this was recorded
//!
//! A track none of whose files has a record of the audio it was placed at
//! was made by provisional grouping (1aC-2), where a track's files all held
//! one `audio_hash`. It keeps the audio of its pinned file, else the audio
//! most of its files share (ties to the lowest file id); the rest leave.

use std::collections::{BTreeMap, HashMap, HashSet};

#[cfg(test)]
thread_local! {
    /// How many times [`plan`] looked at a file, for the test that checks
    /// it stays linear.
    pub(crate) static VISITS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// Counts one look at a file (tests only).
#[inline]
fn visit() {
    #[cfg(test)]
    VISITS.with(|v| v.set(v.get() + 1));
}

/// The audio a file holds, as bytes that are equal only for the very same
/// audio: its `audio_hash` (or, where a decision is remembered, its
/// `blake3` when it has none).
pub type Key = Vec<u8>;

/// The audio a file held when it was put in its track.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Placed {
    /// Nothing was recorded: the file was placed before this was kept, or
    /// has no track yet.
    Unknown,
    /// It held this ([`None`]: no audio the app could name).
    At(Option<Key>),
}

/// A file that has, or should get, a track.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Member {
    /// `file.id`.
    pub file: i64,
    /// Its track now, if it has one.
    pub recording: Option<i64>,
    /// The audio it holds, as its `audio_hash`: `None` for a file with
    /// none. What tracks are told apart by.
    pub key: Option<Key>,
    /// What it holds as a remembered decision names it: its `audio_hash`,
    /// or its `blake3` when it has none.
    pub content: Option<Key>,
    /// A Library track points at the file, so it can't move.
    pub pinned: bool,
    pub placed: Placed,
    /// Every one of its pairs is decided under the current rules, so the
    /// pairs it's part of can be relied on.
    pub decided: bool,
}

/// What the rules said about a pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Outcome {
    Duplicate,
    Cut,
    Rework,
    Review,
    Different,
}

impl Outcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Outcome::Duplicate => "duplicate",
            Outcome::Cut => "cut",
            Outcome::Rework => "rework",
            Outcome::Review => "review",
            Outcome::Different => "different",
        }
    }

    pub fn parse(name: &str) -> Option<Outcome> {
        [
            Outcome::Duplicate,
            Outcome::Cut,
            Outcome::Rework,
            Outcome::Review,
            Outcome::Different,
        ]
        .into_iter()
        .find(|outcome| outcome.as_str() == name)
    }
}

/// A decided pair of files, the lower id first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pair {
    pub a: i64,
    pub b: i64,
    pub outcome: Outcome,
}

/// Where a pair stands (`grouping_pair.state`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum State {
    Waiting,
    Merged,
    BothInLibrary,
    SetDisagrees,
    Linked,
    Open,
    InOneTrack,
    Decided,
    Apart,
}

impl State {
    pub const ALL: [State; 9] = [
        State::Waiting,
        State::Merged,
        State::BothInLibrary,
        State::SetDisagrees,
        State::Linked,
        State::Open,
        State::InOneTrack,
        State::Decided,
        State::Apart,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            State::Waiting => "waiting",
            State::Merged => "merged",
            State::BothInLibrary => "both_in_library",
            State::SetDisagrees => "set_disagrees",
            State::Linked => "linked",
            State::Open => "open",
            State::InOneTrack => "in_one_track",
            State::Decided => "decided",
            State::Apart => "apart",
        }
    }

    pub fn parse(name: &str) -> Option<State> {
        State::ALL.into_iter().find(|state| state.as_str() == name)
    }
}

/// What the user decided about two pieces of audio.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Verdict {
    /// Never merge them, and don't ask again.
    Apart,
    /// They are duplicates, whatever the rules say.
    Same,
}

impl Verdict {
    pub fn as_str(self) -> &'static str {
        match self {
            Verdict::Apart => "apart",
            Verdict::Same => "same",
        }
    }

    pub fn parse(name: &str) -> Option<Verdict> {
        [Verdict::Apart, Verdict::Same]
            .into_iter()
            .find(|verdict| verdict.as_str() == name)
    }
}

/// A remembered decision (`grouping_decision`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Remembered {
    pub key_a: Key,
    pub key_b: Key,
    pub verdict: Verdict,
}

/// A version link between two tracks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Link {
    pub a: i64,
    pub b: i64,
    /// The user's: made or confirmed by them. It settles the two tracks'
    /// pairs. Any other link is the job's own, and stands only while a pair
    /// between the two tracks asks for it.
    pub settled: bool,
}

/// Everything [`plan`] reads.
pub struct Input<'a> {
    pub members: &'a [Member],
    pub pairs: &'a [Pair],
    pub remembered: &'a [Remembered],
    pub links: &'a [Link],
    /// The tracks that are in the Library.
    pub library: &'a HashSet<i64>,
    /// The tracks something other than their files points at.
    pub referenced: &'a HashSet<i64>,
    /// A set of several kinds of audio with more files than this is held
    /// as if it disagreed.
    pub max_set_files: usize,
}

/// Where a file goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Dest {
    /// An existing track.
    Recording(i64),
    /// A new track; files with the same number share it.
    New(usize),
}

/// One file's change of track.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Move {
    pub file: i64,
    /// Its track now, `None` if it has none yet.
    pub from: Option<i64>,
    pub to: Dest,
}

/// A version link a pair asks for, between the tracks its two files end
/// up in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Wanted {
    /// The track of the pair's first file, and of its second.
    pub a: Dest,
    pub b: Dest,
    /// The pair, as an index into [`Input::pairs`].
    pub pair: usize,
}

/// What to do. Any leading part of [`Plan::moves`] can be applied alone:
/// planning again from there gives the rest.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Plan {
    pub moves: Vec<Move>,
    /// How many new tracks the moves need.
    pub new_recordings: usize,
    /// Files that should have moved but were left: between linked versions.
    pub left_alone: usize,
    /// Files that stay where they are and whose record of the audio they
    /// were placed at is to be brought up to date.
    pub settle: Vec<i64>,
    /// The job's own links that no pair asks for any more, as the two
    /// tracks: they go before anything moves.
    pub unsupported_links: Vec<(i64, i64)>,
    /// Where each of [`Input::pairs`] stands once the moves are made;
    /// `None` for a pair with a file that isn't a member.
    pub states: Vec<Option<State>>,
    /// The links the pairs ask for, one per pair of tracks.
    pub wanted_links: Vec<Wanted>,
    /// Pairs inside a waiting set that nobody decided yet.
    pub missing: Vec<(i64, i64)>,
    /// Sets held apart because a pair in them isn't a duplicate, the user
    /// decided two of their tracks apart, two are linked versions, or they
    /// are too big to check.
    pub sets_disagreeing: usize,
    /// Sets held apart because two of their tracks are in the Library.
    pub sets_both_in_library: usize,
    /// Sets waiting for a pair to be decided.
    pub sets_waiting: usize,
}

/// What one kind of audio weighs within a track from before placements
/// were recorded.
struct Weight {
    pinned: bool,
    count: usize,
    lowest_file: i64,
}

/// The audio such a track keeps, if it has files with any.
fn winning_key<'a>(members: &[&'a Member]) -> Option<&'a [u8]> {
    let mut weights: BTreeMap<&[u8], Weight> = BTreeMap::new();
    for m in members {
        visit();
        if let Some(key) = m.key.as_deref() {
            let w = weights.entry(key).or_insert(Weight {
                pinned: false,
                count: 0,
                lowest_file: i64::MAX,
            });
            w.pinned |= m.pinned;
            w.count += 1;
            w.lowest_file = w.lowest_file.min(m.file);
        }
    }
    weights
        .into_iter()
        .max_by_key(|(_, w)| (w.pinned, w.count, std::cmp::Reverse(w.lowest_file)))
        .map(|(key, _)| key)
}

/// The file holds other audio than it was placed at.
fn changed(m: &Member) -> bool {
    match &m.placed {
        Placed::Unknown => true,
        Placed::At(key) => *key != m.key,
    }
}

/// Which of a track's files stay in it. At least one always does.
fn stays(held: &[&Member]) -> Vec<bool> {
    if held.len() == 1 {
        return vec![true];
    }
    let lowest = held.iter().map(|m| m.file).min();
    if held.iter().all(|m| m.placed == Placed::Unknown) {
        // A track from before placements were recorded: the old rule.
        let keeps = winning_key(held);
        return held
            .iter()
            .map(|m| {
                m.pinned
                    || match keeps {
                        Some(key) => m.key.as_deref() == Some(key),
                        None => Some(m.file) == lowest,
                    }
            })
            .collect();
    }
    // A file with no record in a track that has some arrived with it (a
    // run cut short): it counts as placed at what it holds.
    let moved_on = |m: &Member| m.placed != Placed::Unknown && changed(m);
    // A pinned file that holds other audio now: the track is that audio's.
    let anchors: Vec<&[u8]> = held
        .iter()
        .filter(|m| m.pinned && moved_on(m))
        .filter_map(|m| m.key.as_deref())
        .collect();
    let mut stay: Vec<bool> = held
        .iter()
        .map(|m| {
            visit();
            let fits = anchors.is_empty() || m.key.as_deref().is_some_and(|k| anchors.contains(&k));
            m.pinned || (!moved_on(m) && fits)
        })
        .collect();
    if !stay.iter().any(|s| *s) {
        // Every file holds other audio now: the lowest is the track.
        for (m, stay) in held.iter().zip(&mut stay) {
            *stay = Some(m.file) == lowest;
        }
    }
    stay
}

/// A track's files that stay in it, or one file that needs a track.
struct Node {
    /// The track, for a track's node.
    track: Option<i64>,
    /// The track a file left, for a file that holds other audio now.
    origin: Option<i64>,
    /// Indexes into the members.
    files: Vec<usize>,
}

/// Disjoint sets of nodes; each set knows whether it's in the Library.
#[derive(Clone)]
struct Sets {
    parent: Vec<usize>,
    library: Vec<bool>,
}

impl Sets {
    fn find(&mut self, mut x: usize) -> usize {
        while self.parent[x] != x {
            self.parent[x] = self.parent[self.parent[x]];
            x = self.parent[x];
        }
        x
    }

    /// Joins two sets under the lower root, so the answer doesn't depend
    /// on the order of the joins.
    fn union(&mut self, a: usize, b: usize) {
        let (a, b) = (self.find(a), self.find(b));
        if a == b {
            return;
        }
        let (low, high) = (a.min(b), a.max(b));
        self.parent[high] = low;
        self.library[low] |= self.library[high];
    }
}

/// What became of a set of candidates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Status {
    Merge,
    Waiting,
    /// Two of its tracks are in the Library.
    Library,
    /// Two of its tracks are linked versions.
    Linked,
    Disagrees,
}

fn ordered(a: usize, b: usize) -> (usize, usize) {
    (a.min(b), a.max(b))
}

/// The two tracks, the lower id first.
fn tracks(a: i64, b: i64) -> (i64, i64) {
    (a.min(b), a.max(b))
}

/// What the planning steps share.
struct Work<'a> {
    input: &'a Input<'a>,
    nodes: Vec<Node>,
    /// Each member's node.
    node_of: Vec<usize>,
    /// The index of each file among the members.
    index: HashMap<i64, usize>,
    /// The pair of each two files, as an index into the pairs.
    pair_at: HashMap<(i64, i64), usize>,
    /// The links that stand, as node pairs, and whether each is settled.
    links: Vec<(usize, usize, bool)>,
    /// Pairs of nodes the user decided apart, or the same.
    apart: Vec<(usize, usize)>,
    same: Vec<(usize, usize)>,
}

impl Work<'_> {
    fn member(&self, at: usize) -> &Member {
        &self.input.members[at]
    }

    /// The pair of files `x` and `y` (member indexes), if it was decided
    /// and both files' pairs can be relied on.
    fn current_pair(&self, x: usize, y: usize) -> Option<Outcome> {
        let (mx, my) = (self.member(x), self.member(y));
        if !(mx.decided && my.decided) {
            return None;
        }
        let key = (mx.file.min(my.file), mx.file.max(my.file));
        self.pair_at
            .get(&key)
            .map(|&at| self.input.pairs[at].outcome)
    }

    /// Decides each set of candidates in `candidates` that spans more than
    /// one of `merged`'s sets, and joins the ones that merge. `by_pairs`
    /// says whether files holding different audio are in play (then their
    /// pairs are checked); without it every candidate set holds one kind of
    /// audio. Returns each such set's status and nodes, by its candidate
    /// root.
    fn settle(
        &self,
        merged: &mut Sets,
        candidates: &mut Sets,
        by_pairs: bool,
        missing: &mut Vec<(i64, i64)>,
    ) -> BTreeMap<usize, (Status, Vec<usize>)> {
        let mut sets: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
        for node in 0..self.nodes.len() {
            sets.entry(candidates.find(node)).or_default().push(node);
        }
        sets.retain(|_, nodes| {
            let first = merged.find(nodes[0]);
            nodes.iter().any(|&n| merged.find(n) != first)
        });
        // The sets something forbids, whatever their pairs say.
        let mut forbidden: HashMap<usize, Status> = HashMap::new();
        let mut across = |merged: &mut Sets, x: usize, y: usize| -> Option<usize> {
            let set = candidates.find(x);
            (set == candidates.find(y) && merged.find(x) != merged.find(y)).then_some(set)
        };
        for &(x, y, _) in &self.links {
            if let Some(set) = across(merged, x, y) {
                forbidden.insert(set, Status::Linked);
            }
        }
        for &(x, y) in &self.apart {
            if let Some(set) = across(merged, x, y) {
                forbidden.entry(set).or_insert(Status::Disagrees);
            }
        }
        // "The same" between two sets, by the user: their pairs are
        // duplicates.
        let mut same: HashSet<(usize, usize)> = HashSet::new();
        for &(x, y) in &self.same {
            same.insert(ordered(merged.find(x), merged.find(y)));
        }

        let mut settled = BTreeMap::new();
        for (set, nodes) in sets {
            let mut units: Vec<usize> = nodes.iter().map(|&n| merged.find(n)).collect();
            units.sort_unstable();
            units.dedup();
            let in_library = units.iter().filter(|&&u| merged.library[u]).count();
            let status = if in_library >= 2 {
                Status::Library
            } else if let Some(&status) = forbidden.get(&set) {
                status
            } else if by_pairs {
                self.by_pairs(merged, &nodes, &same, missing)
            } else {
                Status::Merge
            };
            settled.insert(set, (status, nodes));
        }
        // Joined only now: a set is judged on the units it was given.
        for (status, nodes) in settled.values() {
            if *status == Status::Merge {
                for &node in &nodes[1..] {
                    merged.union(nodes[0], node);
                }
            }
        }
        settled
    }

    /// Whether every pair of files between two of the set's units is a
    /// duplicate: files holding the same audio are, the user's "the same"
    /// makes two units' files so, and every other pair must have been
    /// decided one.
    fn by_pairs(
        &self,
        merged: &mut Sets,
        nodes: &[usize],
        same: &HashSet<(usize, usize)>,
        missing: &mut Vec<(i64, i64)>,
    ) -> Status {
        let mut files: Vec<(usize, usize)> = Vec::new();
        for &node in nodes {
            let unit = merged.find(node);
            files.extend(self.nodes[node].files.iter().map(|&f| (f, unit)));
        }
        let first = self.member(files[0].0).key.as_deref();
        let one_audio = first.is_some()
            && files
                .iter()
                .all(|&(f, _)| self.member(f).key.as_deref() == first);
        if one_audio {
            return Status::Merge;
        }
        if files.len() > self.input.max_set_files {
            return Status::Disagrees;
        }
        let mut status = Status::Merge;
        let mut unknown: Vec<(i64, i64)> = Vec::new();
        for (i, &(x, unit_x)) in files.iter().enumerate() {
            for &(y, unit_y) in &files[i + 1..] {
                visit();
                if unit_x == unit_y || same.contains(&ordered(unit_x, unit_y)) {
                    continue;
                }
                let (mx, my) = (self.member(x), self.member(y));
                if mx.key.is_some() && mx.key == my.key {
                    continue;
                }
                match self.current_pair(x, y) {
                    Some(Outcome::Duplicate) => {}
                    Some(_) => return Status::Disagrees,
                    None => {
                        status = Status::Waiting;
                        // A file that is due has its pairs decided with
                        // it; only a pair between two decided files was
                        // never asked about.
                        if mx.decided && my.decided {
                            unknown.push((mx.file.min(my.file), mx.file.max(my.file)));
                        }
                    }
                }
            }
        }
        missing.extend(unknown);
        status
    }
}

/// Works out where every file should go. Linear in the files and the
/// pairs, plus the square of the files of each set that holds several
/// kinds of audio (at most [`Input::max_set_files`] each): it runs on the
/// database's one writer.
pub fn plan(input: &Input<'_>) -> Plan {
    let members = input.members;
    let index: HashMap<i64, usize> = members
        .iter()
        .enumerate()
        .map(|(at, m)| (m.file, at))
        .collect();
    let mut by_recording: BTreeMap<i64, Vec<usize>> = BTreeMap::new();
    for (at, m) in members.iter().enumerate() {
        visit();
        if let Some(r) = m.recording {
            by_recording.entry(r).or_default().push(at);
        }
    }

    // The nodes: each track's files that stay in it, and each file that
    // needs a track (it has none, or holds other audio than its track).
    let mut nodes: Vec<Node> = Vec::new();
    let mut node_of: Vec<usize> = vec![usize::MAX; members.len()];
    let mut node_of_track: HashMap<i64, usize> = HashMap::new();
    for (&track, held) in &by_recording {
        let refs: Vec<&Member> = held.iter().map(|&at| &members[at]).collect();
        let stays = stays(&refs);
        let node = nodes.len();
        nodes.push(Node {
            track: Some(track),
            origin: None,
            files: Vec::new(),
        });
        node_of_track.insert(track, node);
        for (&at, stay) in held.iter().zip(stays) {
            if stay {
                nodes[node].files.push(at);
                node_of[at] = node;
            } else {
                node_of[at] = nodes.len();
                nodes.push(Node {
                    track: None,
                    origin: Some(track),
                    files: vec![at],
                });
            }
        }
    }
    for (at, m) in members.iter().enumerate() {
        if m.recording.is_none() {
            node_of[at] = nodes.len();
            nodes.push(Node {
                track: None,
                origin: None,
                files: vec![at],
            });
        }
    }

    let library: Vec<bool> = nodes
        .iter()
        .map(|node| {
            node.track.is_some_and(|t| input.library.contains(&t))
                || node.files.iter().any(|&at| members[at].pinned)
        })
        .collect();
    let mut merged = Sets {
        parent: (0..nodes.len()).collect(),
        library: library.clone(),
    };

    // The nodes holding each kind of audio.
    let mut holders: HashMap<&[u8], Vec<usize>> = HashMap::new();
    for (at, node) in nodes.iter().enumerate() {
        let mut keys: Vec<&[u8]> = node
            .files
            .iter()
            .filter_map(|&file| {
                visit();
                members[file].key.as_deref()
            })
            .collect();
        keys.sort_unstable();
        keys.dedup();
        for key in keys {
            holders.entry(key).or_default().push(at);
        }
    }

    // The links that stand: the user's, and the job's own that a pair
    // between the two tracks still asks for.
    let mut asked_for: HashSet<(i64, i64)> = HashSet::new();
    let mut pair_at: HashMap<(i64, i64), usize> = HashMap::with_capacity(input.pairs.len());
    for (at, pair) in input.pairs.iter().enumerate() {
        pair_at.insert((pair.a, pair.b), at);
        if !matches!(pair.outcome, Outcome::Cut | Outcome::Rework) {
            continue;
        }
        let track = |file: i64| index.get(&file).and_then(|&at| members[at].recording);
        if let (Some(a), Some(b)) = (track(pair.a), track(pair.b)) {
            if a != b {
                asked_for.insert(tracks(a, b));
            }
        }
    }
    let mut links: Vec<(usize, usize, bool)> = Vec::new();
    let mut linked: HashSet<(i64, i64)> = HashSet::new();
    let mut unsupported_links = Vec::new();
    for link in input.links {
        let pair = tracks(link.a, link.b);
        if !link.settled && !asked_for.contains(&pair) {
            unsupported_links.push(pair);
            continue;
        }
        linked.insert(pair);
        if let (Some(&x), Some(&y)) = (node_of_track.get(&link.a), node_of_track.get(&link.b)) {
            links.push((x, y, link.settled));
        }
    }

    // What the user decided, as pairs of nodes. Two tracks holding the
    // very same audio can only be told apart by being in the Library, so a
    // decision between a kind of audio and itself is between Library
    // tracks.
    let mut holding: HashMap<&[u8], Vec<usize>> = HashMap::new();
    if !input.remembered.is_empty() {
        for (at, node) in nodes.iter().enumerate() {
            let mut held: Vec<&[u8]> = node
                .files
                .iter()
                .filter_map(|&file| members[file].content.as_deref())
                .collect();
            held.sort_unstable();
            held.dedup();
            for content in held {
                holding.entry(content).or_default().push(at);
            }
        }
    }
    let (mut apart, mut same) = (Vec::new(), Vec::new());
    for remembered in input.remembered {
        let (Some(xs), Some(ys)) = (
            holding.get(remembered.key_a.as_slice()),
            holding.get(remembered.key_b.as_slice()),
        ) else {
            continue;
        };
        let one_audio = remembered.key_a == remembered.key_b;
        for &x in xs {
            for &y in ys {
                if x == y || (one_audio && !(library[x] && library[y])) {
                    continue;
                }
                match remembered.verdict {
                    Verdict::Apart => apart.push((x, y)),
                    Verdict::Same => same.push((x, y)),
                }
            }
        }
    }

    let work = Work {
        input,
        nodes,
        node_of,
        index,
        pair_at,
        links,
        apart,
        same,
    };
    let mut missing = Vec::new();

    // 1. The very same audio: nodes that hold one kind of audio each, the
    // same one.
    let one_key = |node: &Node| -> Option<&[u8]> {
        let first = members[node.files[0]].key.as_deref()?;
        node.files
            .iter()
            .all(|&at| members[at].key.as_deref() == Some(first))
            .then_some(first)
    };
    let mut candidates = merged.clone();
    let mut first_with: HashMap<&[u8], usize> = HashMap::new();
    for (at, node) in work.nodes.iter().enumerate() {
        if let Some(key) = one_key(node) {
            match first_with.get(key) {
                Some(&first) => candidates.union(first, at),
                None => {
                    first_with.insert(key, at);
                }
            }
        }
    }
    // A set held here is in a set of step 2 as well, and counted there.
    work.settle(&mut merged, &mut candidates, false, &mut missing);

    // 2. Sets joined by duplicate pairs, by the same audio and by the
    // user's "the same".
    let mut candidates = merged.clone();
    for nodes in holders.values() {
        for &node in &nodes[1..] {
            candidates.union(nodes[0], node);
        }
    }
    for pair in input.pairs {
        if pair.outcome != Outcome::Duplicate {
            continue;
        }
        let (Some(&x), Some(&y)) = (work.index.get(&pair.a), work.index.get(&pair.b)) else {
            continue;
        };
        if members[x].decided && members[y].decided {
            candidates.union(work.node_of[x], work.node_of[y]);
        }
    }
    for &(x, y) in &work.same {
        candidates.union(x, y);
    }
    let by_pairs = work.settle(&mut merged, &mut candidates, true, &mut missing);
    missing.sort_unstable();
    missing.dedup();

    let mut plan = Plan {
        unsupported_links,
        missing,
        ..Plan::default()
    };
    for (status, _) in by_pairs.values() {
        match status {
            Status::Merge => {}
            Status::Waiting => plan.sets_waiting += 1,
            Status::Library => plan.sets_both_in_library += 1,
            Status::Linked | Status::Disagrees => plan.sets_disagreeing += 1,
        }
    }

    // Where each node's files go: the track its set ends up in.
    let mut groups: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for node in 0..work.nodes.len() {
        groups.entry(merged.find(node)).or_default().push(node);
    }
    let mut dest: Vec<Dest> = vec![Dest::New(usize::MAX); members.len()];
    let mut leaving: HashSet<i64> = HashSet::new();
    let mut unsettled: HashSet<i64> = HashSet::new();
    for group in groups.values() {
        // An existing track: one in the Library first, then one something
        // refers to, then the lowest id.
        let target = group
            .iter()
            .filter_map(|&node| {
                let track = work.nodes[node].track?;
                Some((!library[node], !input.referenced.contains(&track), track))
            })
            .min()
            .map(|(_, _, track)| track);
        let to = match target {
            Some(track) => Dest::Recording(track),
            None => {
                plan.new_recordings += 1;
                Dest::New(plan.new_recordings - 1)
            }
        };
        for &node in group {
            let came_from = work.nodes[node].track.or(work.nodes[node].origin);
            for &at in &work.nodes[node].files {
                visit();
                let m = &members[at];
                dest[at] = to;
                if target.is_some() && came_from == target {
                    continue;
                }
                if let (Some(from), Some(target)) = (came_from, target) {
                    if linked.contains(&tracks(from, target)) {
                        // Between linked versions: it stays where it is.
                        plan.left_alone += 1;
                        dest[at] = Dest::Recording(from);
                        unsettled.insert(m.file);
                        continue;
                    }
                }
                if let Some(from) = came_from {
                    leaving.insert(from);
                }
                plan.moves.push(Move {
                    file: m.file,
                    from: m.recording,
                    to,
                });
            }
        }
    }
    // Files a link holds back, counted as they always were: the files of
    // the tracks that would have gone into another.
    for (status, set_nodes) in by_pairs.values() {
        if *status != Status::Linked {
            continue;
        }
        let mut set_nodes = set_nodes.clone();
        set_nodes.sort_by_key(|&n| {
            let track = work.nodes[n].track;
            (
                track.is_none(),
                !library[n],
                !track.is_some_and(|t| input.referenced.contains(&t)),
                track,
            )
        });
        for &node in set_nodes.iter().skip(1) {
            plan.left_alone += work.nodes[node].files.len();
        }
    }

    // The files that stay put and whose record is out of date, in tracks
    // nothing is leaving.
    for (at, m) in members.iter().enumerate() {
        let Some(track) = m.recording else { continue };
        let stays = dest[at] == Dest::Recording(track);
        if stays && changed(m) && !leaving.contains(&track) && !unsettled.contains(&m.file) {
            plan.settle.push(m.file);
        }
    }

    // Where each pair stands, and the links the pairs ask for.
    let mut settled: HashSet<(usize, usize)> = HashSet::new();
    for &(x, y) in work.apart.iter().chain(&work.same) {
        settled.insert(ordered(merged.find(x), merged.find(y)));
    }
    for &(x, y, by_user) in &work.links {
        if by_user {
            settled.insert(ordered(merged.find(x), merged.find(y)));
        }
    }
    let mut wanted: BTreeMap<(Dest, Dest), Wanted> = BTreeMap::new();
    for (at, pair) in input.pairs.iter().enumerate() {
        let (Some(&x), Some(&y)) = (work.index.get(&pair.a), work.index.get(&pair.b)) else {
            plan.states.push(None);
            continue;
        };
        let (node_x, node_y) = (work.node_of[x], work.node_of[y]);
        let current = members[x].decided && members[y].decided;
        let state = if dest[x] == dest[y] {
            match pair.outcome {
                Outcome::Duplicate => State::Merged,
                _ => State::InOneTrack,
            }
        } else if pair.outcome == Outcome::Different {
            State::Apart
        } else if settled.contains(&ordered(merged.find(node_x), merged.find(node_y))) {
            State::Decided
        } else {
            match pair.outcome {
                Outcome::Cut | Outcome::Rework => {
                    let key = (dest[x].min(dest[y]), dest[x].max(dest[y]));
                    wanted.entry(key).or_insert(Wanted {
                        a: dest[x],
                        b: dest[y],
                        pair: at,
                    });
                    State::Linked
                }
                Outcome::Review => State::Open,
                _ => {
                    let set = candidates.find(node_x);
                    let together = current && set == candidates.find(node_y);
                    let (unit_x, unit_y) = (merged.find(node_x), merged.find(node_y));
                    let both_in_library = merged.library[unit_x] && merged.library[unit_y];
                    match by_pairs.get(&set).filter(|_| together).map(|s| s.0) {
                        None | Some(Status::Waiting) => State::Waiting,
                        Some(Status::Library) if both_in_library => State::BothInLibrary,
                        Some(_) => State::SetDisagrees,
                    }
                }
            }
        };
        plan.states.push(Some(state));
    }
    plan.wanted_links = wanted.into_values().collect();
    plan
}
