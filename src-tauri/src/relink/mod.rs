//! Relink: matching each rekordbox track to its file (1aC-3 to 1aC-5,
//! ROADMAP 1.2, §2, §5.3).
//!
//! rekordbox names a track's file by its `Location`, and those go stale
//! when a DJ moves files, changes computers or renames things. Relink finds
//! the file again, and records how in `rekordbox_track.relink_method`,
//! `relink_confidence` and `relink_probable`, with `recording_id` set to
//! the file's track (`recording_file`; NULL until it's grouped). A wrong
//! match is worse than none: it attaches one track's cues, grid and play
//! counts to another track's file. So every step below matches only when
//! the evidence picks out exactly one file, and anything else stays
//! missing (with its rekordbox data intact) for the gig stick step (1aD-3)
//! or for the user.
//!
//! **Every run decides every row again**, steps 1 to 5 from scratch. Only
//! a confirmed relink, and a match from a step no run makes (gig stick,
//! 1aD-3), carry over, and only while their file is present, still holds
//! the audio the match was made to (see "The audio a match was made to"
//! below), and no confirmation gives that file to another row. A carried
//! match that's only probable is a fallback: an accepted match from steps
//! 1–4 (the path first) replaces it. A confirmation the user withdrew (its
//! `relink` row deleted) is decided again: a `user` match, or any match at
//! confidence 1.0, never carries over without its `relink` row, so a step
//! that makes carried matches stores less than 1.0. So a better step
//! replaces an old guess (the Location's own file turning up beats a
//! duration match, an audio match or a name guess), a match to a file
//! that's gone or holds other audio now is dropped, and a guess never
//! keeps a file the user has confirmed for another track. Running again
//! with nothing changed changes nothing, and so does a fresh rekordbox
//! read (which replaces every `rekordbox_track` row): for steps 1–5 a
//! run's result depends on what's on disk and in the snapshot, never on
//! what earlier runs stored. (A carried gig-stick match lives on the
//! `rekordbox_track` row, which a read replaces; when 1aD-3 makes them,
//! they should be kept like a confirmation, in a `relink` row.) (Steps 4 and 5 were carried over in an earlier draft; that
//! made results depend on run history, so they're remade like steps 1–3.)
//!
//! In order, each step over every row before the next step starts:
//!
//! 0. **A confirmed relink** (a `same` row of the `relink` table, by the
//!    Location's match key) is re-applied first, over any automatic match, as long as
//!    its file is still present, at confidence 1.0 and never probable. The
//!    one exception: if the file is known to hold other audio than the
//!    user confirmed, the confirmation is kept and still holds the file,
//!    but it's applied as **probable** (confidence 0.5, no rekordbox data
//!    attached) until the user confirms again. It's never deleted here.
//! 1. **Path** (`path`): the decoded Location still names a present file,
//!    compared by the NFC match key with letter case folded, so an NFD
//!    Location finds its NFC file and `e:/MUSIC` finds `E:\Music`. A drive
//!    that's plugged in is asked by where it's mounted now. Only if none
//!    holds the path is a drive that isn't plugged in asked, by where it
//!    was last mounted (`last_mount_path`), and only when no other known
//!    volume was last mounted (or is mounted now) there: two volumes at one
//!    letter, such as a backup clone, give no match. `last_mount_path` only
//!    matches a Location to a file the scan already indexed; it never finds
//!    files on disk. If two files share the key (an NFC name and its NFD
//!    twin, two files on NTFS), the one spelled exactly like the Location
//!    wins, as on Windows; otherwise no match. Path matches aren't
//!    one-to-one: two rows at one path (a confirmed relink and a path
//!    match, or two spellings of one Location) share the file, as they do
//!    in rekordbox, so a Library start must expect several rows on a file.
//!    - *False-match risk:* low. The path is what rekordbox itself opens.
//!      It can be wrong when the file at that path was replaced by other
//!      audio under the same name, which no path rule can see, or when a
//!      drive that isn't plugged in had the letter of the drive the export
//!      meant (confidence 0.9, not 1.0).
//! 2. **File name + duration** (`filename_duration`): a present file
//!    anywhere with the same name (NFC, letter case ignored) whose duration
//!    fits `TotalTime`. rekordbox truncates `TotalTime` to whole seconds
//!    (§5.3), so its own duration is in `[T, T+1)` s; allowing ±0.5 s, the
//!    file must last from `T·1000 − 500` ms up to, not including,
//!    `T·1000 + 1500` ms ([`duration_fits`]).
//!    - *False-match risk:* two different recordings with the same file
//!      name and durations within about 2 s: a clean and a dirty cut both
//!      named `01 Title.mp3` in two folders, or generic names
//!      (`Track 01.mp3`). Guarded by being one-to-one: the track must have
//!      exactly one fitting file, that file must fit no other unmatched
//!      track, no same-named file of unknown duration may exist (it could
//!      fit too), and a file another row already has is never taken. So
//!      one surviving copy of two rekordbox entries stays unmatched rather
//!      than attached to both.
//! 3. **Unique duration** (`unique_duration`): the only file of fitting
//!    duration in the track's **candidate set**. Tracks from one rekordbox
//!    folder share it: the files directly in
//!    - the folder the Location's own folder still names (a file renamed
//!      where it was), found the same way as step 1; and
//!    - every folder that the track's neighbours were matched into, where
//!      neighbours are rows whose Location is in the same rekordbox folder
//!      and "matched" means confirmed, or trusted and by a step that says
//!      where a file went (path, filename + duration, or a carried gig
//!      stick match). Duration-only and name-only guesses aren't evidence,
//!      so guesses never chain; a step-4 match is made after step 3, so
//!      it's never evidence either (in the run that makes it or any other).
//!
//!    Why this set: a renamed file is usually still in its folder, and a
//!    moved one usually moved with its album or crate. Anywhere else,
//!    duration alone means nothing: in a library of thousands, dozens of
//!    files share any two-second window. A set of more than
//!    [`MAX_CANDIDATES`] (50) files skips step 3 for its tracks (it isn't
//!    cut down to 50); the size is summed per folder before any file is
//!    looked at, so a huge set costs nothing.
//!    - *False-match risk:* the highest of the three. When the track's own
//!      file was deleted, another file in the same folder can happen to
//!      fit: a clean/dirty or radio/extended cut can differ by under 2 s.
//!      Guarded by: uniqueness counted over every present file in the set,
//!      including files other rows already have (so a claimed look-alike
//!      blocks the match instead of being skipped over); the one fitting
//!      file must be unclaimed, and no other unmatched track may want it,
//!      whether by duration from a set holding it (even one over the cap)
//!      or by name (step 2's fits); and uniqueness must be provable, so if
//!      any file in the set has an unknown duration (not read yet,
//!      online-only, or unreadable), there's no match until a later run.
//!
//!    **A second signal decides whether it's trusted.** The match is
//!    accepted (confidence 0.8) only when one of the file's title tags
//!    (read only for the files step 3 matches)
//!    agrees with rekordbox's `Name` ([`rules::title_key`]: NFKC,
//!    lowercase, featuring credits dropped, punctuation and spacing folded,
//!    bracket contents kept, so `(Clean)` never agrees with `(Dirty)`).
//!    Otherwise, and when the file has no title tag, it's stored as
//!    **probable** (`relink_probable = 1`, confidence 0.4), like a
//!    filename-only match: its file is taken, but no rekordbox data is
//!    attached until the user confirms it in Review. The remaining risk,
//!    a deleted file plus one look-alike cut in its folder, lands there.
//! 4. **Fingerprint** (`fingerprint`): the audio of a file that's gone. A
//!    rekordbox track with no file has no audio to fingerprint, so the
//!    evidence is what the app knew before: a `file` row the walk found
//!    gone (`present = 0`; the row is kept, with its `audio_hash`, track
//!    and fingerprint) that this row once named. That's the file of its
//!    confirmed relink, else the gone file at its Location (found like
//!    step 1). Its `audio_hash` counts only if the hash stage was current
//!    for it when it went, and its fingerprint only if the fingerprint
//!    stage was: a file that changed and then went before the stages ran
//!    again left values of its older content, which say nothing. The
//!    candidates:
//!    - present files no other row holds with the gone file's
//!      `audio_hash`: **the same audio**, accepted at confidence 0.95. A
//!      candidate counts only while its own hash is current: a file that
//!      changed since it was hashed may hold anything. Any of them is
//!      right, so the file a Library track is linked to is matched, else
//!      the track's best file (else the lowest file id): see "Exact copies
//!      aren't ambiguous" below. (For the candidates further down, which a
//!      title tag or a length can make probable, the linked file is only a
//!      tie-break at equal trust.)
//!
//!    - else, present files no other row holds that are **of the gone
//!      file's track** (1bC-7), on grouping's own current answer for the
//!      two files themselves ([`crate::grouping::current_duplicates`]).
//!      Sharing a track isn't enough: a merge is never undone, so a mate
//!      can sit there though the rules no longer call it a duplicate, or
//!      without ever having been compared with the gone file. So all of
//!      this must hold:
//!      - a pair of the two files is stored as a duplicate, and a grouping
//!        pass found them in one track;
//!      - grouping is current for both: each one's pairs are decided
//!        under the current rules, and each sits in the track with the
//!        audio it holds (the gone file: held when it went). A file that
//!        is due to be decided again, or has no grouping record, isn't;
//!      - the mate's hash is current, and the gone file's was when it
//!        went: grouping can't see a file that changed since it was
//!        hashed.
//!
//!      So a mate overwritten with other audio isn't matched, before
//!      grouping has caught up or after. As for the same audio, any
//!      present file counts (on a drive that's plugged in or not), and of
//!      several the track's best file is matched (else the lowest file
//!      id). It costs no fingerprint comparison.
//!
//!      It's accepted (confidence 0.9) only when both of these hold, and
//!      otherwise stored as **probable** (confidence 0.4):
//!      - the mate lasts within [`rules::FINGERPRINT_WINDOW_MS`] (1 s) of
//!        the gone file, the limit the fingerprint duplicates below are
//!        found within. rekordbox's cues and grid are positions in time,
//!        and grouping calls two files duplicates at 90% of each found in
//!        the other, whatever their lengths: a copy with a longer lead-in,
//!        or a cut-off one, has them elsewhere. A length that isn't known,
//!        on either side, is another length;
//!      - one of the mate's title tags agrees with rekordbox's `Name`,
//!        like a fingerprint duplicate: grouping compared the two files'
//!        names with each other, never with what rekordbox calls the
//!        entry.
//!
//!      *Two limits.* Once the mate is decided again while the gone file
//!      is away (a retag, new rules), their pair goes with that decision
//!      and this rule no longer applies to the row. And every new version
//!      of the grouping rules (or of the comparison) ends it for the files
//!      that were already gone: a gone file is never decided again, so
//!      its record is never current again. The fingerprint duplicates are
//!      then what remains.
//!    - else, the fingerprint duplicates of the gone file (ROADMAP 1.4:
//!      at least 90% matched both ways, difference at most 4) among every
//!      present file lasting within [`rules::FINGERPRINT_WINDOW_MS`] (1 s)
//!      of it: a re-encode or another format. Exactly one, or several in
//!      one track or with one `audio_hash`; if they differ, no match, and
//!      if another row holds any of them, none is taken (a held duplicate
//!      counts against uniqueness, as in steps 2, 3 and 5).
//!
//!    One-to-one: if another unmatched row's gone file leads to any of the
//!    same candidates, neither row is matched (two rekordbox entries, one
//!    surviving copy).
//!    - *False-match risk:* the same audio can't be a wrong file. A
//!      fingerprint duplicate can be another version: a Clean and a Dirty
//!      cut fingerprint alike (ROADMAP 1.4). So, as in step 3, it's
//!      accepted (confidence 0.85) only when one of the file's title tags
//!      agrees with rekordbox's `Name`; otherwise it's stored as
//!      **probable** (confidence 0.4).
//!    - *The unplugged-drive rule:* a drive that isn't plugged in is never
//!      walked, so its files stay present and step 1 matches them where
//!      the drive was last mounted. Only a walk that listed a file's folder
//!      and didn't find it marks it gone, so step 4 never fires for a drive
//!      that's merely unplugged.
//!    - *Never on a partial comparison:* a row is compared with every
//!      file of about its length, or it gets no fingerprint match this
//!      run. A file there that the fingerprint stage hasn't looked at yet
//!      could be the duplicate, so the row waits (the relink after the
//!      fingerprint job decides it). And no run compares more than
//!      [`rules::MAX_COMPARISONS`] pairs, because relink runs on the one
//!      writer: rows are taken fewest candidates first (then by row id),
//!      and the rows the budget doesn't reach are left undecided, counted
//!      in [`Summary::fingerprint_undecided`] and logged (a row that's
//!      only waiting for a fingerprint isn't counted there). A waiting or
//!      undecided row's candidates (every file of about its length) still
//!      count as sought, so no other row is given one of them meanwhile,
//!      not even as the same audio. A file the stage failed or skipped
//!      (can't be fingerprinted, online-only) has no fingerprint and isn't
//!      a candidate. Nor is a file of unknown duration: one that hasn't
//!      been read yet isn't "about that length", so a relink during a read
//!      stage can decide without it, and the relink after the fingerprint
//!      job decides again.
//! 5. **File name only** (`filename_only`): the only present file with the
//!    Location's file name (NFC, letter case ignored), or exact copies of
//!    one. Always stored as **probable** (`relink_probable = 1`, confidence
//!    0.5): its file is taken, but no rekordbox data is attached until the
//!    user confirms it in Review.
//!    - *False-match risk:* high, which is why it's never trusted: generic
//!      names (`Track 01.mp3`), or another cut under the same name.
//!      Guarded by: a file whose known duration contradicts a usable
//!      `TotalTime` (outside step 2's window) isn't a candidate, since the
//!      same name at another length is more likely another cut (an unknown
//!      duration on either side contradicts nothing); uniqueness is counted
//!      over every other present file with that name, including one another
//!      row holds (a held look-alike blocks the match instead of being
//!      skipped over); the file must be unheld; and two unmatched rows with
//!      the same file name both stay unmatched.
//!
//! **"Not the same"** (1bE-6). The user can say a probable match is wrong:
//! the file is not that entry's ([`decide::reject_match`], a `not_same` row
//! of `relink`, by the Location's match key and the audio the file holds:
//! its `audio_hash`, or its `blake3` when it has none). From then on no run
//! gives that row a file holding that audio in steps 2 to 5, wherever the
//! file is now (moved, renamed, its music folder removed and added again),
//! and a carried match to one is dropped. Step 1 isn't affected: a file at
//! the Location's own path is the file rekordbox itself opens, and is never
//! a probable match to say no to. The step that would have made the match
//! makes none for the row, and the row goes on through the later steps,
//! which may find another file: a new item in Review. The file still counts
//! wherever the rules count files (a name shared by two files is ambiguous,
//! whatever was said about one of them), so a "not the same" can only take
//! a match away, never make one. A file encoded again holds other audio: it
//! is another question.
//!
//! **The audio a match was made to.** Every match records the matched
//! file's `audio_hash` (`rekordbox_track.relink_audio_hash`), and a
//! confirmed relink the hash at confirmation (`relink.audio_hash`,
//! [`decide::confirm_match`]). It's what a confirmation, and a carried (gig stick)
//! match, are checked against: a carried match whose file now has another
//! `audio_hash` is dropped and the row decided again; a confirmation is
//! downgraded as in step 0. Steps 1–5 are remade by every run, so for
//! them nothing reads it: it only says what the file held, and is
//! recorded for every match so that 1aD-3's gig-stick matches have it
//! from the run that makes them. Only a hash that's current for
//! the file counts, and only a current hash is ever recorded (the hash
//! stage done at the file's size and mtime now): a tag-only edit keeps the
//! `audio_hash`, so it changes nothing, and a stale or missing hash means
//! unknown, never changed. Where no hash was recorded (the file wasn't
//! hashed yet, or the row is older than this), the next run that finds the
//! file's hash current records it.
//!
//! One consequence for step 4: while a matched file's hash is stale (it
//! changed on disk and hasn't been hashed again), it isn't known to hold
//! the gone file's audio, so the same-audio match is withdrawn until the
//! hash stage has run, and comes back if the audio is the same.
//!
//! **Exact copies aren't ambiguous.** When every file that fits in step 2,
//! 3 or 5 has the same `audio_hash` (current for each file), they hold the
//! same audio: one of them is matched, not nothing; if another row holds
//! any of the copies, none is taken. Which one, here and among the
//! candidates of step 4 ([`rules`], `Pick`): the track's best file, else
//! the lowest file id; but the file a Library track is linked to instead,
//! wherever a match to it gets the same trust as a match to that one. A
//! Library track is sent as rekordbox's entry only while the entry is
//! matched to its linked file (1bE-10): a copy that turns up later, and
//! ranks ahead, mustn't pull the entry off it. It is a tie-break only. In
//! steps 3 and 4 a match is trusted on the file's own title tags and
//! length, and the linked file is matched only if it would be trusted
//! exactly as the usual file would: being linked never turns a trusted
//! match into a probable one, or a probable one into a trusted one. A row
//! none of whose candidates is linked gets the file it always got.
//!
//! **Relink never changes a Library track's linked file.** The step after
//! it does, in the same job and in a transaction of its own
//! ([`crate::library::follow`]): a Library track whose linked file is gone
//! follows it to the same audio.
//!
//! A file of unknown duration blocks steps 2 and 3: a same-named file
//! that's online-only (never read unless the user opts in) or not read yet
//! makes the name ambiguous, and the track falls through to step 3. A file
//! whose duration can never be read (broken, or online-only for good)
//! blocks its name and its folder for good; steps 4 and 5 and the user
//! are the way past it (step 5 doesn't look at durations).
//!
//! **Never on size** (§5.3): rekordbox rewrites tags, so sizes change and
//! the XML's `Size` can be stale. Nothing here reads it.
//!
//! **Streaming entries** (`soundcloud:…`) are never matched, not even by a
//! confirmed relink.
//!
//! Relink reads only the database: no file is opened. It runs as a job
//! ([`job`]), queued after every rekordbox read, after a scan's read and
//! fingerprint stages (`scan::chain`), after every grouping run, and
//! again when asked. A relink run asks for an attach and nothing else (it
//! never queues grouping), so the jobs can't go round in a loop.

