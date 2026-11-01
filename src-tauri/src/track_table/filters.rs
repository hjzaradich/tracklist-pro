//! The track table's filters (1cA-4; ROADMAP 1.11 "The track table"): the
//! chips above the table, each one a condition on a row of `track_row`. All
//! the filters of a query apply together, except the format filters, which
//! are alternatives (a track is in any of the formats named). Free filters
//! such as "BPM 120 to 126" are 1cB's.
//!
//! "The file" of a row is the file it shows (`track_row.file_id`): the
//! Library track's linked file, else the track's best file. A quality
//! problem is what Review's "Quality problems" group holds, by the SQL text
//! Review itself reads (`review::quality_problem_recordings`), worked out
//! when the query runs: a Library track whose linked file is on disk and
//! flagged by a current verdict, cut off or broken, with no better copy and
//! no "Keep anyway" or "Find a better file" for that audio, or one with a
//! "Better file available" item whose linked file is still there.
//!
//! **Doors from the Overview** (1cC-4, "Every number is a door"): the
//! filters [`TableFilter::AnalyzedByRekordbox`] and
//! [`TableFilter::NotAnalyzedByRekordbox`] are the Overview's own definition,
//! in the very SQL text it counts with (`overview::ANALYZED`), looked up
//! through the row's Library track when the query runs. A number on the
//! Overview and the rows behind its chip can't mean different things, because
//! there is one definition; a test counts both. The crate filters read the
//! flags `track_row` keeps, worked out from the same text
//! (`overview::IN_A_CRATE`), so they agree once the index has caught up. So do
//! the rest, whose definitions cost too much per row to run in a filter:
//! [`TableFilter::MissingTags`] and [`TableFilter::MissingArt`] (1cC-U1b;
//! `overview::gaps_of`), and [`TableFilter::LowQuality`],
//! [`TableFilter::Energy`] and [`TableFilter::NoEnergy`] (1cD-R1a;
//! `overview::door_answers_of`). The index job works each Library track's
//! answer out with the Overview's own function and keeps it in the row's
//! flags, which the Overview counts and these filters list.
//!
//! **The genre door** (1cD-R1b, [`TableFilter::Genre`]) reads `track_row`'s
//! `genre_group`, the lower-cased genre the index job keeps from the
//! Overview's own definition (`overview::door_answers_of`), under the same
//! rule the Overview counts it by (`overview::genre_group`): capitals don't
//! make another genre. The name is a value, bound as a parameter, never part
//! of the SQL, so any text is safe in it.

use rusqlite::types::Value;
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::overview::{library_track_where, ENERGY_LEVELS};
use crate::search::flag;

/// One filter chip.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum TableFilter {
    /// The file is not on disk: a Library track marked missing by the scan,
    /// or a track whose file is gone. (A file on an unplugged drive counts
    /// as on disk, as everywhere.)
    Missing,
    /// rekordbox has the track: one of its files is the trusted match of a
    /// rekordbox entry.
    InRekordbox,
    /// rekordbox doesn't have the track.
    NotInRekordbox,
    /// The file is in this format (`file.sniffed_format`, e.g. `flac`).
    Format { format: String },
    /// The track is one of Review's "Quality problems" (see the module doc).
    QualityProblem,
    /// The track is in at least one crate.
    InCrate,
    /// The track is in no crate.
    InNoCrate,
    /// The track is in the Library.
    InLibrary,
    /// The track is not in the Library.
    NotInLibrary,
    /// The track's linked file has a current quality verdict that flags it
    /// (low bitrate, a suspected transcode, cut off or broken): the Overview's
    /// "tracks made from low-quality files" (`track_row` flag
    /// [`flag::LOW_QUALITY`]). Not the same as
    /// [`TableFilter::QualityProblem`], which is Review's group and leaves
    /// out a track with a better copy or a "Keep anyway".
    LowQuality,
    /// rekordbox supplied a BPM or a key for the track (the Overview's
    /// "analyzed").
    AnalyzedByRekordbox,
    /// rekordbox didn't: a Library track with no `analysis` row from it.
    NotAnalyzedByRekordbox,
    /// The energy the Library track shows is this level, 1 to 10 (the
    /// Overview's energy bars; `track_row` bits [`flag::ENERGY`]).
    Energy { level: u8 },
    /// No source gives the Library track an energy.
    NoEnergy,
    /// The Library track has no title, no artist or no genre (the
    /// Overview's "tracks with missing tags"; `track_row` flag
    /// [`flag::MISSING_TAGS`]).
    MissingTags,
    /// The Library track's linked file has tags and no embedded cover (the
    /// Overview's "tracks with missing art"; `track_row` flag
    /// [`flag::MISSING_ART`]).
    MissingArt,
    /// The Library track's genre is this one, however it is capitalized (the
    /// Overview's genre bars; `track_row.genre_group`). Free text: it is
    /// bound as a parameter. A name that is blank, or that no track has,
    /// lists nothing.
    Genre { name: String },
}

