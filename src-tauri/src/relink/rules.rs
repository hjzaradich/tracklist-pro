//! The relink rules, on data already loaded: no database, no disk.
//!
//! [`plan`] takes every present file and every `rekordbox_track` row and
//! decides which rows get a file. The steps run in order over all rows
//! (every row's step 1 before any row's step 2), so a later step can never
//! take a file an earlier, stronger step gives to another row. See the
//! module docs in [`super`] for each rule and its false-match risk.

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

    /// Whether a match made this way says where a track's neighbours went
    /// (step 3's candidate set). A duration-only or name-only guess
    /// doesn't, unless the user confirmed it.
    fn is_evidence(self) -> bool {
        !matches!(self, Method::UniqueDuration | Method::FilenameOnly)
    }
}

/// How sure each step is, stored as `relink_confidence`. Nothing reads
/// these as thresholds yet; they rank the steps for Review (1aD).
pub mod confidence {
    /// Confirmed by the user.
    pub const CONFIRMED: f64 = 1.0;
    /// The Location names the file on a drive that's plugged in.
    pub const PATH: f64 = 1.0;
    /// The Location names where the file was when its drive was last seen.
    pub const PATH_OFFLINE: f64 = 0.9;
    pub const FILENAME_DURATION: f64 = 0.9;
    pub const UNIQUE_DURATION: f64 = 0.6;
}

/// Step 3 looks at no more files than this. In a bigger set, a single file
/// within the duration window is more likely chance than evidence; step 4
/// (the fingerprint, 1aD) is the tool for big folders.
pub const MAX_CANDIDATES: usize = 50;

/// A file that is present (`file.present = 1`), whether or not its drive
/// is plugged in.
#[derive(Debug, Clone)]
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
    /// when neither is known.
    pub path: Option<String>,
    pub online: bool,
    /// `None` until stage 2 has read it (or when it couldn't, or when the
    /// file is online-only and wasn't read).
    pub duration_ms: Option<i64>,
}

/// One `rekordbox_track` row.
#[derive(Debug, Clone)]
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
    /// Its match before this run.
    pub current: Option<(i64, Method)>,
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

/// A match to store.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Decision {
    /// The `rekordbox_track` row.
    pub track: i64,
    pub file: i64,
    pub method: Method,
    pub confidence: f64,
    /// Re-applied from the `relink` table.
    pub confirmed: bool,
}

