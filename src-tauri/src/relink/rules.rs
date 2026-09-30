//! The relink rules, on data already loaded: no database, no disk.
//!
//! [`plan`] takes every present file and every `rekordbox_track` row and
//! decides each row's match from scratch. Only a confirmed relink, and a
//! match from a step this run doesn't make (fingerprint, filename only, gig
//! stick; 1aD, 1bE), carry over, and only while their file is present. A
//! `user` match carries over only as a confirmed relink.
//! The steps run in order over all rows (every row's step 1 before any
//! row's step 2), so a later step can never take a file an earlier,
//! stronger step gives to another row. See the module docs in [`super`]
//! for each rule and its false-match risk.

use std::collections::{HashMap, HashSet};

use unicode_normalization::UnicodeNormalization;

use crate::rekordbox::location::{FilePath, Location, PathStyle};

/// How a rekordbox track was matched to its file: `relink_method` in
/// `rekordbox_track` and `method` in `relink`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Method {
    /// Step 1: the Location still names the file.
    Path,
    /// Step 2: same file name, and the durations agree.
    FilenameDuration,
    /// Step 3: the only file of that duration in the candidate set.
    UniqueDuration,
    /// Step 4 (1aD): the acoustic fingerprint.
    Fingerprint,
    /// Step 5 (1aD): the file name alone; probable until confirmed.
    FilenameOnly,
    /// Step 6 (1aD): recovered from a gig stick.
    GigStick,
    /// Picked by the user.
    User,
}

impl Method {
    pub const ALL: [Method; 7] = [
        Method::Path,
        Method::FilenameDuration,
        Method::UniqueDuration,
        Method::Fingerprint,
        Method::FilenameOnly,
        Method::GigStick,
        Method::User,
    ];

    /// The name stored in the database.
    pub fn as_str(self) -> &'static str {
        match self {
            Method::Path => "path",
            Method::FilenameDuration => "filename_duration",
            Method::UniqueDuration => "unique_duration",
            Method::Fingerprint => "fingerprint",
            Method::FilenameOnly => "filename_only",
            Method::GigStick => "gig_stick",
            Method::User => "user",
        }
    }

    /// The method stored as `name`, if there is one.
    pub fn parse(name: &str) -> Option<Method> {
        Method::ALL.into_iter().find(|m| m.as_str() == name)
    }

    /// Whether this run's steps make matches this way, and so remake them
    /// from scratch. Matches made by 1aD's and 1bE's steps carry over
    /// while their file is present; a `user` match only with its `relink`
    /// row.
    fn is_recomputed(self) -> bool {
        matches!(
            self,
            Method::Path | Method::FilenameDuration | Method::UniqueDuration
        )
    }

    /// Whether a match made this way says where a track's neighbours went
    /// (step 3's candidate set). A duration-only or name-only guess
    /// doesn't, unless the user confirmed it.
    fn is_evidence(self) -> bool {
        !matches!(self, Method::UniqueDuration | Method::FilenameOnly)
    }
}

/// How sure each step is, stored as `relink_confidence`. Nothing reads
/// these as thresholds; `relink_probable` says whether a match is trusted.
pub mod confidence {
    /// Confirmed by the user. Reserved for confirmations among the methods
    /// that carry over (fingerprint, filename only, gig stick): a stored
    /// 1.0 without its `relink` row is a withdrawn confirmation, decided
    /// again. The steps that make those matches store less than 1.0.
    pub const CONFIRMED: f64 = 1.0;
    /// The Location names the file on a drive that's plugged in.
    pub const PATH: f64 = 1.0;
    /// The Location names where the file was when its drive was last seen.
    pub const PATH_OFFLINE: f64 = 0.9;
    pub const FILENAME_DURATION: f64 = 0.9;
    /// A unique duration a title tag agrees with.
    pub const UNIQUE_DURATION: f64 = 0.8;
    /// A unique duration no title tag agrees with: probable only.
    pub const UNIQUE_DURATION_PROBABLE: f64 = 0.4;
}