pub mod decide;
pub mod job;
pub mod rules;

use std::collections::{HashMap, HashSet};

use rusqlite::{params, Connection, OptionalExtension};

use crate::fingerprint::Fingerprint;
use crate::paths::Volumes;
use crate::volume::VolumeId;

pub use job::{relink_job, relink_rekordbox_tracks, relinker, request, Relinker};
pub use rules::{duration_fits, Method, FINGERPRINT_WINDOW_MS, MAX_CANDIDATES, MAX_COMPARISONS};

/// Whether the hash stage is current for file `f`: done by this build's
/// hash definition (`?1`) at the file's size and mtime now, so `audio_hash`
/// is what the file holds now.
const HASH_CURRENT: &str = "EXISTS (SELECT 1 FROM file_stage s
             WHERE s.file_id = f.id AND s.stage = 'hash' AND s.status = 'done'
               AND s.version = ?1 AND s.size IS f.size AND s.mtime IS f.mtime)";

/// [`HASH_CURRENT`] for a `file` row under another alias than `f`: the one
/// rule for "this hash is what the file holds now", for the code outside
/// relink that asks it ([`crate::library::follow`]). For a file that's
/// gone it says the hash was current when it went: the row keeps the size
/// and mtime it had then. Takes [`hash_version`] as `?1`.
pub(crate) fn hash_current_of(alias: &str) -> String {
    format!(
        "EXISTS (SELECT 1 FROM file_stage s
             WHERE s.file_id = {alias}.id AND s.stage = 'hash' AND s.status = 'done'
               AND s.version = ?1 AND s.size IS {alias}.size AND s.mtime IS {alias}.mtime)"
    )
}