/// The ids of the tracks Review lists as quality problems, as the JSON array
/// [`conditions`] takes. The SQL is Review's own and costs a few tens of
/// milliseconds per thousand flagged files, so a query evaluates it once for
/// its count and its window together.
pub(super) fn quality_problems(conn: &rusqlite::Connection) -> rusqlite::Result<String> {
    let ids: Vec<i64> = conn
        .prepare_cached(&crate::review::quality_problem_recordings())?
        .query_map([], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    Ok(serde_json::to_string(&ids).expect("numbers always serialize"))
}

/// The conditions of `filters`, as SQL on `tr` (a `track_row`), pushing the
/// values they need onto `params` as numbered parameters. Each reads
/// `track_row.flags` or `format`, which every index holds, except the
/// quality problems, which are Review's definition: `quality` is
/// [`quality_problems`] when the caller has worked it out already, else the
/// definition itself is put into the query; and the two "analyzed" doors,
/// which look at the row's Library track.
pub(super) fn conditions(
    filters: &[TableFilter],
    quality: Option<&str>,
    params: &mut Vec<Value>,
) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut formats: Vec<&str> = Vec::new();
    let has = |bit: i64| format!("(tr.flags & {bit}) <> 0");
    let lacks = |bit: i64| format!("(tr.flags & {bit}) = 0");
    // A repeated filter counts once.
    let mut once: Vec<&TableFilter> = Vec::new();
    for filter in filters {
        if !once.contains(&filter) {
            once.push(filter);
        }
    }
    for filter in once {
        let sql = match filter {
            TableFilter::Missing => has(flag::MISSING),
            TableFilter::InRekordbox => has(flag::IN_REKORDBOX),
            TableFilter::NotInRekordbox => lacks(flag::IN_REKORDBOX),
            TableFilter::Format { format } => {
                formats.push(format);
                continue;
            }
            TableFilter::QualityProblem => match quality {
                Some(ids) => {
                    params.push(Value::Text(ids.to_owned()));
                    format!(
                        "tr.recording_id IN (SELECT value FROM json_each(?{}))",
                        params.len()
                    )
                }
                None => format!(
                    "tr.recording_id IN ({})",
                    crate::review::quality_problem_recordings()
                ),
            },
            TableFilter::InCrate => has(flag::IN_CRATE),
            TableFilter::InNoCrate => lacks(flag::IN_CRATE),
            TableFilter::InLibrary => has(flag::IN_LIBRARY),
            TableFilter::NotInLibrary => lacks(flag::IN_LIBRARY),
            TableFilter::LowQuality => has(flag::LOW_QUALITY),
            TableFilter::AnalyzedByRekordbox => library_track_where(crate::overview::ANALYZED),
            TableFilter::NotAnalyzedByRekordbox => {
                library_track_where(&format!("NOT {}", crate::overview::ANALYZED))
            }
            TableFilter::Energy { level } if (1..=ENERGY_LEVELS).contains(&usize::from(*level)) => {
                format!("(tr.flags & {}) = {}", flag::ENERGY, flag::energy(*level))
            }
            // A level that isn't one holds no track. Without this, level 0
            // would read as "no energy" on every row, in the Library or not.
            TableFilter::Energy { .. } => "0".to_owned(),
            TableFilter::NoEnergy => format!(
                "(tr.flags & {}) = {}",
                flag::IN_LIBRARY | flag::ENERGY,
                flag::IN_LIBRARY
            ),
            TableFilter::MissingTags => has(flag::MISSING_TAGS),
            TableFilter::MissingArt => has(flag::MISSING_ART),
            TableFilter::Genre { name } => {
                params.push(Value::Text(crate::overview::genre_group(name)));
                format!("tr.genre_group = ?{}", params.len())
            }
        };
        out.push(sql);
    }
    if !formats.is_empty() {
        let mut marks = Vec::new();
        for format in formats {
            params.push(Value::Text(format.to_owned()));
            marks.push(format!("?{}", params.len()));
        }
        out.push(format!("tr.format IN ({})", marks.join(", ")));
    }
    out
}