/// Step 3 looks at no more files than this. A bigger candidate set skips
/// step 3 for its tracks (it isn't cut down): in a big set, a single file
/// within the duration window is more likely chance than evidence, and
/// step 4 (the fingerprint, 1aD) is the tool for big folders.
pub const MAX_CANDIDATES: usize = 50;

/// A file that is present (`file.present = 1`), whether or not its drive
/// is plugged in.
#[derive(Debug, Clone, Default)]
pub struct File {
    pub id: i64,
    /// Its music folder's row id.
    pub folder: i64,
    /// The folder it's in, from its music folder, in the on-disk spelling;
    /// `""` at the top of the music folder.
    pub parent: String,
    /// Its name, in the on-disk spelling.
    pub name: String,
    /// Its full path, `/`-separated (`E:/Music/a.mp3`,
    /// `//server/share/a.mp3`): under its volume's mount point now when
    /// `online`, or where the volume was last mounted when not. `None`
    /// when neither is known, or when another known volume was last
    /// mounted there too.
    pub path: Option<String>,
    pub online: bool,
    /// `None` until stage 2 has read it (or when it couldn't, or when the
    /// file is online-only and wasn't read).
    pub duration_ms: Option<i64>,
    /// Its `audio_hash`, once stage 3 has one.
    pub audio_hash: Option<Vec<u8>>,
    /// Its track (`recording_file`), once grouped.
    pub recording: Option<i64>,
    /// Whether it's its track's best file.
    pub best: bool,
}

/// One `rekordbox_track` row.
#[derive(Debug, Clone, Default)]
pub struct Track {
    pub id: i64,
    /// `location_key` as stored: the Location decoded by hand, NFC, and for
    /// Windows paths, letter case folded.
    pub key: String,
    /// The decoded Location; `None` if it doesn't decode.
    pub location: Option<Location>,
    /// `TotalTime`, whole seconds (truncated, §5.3); `None` when missing,
    /// not a number, or 0.
    pub total_s: Option<u32>,
    /// rekordbox's `Name` as a [`title_key`]; empty when it has none.
    pub name: String,
    /// Its match as stored before this run.
    pub current: Option<Target>,
    /// Its `recording_id` as stored before this run.
    pub recording: Option<i64>,
}

/// A row's match: what `file_id`, `relink_method`, `relink_confidence` and
/// `relink_probable` hold.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Target {
    pub file: i64,
    pub method: Method,
    pub confidence: Option<f64>,
    /// Matched, but not trusted until the user confirms it.
    pub probable: bool,
}

/// A confirmed relink (the `relink` table).
#[derive(Debug, Clone, Copy)]
pub struct Confirmed {
    pub file: i64,
    pub method: Method,
}

/// Everything [`plan`] looks at.
#[derive(Debug, Clone, Default)]
pub struct Input {
    pub files: Vec<File>,
    pub tracks: Vec<Track>,
    /// By Location key.
    pub confirmed: HashMap<String, Confirmed>,
}

/// A row to write: its new match (`None` clears it) and the track its file
/// belongs to.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Decision {
    /// The `rekordbox_track` row.
    pub track: i64,
    pub target: Option<Target>,
    pub recording: Option<i64>,
}

/// What [`plan`] decided.
#[derive(Debug, Clone, Default)]
pub struct Plan {
    /// Rows whose stored match or track changes, in row order.
    pub decisions: Vec<Decision>,
    /// Every row after this run, by how it's matched.
    pub confirmed: usize,
    pub path: usize,
    pub filename_duration: usize,
    /// Unique duration, accepted.
    pub unique_duration: usize,
    /// Probable, waiting for the user.
    pub probable: usize,
    /// Carried over from a step this run doesn't make.
    pub other: usize,
    pub streaming: usize,
    pub missing: usize,
    /// Files step 3 looked at, for tests of how much work it does.
    pub examined: usize,
}

/// Whether a file lasting `duration_ms` can be the track rekordbox says
/// lasts `total_s`. rekordbox truncates to whole seconds (§5.3), so its
/// own duration is somewhere in `[T, T+1)` s; allowing ±0.5 s for decoders
/// that disagree, the file must last from `T·1000 − 500` ms up to, not
/// including, `T·1000 + 1500` ms.
pub fn duration_fits(total_s: u32, duration_ms: i64) -> bool {
    let t = i64::from(total_s) * 1000;
    duration_ms >= t - 500 && duration_ms < t + 1500
}