/// The hash stage's version in `file_stage`: what [`HASH_CURRENT`] takes
/// as `?1`.
pub(crate) fn hash_version() -> i64 {
    i64::from(crate::hash::DEFINITION)
}

/// Where each known volume is mounted right now, by its stored identity:
/// the mount point as the OS gives it (`E:\`, `\\?\E:\`,
/// `\\?\UNC\server\share\`). A volume not listed isn't plugged in.
#[derive(Debug, Clone, Default)]
pub struct Mounted(HashMap<String, String>);

impl Mounted {
    /// From `(identity, mount point)` pairs.
    pub fn new<I, S, T>(pairs: I) -> Mounted
    where
        I: IntoIterator<Item = (S, T)>,
        S: Into<String>,
        T: Into<String>,
    {
        Mounted(
            pairs
                .into_iter()
                .map(|(id, mount)| (id.into(), mount.into()))
                .collect(),
        )
    }

    /// Asks `volumes` where each of `identities` is mounted now.
    pub fn ask(identities: &[String], volumes: &impl Volumes) -> Mounted {
        Mounted(
            identities
                .iter()
                .filter_map(|identity| {
                    let id = VolumeId::from_stored(identity.clone()).ok()?;
                    let mount = volumes.mount_path(&id)?;
                    Some((identity.clone(), mount.to_string_lossy().into_owned()))
                })
                .collect(),
        )
    }
}