/// What [`plan`] decided: the matches to store, and the rows left alone.
#[derive(Debug, Clone, Default)]
pub struct Plan {
    /// New or changed matches, in step order.
    pub decisions: Vec<Decision>,
    /// Rows already matched before this run and left as they were.
    pub kept: usize,
    /// Streaming entries: never matched.
    pub streaming: usize,
    /// Rows with no file after this run.
    pub missing: usize,
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

/// A path's key for matching: NFC, and letter case folded the way NTFS
/// folds it. The same key [`FilePath::match_key`] gives a Windows path.
pub fn path_key(path: &str) -> String {
    path.nfc().map(upcase).collect()
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

    fn place_of(&self, file: i64) -> Option<Place> {
        let f = &self.files[*self.by_id.get(&file)?];
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
}

/// What a step found for one track: the files that fit, and whether a file
/// whose duration isn't known yet could have fit too.
struct Fits {
    track: usize,
    files: Vec<i64>,
    blocked: bool,
}

/// Among `candidates`, the files whose duration fits `total_s`.
fn fits(index: &Index<'_>, track: usize, total_s: u32, candidates: &[usize]) -> Fits {
    let mut files = Vec::new();
    let mut blocked = false;
    for &i in candidates {
        let file = &index.files[i];
        match file.duration_ms {
            Some(d) if duration_fits(total_s, d) => files.push(file.id),
            Some(_) => {}
            None => blocked = true,
        }
    }
    Fits {
        track,
        files,
        blocked,
    }
}

/// The one-to-one matches among `found`: a track gets a file only if that
/// file is the only one that fits it, no other track in this step fits
/// that file, no file of unknown duration could also fit it, and no other
/// row already has the file.
fn one_to_one(found: &[Fits], claimed: &HashSet<i64>) -> Vec<(usize, i64)> {
    let mut wanted: HashMap<i64, usize> = HashMap::new();
    for f in found {
        for &file in &f.files {
            *wanted.entry(file).or_default() += 1;
        }
    }
    found
        .iter()
        .filter_map(|f| match f.files.as_slice() {
            [file] if !f.blocked && wanted[file] == 1 && !claimed.contains(file) => {
                Some((f.track, *file))
            }
            _ => None,
        })
        .collect()
}

/// Decides every row's match. Rows matched before this run keep their
/// match, except that a confirmed relink replaces it.
pub fn plan(input: &Input) -> Plan {
    let index = Index::new(&input.files);
    let mut out = Plan::default();
    // Each row's match as this run goes: file, method, whether confirmed.
    let mut matched: Vec<Option<(i64, Method, bool)>> = input
        .tracks
        .iter()
        .map(|t| t.current.map(|(file, method)| (file, method, false)))
        .collect();
    let is_file = |t: &Track| matches!(t.location, Some(Location::File(_)));
    let mut decide = |matched: &mut Vec<Option<(i64, Method, bool)>>,
                      i: usize,
                      file: i64,
                      method: Method,
                      confidence: f64,
                      confirmed: bool| {
        matched[i] = Some((file, method, confirmed));
        out.decisions.push(Decision {
            track: input.tracks[i].id,
            file,
            method,
            confidence,
            confirmed,
        });
    };

    // Step 0: a confirmed relink is re-applied first, over any automatic
    // match, as long as its file is still present.
    for (i, track) in input.tracks.iter().enumerate() {
        if !is_file(track) {
            continue;
        }
        let Some(c) = input.confirmed.get(&track.key) else {
            continue;
        };
        if !index.by_id.contains_key(&c.file) {
            continue;
        }
        if track.current != Some((c.file, c.method)) {
            decide(
                &mut matched,
                i,
                c.file,
                c.method,
                confidence::CONFIRMED,
                true,
            );
        } else {
            matched[i] = Some((c.file, c.method, true));
        }
    }

    // Step 1: the Location still names a present file.
    for (i, track) in input.tracks.iter().enumerate() {
        if matched[i].is_some() {
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
            decide(&mut matched, i, file, Method::Path, confidence, false);
        }
    }

    let claimed = |matched: &[Option<(i64, Method, bool)>]| -> HashSet<i64> {
        matched.iter().flatten().map(|&(file, _, _)| file).collect()
    };
    let open = |matched: &[Option<(i64, Method, bool)>]| -> Vec<(usize, u32)> {
        input
            .tracks
            .iter()
            .enumerate()
            .filter(|(i, t)| matched[*i].is_none() && is_file(t))
            .filter_map(|(i, t)| Some((i, t.total_s?)))
            .collect()
    };

    // Step 2: the same file name anywhere, and the duration agrees.
    let taken = claimed(&matched);
    let found: Vec<Fits> = open(&matched)
        .into_iter()
        .filter_map(|(i, total_s)| {
            let path = input.tracks[i].location.as_ref()?.as_file()?;
            let same_name = index.names.get(&path_key(path.file_name()))?;
            Some(fits(&index, i, total_s, same_name))
        })
        .collect();
    for (i, file) in one_to_one(&found, &taken) {
        decide(
            &mut matched,
            i,
            file,
            Method::FilenameDuration,
            confidence::FILENAME_DURATION,
            false,
        );
    }

    // Step 3: the only file of that duration in the candidate set: the
    // folder the Location's own folder still names, plus every folder that
    // its neighbours (rows from the same rekordbox folder) were matched
    // into by a step that says where files went.
    let mut went: HashMap<&str, HashSet<Place>> = HashMap::new();
    for (i, track) in input.tracks.iter().enumerate() {
        let Some((file, method, confirmed)) = matched[i] else {
            continue;
        };
        if !(confirmed || method.is_evidence()) || !is_file(track) {
            continue;
        }
        if let Some(place) = index.place_of(file) {
            went.entry(folder_of(&track.key)).or_default().insert(place);
        }
    }
    let taken = claimed(&matched);
    let found: Vec<Fits> = open(&matched)
        .into_iter()
        .filter_map(|(i, total_s)| {
            let track = &input.tracks[i];
            let mut places: HashSet<&Place> = HashSet::new();
            if let Some(path) = windows_path(track.location.as_ref()) {
                places.extend(
                    index
                        .places_at(folder_of(path.as_str()))
                        .into_iter()
                        .flatten(),
                );
            }
            places.extend(went.get(folder_of(&track.key)).into_iter().flatten());
            let candidates: Vec<usize> = places
                .into_iter()
                .filter_map(|p| index.places.get(p))
                .flatten()
                .copied()
                .collect();
            if candidates.is_empty() || candidates.len() > MAX_CANDIDATES {
                return None;
            }
            Some(fits(&index, i, total_s, &candidates))
        })
        .collect();
    for (i, file) in one_to_one(&found, &taken) {
        decide(
            &mut matched,
            i,
            file,
            Method::UniqueDuration,
            confidence::UNIQUE_DURATION,
            false,
        );
    }

    for (i, track) in input.tracks.iter().enumerate() {
        if !is_file(track) {
            if matches!(track.location, Some(Location::Streaming(_))) {
                out.streaming += 1;
            } else {
                out.missing += 1;
            }
        } else if matched[i].is_none() {
            out.missing += 1;
        } else if track.current.is_some() && track.current == matched[i].map(|(f, m, _)| (f, m)) {
            out.kept += 1;
        }
    }
    out
}