/// The (at most two) `TotalTime`s a file lasting `duration_ms` fits.
fn totals_fitting(duration_ms: i64) -> impl Iterator<Item = u32> {
    let high = (duration_ms + 500).div_euclid(1000);
    [high - 1, high]
        .into_iter()
        .filter_map(|t| u32::try_from(t).ok())
        .filter(move |&t| t > 0 && duration_fits(t, duration_ms))
}

/// A path's key for matching: NFC, and letter case folded the way NTFS
/// folds it. The same key [`FilePath::match_key`] gives a Windows path.
pub fn path_key(path: &str) -> String {
    path.nfc().map(upcase).collect()
}

/// A title for comparing a file's title tag with rekordbox's `Name`: NFKC
/// and lowercase; featuring credits dropped (`(feat. X)`, `[ft X]`, or a
/// trailing ` feat. X`); every run of characters that aren't letters or
/// digits made one space. Bracket contents stay, so `Title (Clean)` and
/// `Title (Dirty)`, or an original and its remix, never agree.
pub fn title_key(title: &str) -> String {
    let lower: String = title.nfkc().flat_map(char::to_lowercase).collect();
    let credited = drop_featuring(&lower);
    let words: Vec<&str> = credited
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();
    words.join(" ")
}

/// `s` (lowercase) without its featuring credits.
fn drop_featuring(s: &str) -> String {
    const OPEN: &str = "([{";
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(c) = rest.chars().next() {
        if OPEN.contains(c) {
            let inner = &rest[c.len_utf8()..];
            if starts_with_featuring(inner.trim_start()) {
                let close = match c {
                    '(' => ')',
                    '[' => ']',
                    _ => '}',
                };
                rest = inner.find(close).map_or("", |at| &inner[at + 1..]);
                out.push(' ');
                continue;
            }
        }
        let word_start = out.chars().last().is_none_or(|p| !p.is_alphanumeric());
        if word_start && starts_with_featuring(rest) {
            // To the next bracket, or the end.
            let end = rest.find(|b| OPEN.contains(b)).unwrap_or(rest.len());
            rest = &rest[end..];
            out.push(' ');
            continue;
        }
        out.push(c);
        rest = &rest[c.len_utf8()..];
    }
    out
}

/// Whether `s` starts with the word `feat`, `ft` or `featuring`.
fn starts_with_featuring(s: &str) -> bool {
    ["featuring", "feat", "ft"].iter().any(|word| {
        s.strip_prefix(word)
            .is_some_and(|after| after.chars().next().is_none_or(|c| !c.is_alphanumeric()))
    })
}

/// A path as Windows compares it: letter case folded, spelling otherwise
/// exact (an NFC name and its NFD twin are two files).
fn windows_spelling(path: &str) -> String {
    path.chars().map(upcase).collect()
}

/// A letter's simple uppercase form when the letter and the result are each
/// one UTF-16 unit, as in an NTFS upcase table; otherwise the letter itself.
/// (The rule of `rekordbox::location` and `paths`; a test checks this copy
/// keys paths the way Locations are keyed.)
fn upcase(c: char) -> char {
    if u32::from(c) > 0xFFFF {
        return c;
    }
    let mut up = c.to_uppercase();
    match (up.next(), up.next()) {
        (Some(u), None) if u32::from(u) <= 0xFFFF => u,
        _ => c,
    }
}

/// Everything before the last `/`: a path's folder.
fn folder_of(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(dir, _)| dir)
}

/// A folder files are in: a music folder's row id and the path within it.
type Place = (i64, String);

/// The Windows path a Location names, or `None` for a macOS path, a
/// streaming entry or no Location.
fn windows_path(location: Option<&Location>) -> Option<&FilePath> {
    let path = location?.as_file()?;
    matches!(
        path.style(),
        PathStyle::WindowsDrive | PathStyle::WindowsUnc
    )
    .then_some(path)
}