/// A [`Mounted`] answers where a volume is as the volumes it was asked of
/// did, without asking anything again: a snapshot to hand to code that runs
/// on the database's writer and must not ask the system there
/// ([`crate::library::follow`]). It knows mount points only, so it doesn't
/// look a path's volume up.
impl Volumes for Mounted {
    fn volume_for(&self, path: &std::path::Path) -> std::io::Result<crate::volume::Volume> {
        Err(std::io::Error::other(format!(
            "a mount snapshot doesn't look up the volume of {}",
            path.display()
        )))
    }

    fn mount_path(&self, id: &VolumeId) -> Option<std::path::PathBuf> {
        self.0.get(id.as_str()).map(std::path::PathBuf::from)
    }
}

/// Every volume identity the library knows.
pub fn identities(conn: &Connection) -> rusqlite::Result<Vec<String>> {
    conn.prepare("SELECT identity FROM volume ORDER BY id")?
        .query_map([], |r| r.get(0))?
        .collect()
}

/// A mount point as a `/`-separated path without a trailing `/`: `E:`,
/// `C:/mnt/usb`, `//server/share`. `None` if it's neither a drive nor a
/// network path.
fn mount_text(mount: &str) -> Option<String> {
    let plain = if let Some(unc) = mount.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{unc}")
    } else if let Some(local) = mount.strip_prefix(r"\\?\") {
        local.to_owned()
    } else {
        mount.to_owned()
    };
    let text = plain.replace('\\', "/");
    let text = text.trim_end_matches('/');
    let b = text.as_bytes();
    let drive = b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':';
    let network = text.len() > 2 && text.starts_with("//") && !text[2..].starts_with('/');
    (drive || network).then(|| text.to_owned())
}

/// What a run left, row by row.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Summary {
    /// Rows with a confirmed relink.
    pub confirmed: u32,
    /// Rows matched by each step, trusted.
    pub path: u32,
    pub filename_duration: u32,
    pub unique_duration: u32,
    pub fingerprint: u32,
    /// Rows step 4 left undecided because they were over the run's
    /// fingerprint comparison budget ([`MAX_COMPARISONS`]). Rows waiting
    /// for a file's fingerprint aren't counted.
    pub fingerprint_undecided: u32,
    /// Rows matched but only probable, waiting for the user to confirm:
    /// every filename-only match, a unique duration, a track mate or a
    /// fingerprint duplicate no title tag agrees with, and a confirmation
    /// whose file's audio changed.
    pub probable: u32,
    /// Rows keeping a match from a step this run doesn't make (gig stick).
    pub other: u32,
    /// Streaming entries, never matched.
    pub streaming: u32,
    /// Rows without a file.
    pub missing: u32,
    /// Rows this run wrote: a new, changed or dropped match, a changed
    /// track, or the matched file's audio recorded.
    pub changed: u32,
}

/// Decides every `rekordbox_track` row's match again and stores what
/// changed, in one transaction. See the module docs for the rules. With
/// no rekordbox tracks at all it returns at once, before any file is
/// loaded.
pub fn relink(conn: &mut Connection, mounted: &Mounted) -> rusqlite::Result<Summary> {
    let tx = conn.transaction()?;
    // Nothing to match: a library that doesn't use rekordbox is asked for a
    // relink after every grouping run, and mustn't load every file for it.
    let any_tracks: bool =
        tx.query_row("SELECT EXISTS (SELECT 1 FROM rekordbox_track)", [], |r| {
            r.get(0)
        })?;
    if !any_tracks {
        return Ok(Summary::default());
    }
    let input = load(&tx, mounted)?;
    let plan = {
        let mut raw_tags = tx.prepare("SELECT raw_tags FROM file WHERE id = ?1")?;
        // A file's fingerprint only while the fingerprint stage is current
        // for it: a changed file's old fingerprint isn't its own any more.
        // The same for a gone file, by the size and mtime it had when it
        // went (its rows keep them): one that changed and then went before
        // the stage ran again left a fingerprint of older content. A blob
        // this build can't read is no fingerprint.
        let mut fingerprint = tx.prepare(
            "SELECT f.fingerprint FROM file f
             WHERE f.id = ?1 AND f.fingerprint IS NOT NULL
               AND EXISTS (
                   SELECT 1 FROM file_stage s
                   WHERE s.file_id = f.id AND s.stage = 'fingerprint' AND s.status = 'done'
                     AND s.version = ?2 AND s.size IS f.size AND s.mtime IS f.mtime)",
        )?;
        rules::plan(
            &input,
            |file| {
                let tags: Option<String> = raw_tags.query_row([file], |r| r.get(0))?;
                Ok::<_, rusqlite::Error>(tags.as_deref().map(titles).unwrap_or_default())
            },
            |file| {
                let version = i64::from(crate::fingerprint::VERSION);
                let blob: Option<Vec<u8>> = fingerprint
                    .query_row(params![file, version], |r| r.get(0))
                    .optional()?;
                Ok(blob.and_then(|b| Fingerprint::from_blob(&b).ok()))
            },
            |file| crate::grouping::current_duplicates(&tx, file),
        )?
    };
    {
        let mut update = tx.prepare(
            "UPDATE rekordbox_track
             SET file_id = ?2, relink_method = ?3, relink_confidence = ?4,
                 relink_probable = ?5, recording_id = ?6, relink_audio_hash = ?7
             WHERE id = ?1",
        )?;
        for d in &plan.decisions {
            let t = d.target;
            update.execute(params![
                d.track,
                t.map(|t| t.file),
                t.map(|t| t.method.as_str()),
                t.and_then(|t| t.confidence),
                t.is_some_and(|t| t.probable),
                d.recording,
                d.evidence,
            ])?;
        }
        // Never over a hash that's already recorded.
        let mut record = tx.prepare(
            "UPDATE relink SET audio_hash = ?2
             WHERE location_key = ?1 AND verdict = 'same' AND audio_hash IS NULL",
        )?;
        for (key, audio) in &plan.confirmed_audio {
            record.execute(params![key, audio])?;
        }
        // A confirmation made before its file was hashed gets the audio it
        // is kept by, once that is known (never over one that is there).
        tx.execute(
            &format!(
                "UPDATE relink SET file_key =
                     (SELECT {key} FROM file f WHERE f.id = relink.file_id)
                 WHERE file_key IS NULL AND file_id IS NOT NULL
                   AND EXISTS (SELECT 1 FROM file f
                               WHERE f.id = relink.file_id AND {key} IS NOT NULL)",
                key = decide::FILE_KEY
            ),
            [],
        )?;
    }
    tx.commit()?;
    Ok(Summary {
        confirmed: count(plan.confirmed),
        path: count(plan.path),
        filename_duration: count(plan.filename_duration),
        unique_duration: count(plan.unique_duration),
        fingerprint: count(plan.fingerprint),
        fingerprint_undecided: count(plan.over_budget),
        probable: count(plan.probable),
        other: count(plan.other),
        streaming: count(plan.streaming),
        missing: count(plan.missing),
        changed: count(plan.decisions.len()),
    })
}