/// Lookups over the present files.
struct Index<'a> {
    files: &'a [File],
    by_id: HashMap<i64, usize>,
    /// Path key → files, for drives plugged in and for drives that aren't.
    online_paths: HashMap<String, Vec<usize>>,
    offline_paths: HashMap<String, Vec<usize>>,
    /// Folder key → the places with that path.
    online_folders: HashMap<String, HashSet<Place>>,
    offline_folders: HashMap<String, HashSet<Place>>,
    /// Name key → files.
    names: HashMap<String, Vec<usize>>,
    /// Place → files.
    places: HashMap<Place, Vec<usize>>,
}

impl<'a> Index<'a> {
    fn new(files: &'a [File]) -> Index<'a> {
        let mut index = Index {
            files,
            by_id: HashMap::with_capacity(files.len()),
            online_paths: HashMap::new(),
            offline_paths: HashMap::new(),
            online_folders: HashMap::new(),
            offline_folders: HashMap::new(),
            names: HashMap::new(),
            places: HashMap::new(),
        };
        for (i, file) in files.iter().enumerate() {
            index.by_id.insert(file.id, i);
            let place = (file.folder, file.parent.clone());
            if let Some(path) = &file.path {
                let key = path_key(path);
                let folder = folder_of(&key).to_owned();
                let (paths, folders) = if file.online {
                    (&mut index.online_paths, &mut index.online_folders)
                } else {
                    (&mut index.offline_paths, &mut index.offline_folders)
                };
                paths.entry(key).or_default().push(i);
                folders.entry(folder).or_default().insert(place.clone());
            }
            index.names.entry(path_key(&file.name)).or_default().push(i);
            index.places.entry(place).or_default().push(i);
        }
        index
    }

    fn file(&self, id: i64) -> Option<&File> {
        self.by_id.get(&id).map(|&i| &self.files[i])
    }

    fn place_of(&self, file: i64) -> Option<Place> {
        let f = self.file(file)?;
        Some((f.folder, f.parent.clone()))
    }

    /// Step 1: the file the Windows path names. A drive plugged in is asked
    /// first; only if none of them holds the path are drives that aren't
    /// asked, by where they were last mounted. Two files with the same key
    /// (an NFC name and its NFD twin) are told apart by the exact spelling,
    /// as Windows would; if that doesn't settle it, there's no match.
    fn at_path(&self, path: &FilePath) -> Option<(i64, bool)> {
        let key = path_key(path.as_str());
        for (paths, online) in [(&self.online_paths, true), (&self.offline_paths, false)] {
            let Some(found) = paths.get(&key) else {
                continue;
            };
            let pick = match found.as_slice() {
                [only] => Some(*only),
                _ => {
                    let want = windows_spelling(path.as_str());
                    let exact: Vec<usize> = found
                        .iter()
                        .copied()
                        .filter(|&i| {
                            self.files[i].path.as_deref().map(windows_spelling).as_ref()
                                == Some(&want)
                        })
                        .collect();
                    match exact.as_slice() {
                        [only] => Some(*only),
                        _ => None,
                    }
                }
            };
            return pick.map(|i| (self.files[i].id, online));
        }
        None
    }

    /// The places a Windows folder path names: on drives plugged in if any,
    /// otherwise where drives were last mounted.
    fn places_at(&self, folder: &str) -> Option<&HashSet<Place>> {
        let key = path_key(folder);
        self.online_folders
            .get(&key)
            .or_else(|| self.offline_folders.get(&key))
    }

    /// Among `candidates`, the files whose duration fits `total_s`, and
    /// whether a candidate of unknown duration could have fit too.
    fn fits(&self, total_s: u32, candidates: &[usize]) -> (Vec<i64>, bool) {
        let mut files = Vec::new();
        let mut blocked = false;
        for &i in candidates {
            let file = &self.files[i];
            match file.duration_ms {
                Some(d) if duration_fits(total_s, d) => files.push(file.id),
                Some(_) => {}
                None => blocked = true,
            }
        }
        (files, blocked)
    }

    /// The one file to match among `fits`: the only one, or, when every
    /// one of them holds the same audio (one `audio_hash`: exact copies),
    /// their track's best file, else the lowest id. `None` if they differ.
    fn pick(&self, fits: &[i64]) -> Option<i64> {
        match fits {
            [] => None,
            [only] => Some(*only),
            _ => {
                let hash = self.file(fits[0])?.audio_hash.as_ref()?;
                let copies: Vec<&File> = fits.iter().filter_map(|&f| self.file(f)).collect();
                if copies.len() != fits.len()
                    || copies.iter().any(|f| f.audio_hash.as_ref() != Some(hash))
                {
                    return None;
                }
                copies
                    .iter()
                    .find(|f| f.best)
                    .or_else(|| copies.iter().min_by_key(|f| f.id))
                    .map(|f| f.id)
            }
        }
    }
}

/// A row's match as this run decides it.
#[derive(Debug, Clone, Copy)]
struct Now {
    target: Target,
    confirmed: bool,
}

/// Decides every row's match from scratch; see the module docs. `titles`
/// gives a file's title tags, each as a [`title_key`]; it's asked only for
/// the files step 3 matches, so tags are read for a handful of files, not
/// every one.
pub fn plan<E>(
    input: &Input,
    mut titles: impl FnMut(i64) -> Result<Vec<String>, E>,
) -> Result<Plan, E> {
    let index = Index::new(&input.files);
    let tracks = &input.tracks;
    let mut out = Plan::default();
    let is_file = |t: &Track| matches!(t.location, Some(Location::File(_)));
    let mut now: Vec<Option<Now>> = vec![None; tracks.len()];
    let set = |now: &mut Vec<Option<Now>>, i: usize, file, method, confidence, probable| {
        now[i] = Some(Now {
            target: Target {
                file,
                method,
                confidence: Some(confidence),
                probable,
            },
            confirmed: false,
        });
    };

    // Step 0: every confirmed relink is re-applied first, while its file is
    // present.
    for (i, track) in tracks.iter().enumerate() {
        if !is_file(track) {
            continue;
        }
        if let Some(c) = input.confirmed.get(&track.key) {
            if index.by_id.contains_key(&c.file) {
                now[i] = Some(Now {
                    target: Target {
                        file: c.file,
                        method: c.method,
                        confidence: Some(confidence::CONFIRMED),
                        probable: false,
                    },
                    confirmed: true,
                });
            }
        }
    }
    // Then a match from a step this run doesn't make (1aD, 1bE) carries
    // over, while its file is present and no confirmation gives that file
    // to another row. A match the user confirmed (`user`, or confidence
    // 1.0) whose `relink` row is gone was withdrawn: it's decided again. A
    // carried *probable* match is only a fallback: the row still goes
    // through steps 1–3, and an accepted match there replaces it.
    //
    // Not checked here: whether the file's audio changed since the match
    // was made. That needs evidence recorded when the match is made, which
    // belongs to the steps that make carried matches (1aD-1, 1aD-2, 1bE-6).
    let confirmed_files: HashSet<i64> = now.iter().flatten().map(|n| n.target.file).collect();
    let mut fallback: Vec<Option<Target>> = vec![None; tracks.len()];
    for (i, track) in tracks.iter().enumerate() {
        if now[i].is_some() || !is_file(track) {
            continue;
        }
        let Some(current) = track.current else {
            continue;
        };
        let carried = !current.method.is_recomputed()
            && current.method != Method::User
            && current.confidence != Some(confidence::CONFIRMED)
            && index.by_id.contains_key(&current.file)
            && !confirmed_files.contains(&current.file);
        if !carried {
            continue;
        }
        if current.probable {
            fallback[i] = Some(current);
        } else {
            now[i] = Some(Now {
                target: current,
                confirmed: false,
            });
        }
    }

    // Step 1: the Location still names a present file.
    for (i, track) in tracks.iter().enumerate() {
        if now[i].is_some() {
            continue;
        }
        let Some(path) = windows_path(track.location.as_ref()) else {
            continue;
        };
        if let Some((file, online)) = index.at_path(path) {
            let confidence = if online {
                confidence::PATH
            } else {
                confidence::PATH_OFFLINE
            };
            set(&mut now, i, file, Method::Path, confidence, false);
        }
    }

    // Which rows hold each file. A carried probable match's file stays
    // taken for the other rows, but not for its own row, which may still
    // upgrade to an accepted match on that same file.
    let taken = |now: &[Option<Now>]| -> HashMap<i64, HashSet<usize>> {
        let mut holders: HashMap<i64, HashSet<usize>> = HashMap::new();
        for (i, n) in now.iter().enumerate() {
            if let Some(n) = n {
                holders.entry(n.target.file).or_default().insert(i);
            }
        }
        for (i, t) in fallback.iter().enumerate() {
            if let Some(t) = t {
                holders.entry(t.file).or_default().insert(i);
            }
        }
        holders
    };
    let held_by_other = |holders: &HashMap<i64, HashSet<usize>>, file: i64, i: usize| {
        holders
            .get(&file)
            .is_some_and(|h| h.iter().any(|&o| o != i))
    };
    let open = |now: &[Option<Now>]| -> Vec<(usize, u32)> {
        tracks
            .iter()
            .enumerate()
            .filter(|(i, t)| now[*i].is_none() && is_file(t))
            .filter_map(|(i, t)| Some((i, t.total_s?)))
            .collect()
    };

    // Step 2: the same file name anywhere, and the duration fits. Every
    // track that fits a file counts against giving it to another.
    let claimed = taken(&now);
    let mut by_name: Vec<(usize, Vec<i64>, bool)> = Vec::new();
    for (i, total_s) in open(&now) {
        let Some(path) = tracks[i].location.as_ref().and_then(Location::as_file) else {
            continue;
        };
        if let Some(same_name) = index.names.get(&path_key(path.file_name())) {
            let (fits, blocked) = index.fits(total_s, same_name);
            if !fits.is_empty() || blocked {
                by_name.push((i, fits, blocked));
            }
        }
    }
    let mut wanted: HashMap<i64, HashSet<usize>> = HashMap::new();
    for (i, fits, _) in &by_name {
        for &file in fits {
            wanted.entry(file).or_default().insert(*i);
        }
    }
    let alone = |wanted: &HashMap<i64, HashSet<usize>>, file: i64, i: usize| {
        wanted.get(&file).is_none_or(|w| w.iter().all(|&o| o == i))
    };
    for (i, fits, blocked) in &by_name {
        if *blocked {
            continue;
        }
        let Some(file) = index.pick(fits) else {
            continue;
        };
        let copy_taken = fits.iter().any(|&f| held_by_other(&claimed, f, *i));
        if fits.iter().all(|&f| alone(&wanted, f, *i)) && !copy_taken {
            set(
                &mut now,
                *i,
                file,
                Method::FilenameDuration,
                confidence::FILENAME_DURATION,
                false,
            );
        }
    }

    // Step 3: the only file of that duration in the candidate set. Tracks
    // from one rekordbox folder share their set: the files directly in the
    // folder that folder still names, plus the folders its neighbours were
    // matched into by a step that says where files went.
    let claimed = taken(&now);
    let mut went: HashMap<&str, HashSet<Place>> = HashMap::new();
    for (i, track) in tracks.iter().enumerate() {
        let Some(n) = now[i] else { continue };
        if !(n.confirmed || (n.target.method.is_evidence() && !n.target.probable))
            || !is_file(track)
        {
            continue;
        }
        if let Some(place) = index.place_of(n.target.file) {
            went.entry(folder_of(&track.key)).or_default().insert(place);
        }
    }
    // The open tracks, by rekordbox folder and by TotalTime.
    let mut folders: HashMap<&str, HashMap<u32, Vec<usize>>> = HashMap::new();
    for (i, total_s) in open(&now) {
        folders
            .entry(folder_of(&tracks[i].key))
            .or_default()
            .entry(total_s)
            .or_default()
            .push(i);
    }
    // Which rekordbox folders each place is a candidate for, and what each
    // track found in its set when the set is small enough to look at.
    let mut covers: HashMap<&Place, Vec<&str>> = HashMap::new();
    let mut found: Vec<(usize, Vec<i64>, bool)> = Vec::new();
    let mut folder_names: Vec<&&str> = folders.keys().collect();
    folder_names.sort();
    for &folder in folder_names {
        let by_total = &folders[folder];
        let mut places: HashSet<&Place> = HashSet::new();
        let own = by_total
            .values()
            .flatten()
            .find_map(|&i| windows_path(tracks[i].location.as_ref()));
        if let Some(path) = own {
            places.extend(
                index
                    .places_at(folder_of(path.as_str()))
                    .into_iter()
                    .flatten(),
            );
        }
        places.extend(went.get(folder).into_iter().flatten());
        for &place in &places {
            covers.entry(place).or_default().push(folder);
        }
        let size: usize = places
            .iter()
            .map(|p| index.places.get(*p).map_or(0, Vec::len))
            .sum();
        if size == 0 || size > MAX_CANDIDATES {
            continue;
        }
        let candidates: Vec<usize> = places
            .iter()
            .filter_map(|p| index.places.get(*p))
            .flatten()
            .copied()
            .collect();
        out.examined += candidates.len();
        for (&total_s, members) in by_total {
            let (fits, blocked) = index.fits(total_s, &candidates);
            for &i in members {
                found.push((i, fits.clone(), blocked));
            }
        }
    }
    // Every open track that could be a file's: by duration within a set
    // holding it (however big the set), or by name.
    let wants = |file: i64| -> HashSet<usize> {
        let mut who = HashSet::new();
        let Some(f) = index.file(file) else {
            return who;
        };
        if let (Some(d), Some(place)) = (f.duration_ms, index.place_of(file)) {
            for folder in covers.get(&place).into_iter().flatten() {
                for total in totals_fitting(d) {
                    who.extend(folders[folder].get(&total).into_iter().flatten());
                }
            }
        }
        for (i, fits, _) in &by_name {
            if now[*i].is_none() && fits.contains(&file) {
                who.insert(*i);
            }
        }
        who
    };
    let mut decided = Vec::new();
    for (i, fits, blocked) in &found {
        if *blocked {
            continue;
        }
        let Some(file) = index.pick(fits) else {
            continue;
        };
        let alone = fits.iter().all(|&f| wants(f).iter().all(|&o| o == *i));
        let copy_taken = fits.iter().any(|&f| held_by_other(&claimed, f, *i));
        if alone && !copy_taken {
            decided.push((*i, file));
        }
    }
    // Accepted only when one of the file's title tags agrees with
    // rekordbox's Name; otherwise probable, like a filename-only match.
    for (i, file) in decided {
        let name = &tracks[i].name;
        let agrees = !name.is_empty() && titles(file)?.contains(name);
        let confidence = if agrees {
            confidence::UNIQUE_DURATION
        } else {
            confidence::UNIQUE_DURATION_PROBABLE
        };
        set(
            &mut now,
            i,
            file,
            Method::UniqueDuration,
            confidence,
            !agrees,
        );
    }

    // A carried probable match stands unless steps 1–3 found an accepted
    // one.
    for (i, carried) in fallback.iter().enumerate() {
        if let Some(carried) = carried {
            if now[i].is_none_or(|n| n.target.probable) {
                now[i] = Some(Now {
                    target: *carried,
                    confirmed: false,
                });
            }
        }
    }

    for (i, track) in tracks.iter().enumerate() {
        let target = now[i].map(|n| n.target);
        let recording = target.and_then(|t| index.file(t.file)?.recording);
        if target != track.current || recording != track.recording {
            out.decisions.push(Decision {
                track: track.id,
                target,
                recording,
            });
        }
        match now[i] {
            _ if matches!(track.location, Some(Location::Streaming(_))) => out.streaming += 1,
            None => out.missing += 1,
            Some(n) if n.confirmed => out.confirmed += 1,
            Some(n) if n.target.probable => out.probable += 1,
            Some(n) => match n.target.method {
                Method::Path => out.path += 1,
                Method::FilenameDuration => out.filename_duration += 1,
                Method::UniqueDuration => out.unique_duration += 1,
                _ => out.other += 1,
            },
        }
    }
    Ok(out)
}