fn count(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// A known volume's mount point as the rules use it.
pub(crate) struct VolumeMount {
    pub id: i64,
    pub identity: String,
    /// Where the volume is mounted now, or, if it isn't plugged in, where it
    /// was last mounted, as a `/`-separated path ([`mount_text`]). `None` if
    /// there's none, or if the volume isn't plugged in and another known
    /// volume was last mounted (or is mounted now) at the same place.
    pub mount: Option<String>,
    /// Whether the volume is plugged in.
    pub online: bool,
}

/// Where each known volume is, for matching a rekordbox Location to a file
/// the scan already indexed.
///
/// `last_mount_path` is never used to find files on disk. It counts only
/// when no other known volume was last mounted (or is mounted now) at the
/// same place: two volumes last at one letter (a backup clone) can't say
/// which one a Location meant. Everything that places a file by where its
/// volume was last mounted uses this rule.
pub(crate) fn volume_mounts(
    conn: &Connection,
    mounted: &Mounted,
) -> rusqlite::Result<Vec<VolumeMount>> {
    let mut volumes: Vec<(i64, String, Option<String>, bool)> = Vec::new();
    let mut at: HashMap<String, HashSet<i64>> = HashMap::new();
    {
        let mut stmt = conn.prepare("SELECT id, identity, last_mount_path FROM volume")?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<String>>(2)?,
            ))
        })?;
        for row in rows {
            let (id, identity, last) = row?;
            let last = last.as_deref().and_then(mount_text);
            let now = mounted.0.get(&identity).and_then(|m| mount_text(m));
            for place in [&last, &now].into_iter().flatten() {
                at.entry(rules::path_key(place)).or_default().insert(id);
            }
            match now {
                Some(now) => volumes.push((id, identity, Some(now), true)),
                None => volumes.push((id, identity, last, false)),
            }
        }
    }
    Ok(volumes
        .into_iter()
        .map(|(id, identity, mount, online)| {
            let alone = |m: &String| at.get(&rules::path_key(m)).is_some_and(|v| v.len() == 1);
            VolumeMount {
                id,
                identity,
                mount: mount.filter(|m| online || alone(m)),
                online,
            }
        })
        .collect())
}

/// Reads what the rules look at.
fn load(conn: &Connection, mounted: &Mounted) -> rusqlite::Result<rules::Input> {
    // Each music folder's path: under its volume's mount point now, or
    // where the volume was last mounted if it isn't plugged in
    // ([`volume_mounts`]).
    let mounts: HashMap<i64, (Option<String>, bool)> = volume_mounts(conn, mounted)?
        .into_iter()
        .map(|v| (v.id, (v.mount, v.online)))
        .collect();
    let mut folders: HashMap<i64, (Option<String>, bool)> = HashMap::new();
    {
        let mut stmt = conn.prepare("SELECT id, volume_id, rel_path FROM music_folder")?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?;
        for row in rows {
            let (id, volume, rel) = row?;
            let (mount, online) = mounts.get(&volume).cloned().unwrap_or((None, false));
            let base = mount.map(|m| {
                if rel.is_empty() {
                    m
                } else {
                    format!("{m}/{rel}")
                }
            });
            folders.insert(id, (base, online));
        }
    }

    let mut files = Vec::new();
    let mut absent = Vec::new();
    {
        let mut stmt = conn.prepare(&format!(
            "SELECT f.id, f.music_folder_id, f.rel_path, f.duration_ms,
                    f.audio_hash, rf.recording_id, rf.role = 'best', f.present,
                    {HASH_CURRENT},
                    NOT EXISTS (SELECT 1 FROM file_stage s
                                WHERE s.file_id = f.id AND s.stage = 'fingerprint'
                                  AND s.version = ?2 AND s.size IS f.size
                                  AND s.mtime IS f.mtime),
                    EXISTS (SELECT 1 FROM library_track lt WHERE lt.linked_file_id = f.id),
                    {}
             FROM file f LEFT JOIN recording_file rf ON rf.file_id = f.id
             ORDER BY f.id",
            decide::FILE_KEY
        ))?;
        let fingerprint_version = i64::from(crate::fingerprint::VERSION);
        let rows = stmt.query_map([hash_version(), fingerprint_version], |r| {
            Ok((
                (
                    r.get::<_, i64>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, Option<i64>>(3)?,
                    r.get::<_, Option<Vec<u8>>>(4)?,
                ),
                (
                    r.get::<_, Option<i64>>(5)?,
                    r.get::<_, Option<bool>>(6)?,
                    r.get::<_, bool>(7)?,
                    r.get::<_, bool>(8)?,
                    r.get::<_, bool>(9)?,
                    r.get::<_, bool>(10)?,
                    r.get::<_, Option<Vec<u8>>>(11)?,
                ),
            ))
        })?;
        for row in rows {
            let (
                (id, folder, rel, duration_ms, audio_hash),
                (recording, best, present, current, fingerprint_pending, linked, content_key),
            ) = row?;
            let (base, online) = folders.get(&folder).cloned().unwrap_or((None, false));
            let path = base.map(|b| format!("{b}/{rel}"));
            let audio_hash = audio_hash.filter(|h| !h.is_empty());
            if !present {
                // Gone: kept for what it says about the audio that was
                // there (step 4). A hash that wasn't current when the file
                // went is of older content.
                absent.push(rules::Absent {
                    id,
                    path,
                    duration_ms,
                    audio_hash: audio_hash.filter(|_| current),
                    hash_current: current,
                });
                continue;
            }
            let (parent, name) = match rel.rsplit_once('/') {
                Some((parent, name)) => (parent.to_owned(), name.to_owned()),
                None => (String::new(), rel.clone()),
            };
            files.push(rules::File {
                id,
                folder,
                parent,
                name,
                path,
                online,
                duration_ms,
                audio_hash,
                hash_current: current,
                fingerprint_pending,
                recording,
                best: best.unwrap_or(false),
                linked,
                content_key: content_key.filter(|k| !k.is_empty()),
            });
        }
    }

    let mut tracks = Vec::new();
    {
        // As text, whatever JSON type the value has: a number where a
        // string belongs mustn't fail the run.
        let mut stmt = conn.prepare(
            "SELECT id, location, location_key,
                    CAST(json_extract(attributes, '$.TotalTime') AS TEXT),
                    CAST(json_extract(attributes, '$.Name') AS TEXT),
                    file_id, relink_method, relink_confidence, relink_probable, recording_id,
                    relink_audio_hash
             FROM rekordbox_track ORDER BY id",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                (
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, Option<String>>(3)?,
                    r.get::<_, Option<String>>(4)?,
                ),
                (
                    r.get::<_, Option<i64>>(5)?,
                    r.get::<_, Option<String>>(6)?,
                    r.get::<_, Option<f64>>(7)?,
                    r.get::<_, bool>(8)?,
                    r.get::<_, Option<i64>>(9)?,
                    r.get::<_, Option<Vec<u8>>>(10)?,
                ),
            ))
        })?;
        for row in rows {
            let (
                (id, location, key, total, name),
                (file, method, confidence, probable, recording, evidence),
            ) = row?;
            let total_s = total
                .as_deref()
                .and_then(crate::rekordbox::attrs::digits::<u32>)
                .filter(|&t| t > 0);
            // The table allows only the methods `Method` knows; should a
            // newer build add one, the row keeps its file (as the user's).
            let current = file.map(|file| rules::Target {
                file,
                method: method
                    .as_deref()
                    .and_then(Method::parse)
                    .unwrap_or(Method::User),
                confidence,
                probable,
            });
            tracks.push(rules::Track {
                id,
                key,
                location: crate::rekordbox::location::decode(&location).ok(),
                total_s,
                name: name.as_deref().map(rules::title_key).unwrap_or_default(),
                current,
                evidence: evidence.filter(|h| !h.is_empty()),
                recording,
            });
        }
    }

    let mut confirmed = HashMap::new();
    {
        // The file a confirmation points at; when that row is gone (its
        // music folder was removed), a file on disk with the audio it was
        // made for. A confirmation with neither waits for one.
        let mut stmt = conn.prepare(
            "SELECT r.location_key,
                    coalesce(r.file_id,
                             (SELECT f.id FROM file f
                              WHERE f.present = 1
                                AND (f.audio_hash = r.file_key OR f.blake3 = r.file_key)
                              ORDER BY f.id LIMIT 1)),
                    r.method, r.audio_hash
             FROM relink r WHERE r.verdict = 'same'",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, Option<i64>>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, Option<Vec<u8>>>(3)?,
            ))
        })?;
        for row in rows {
            let (key, file, method, audio_hash) = row?;
            let Some(file) = file else {
                continue;
            };
            if let Some(method) = Method::parse(&method) {
                let audio_hash = audio_hash.filter(|h| !h.is_empty());
                confirmed.insert(
                    key,
                    rules::Confirmed {
                        file,
                        method,
                        audio_hash,
                    },
                );
            }
        }
    }

    // What the user said is not an entry's audio, by Location key.
    let mut rejected: HashMap<String, Vec<rules::Rejected>> = HashMap::new();
    {
        let mut stmt = conn.prepare(
            "SELECT location_key, file_key FROM relink WHERE verdict = 'not_same' ORDER BY id",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, Vec<u8>>(1)?))
        })?;
        for row in rows {
            let (key, audio) = row?;
            rejected
                .entry(key)
                .or_default()
                .push(rules::Rejected { audio });
        }
    }

    Ok(rules::Input {
        files,
        absent,
        tracks,
        confirmed,
        rejected,
    })
}

/// The title tags in a file's `raw_tags` (one array of `{key, value}` per
/// tag block type), each as a [`rules::title_key`]. Every block's own
/// title field counts: ID3v2 `TIT2`, ID3v1 `title`, APE and Vorbis
/// `TITLE` (any case), MP4 `©nam`, RIFF `INAM`, AIFF `NAME`.
fn titles(raw_tags: &str) -> Vec<String> {
    let Ok(serde_json::Value::Object(blocks)) = serde_json::from_str(raw_tags) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (block, items) in &blocks {
        let is_title = |key: &str| match block.as_str() {
            "id3v2" => key == "TIT2",
            "id3v1" => key == "title",
            "ape" | "vorbis_comments" => key.eq_ignore_ascii_case("title"),
            "mp4_ilst" => key == "\u{a9}nam",
            "riff_info" => key == "INAM",
            "aiff_text" => key == "NAME",
            _ => false,
        };
        for item in items.as_array().into_iter().flatten() {
            let key = item.get("key").and_then(|k| k.as_str()).unwrap_or("");
            let text = item
                .get("value")
                .and_then(|v| v.get("text"))
                .and_then(|t| t.as_str());
            if let (true, Some(text)) = (is_title(key), text) {
                let title = rules::title_key(text);
                if !title.is_empty() {
                    out.push(title);
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests;
