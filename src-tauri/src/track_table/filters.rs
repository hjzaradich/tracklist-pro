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

use rusqlite::types::Value;
use serde::{Deserialize, Serialize};
use specta::Type;

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
}

/// The conditions of `filters`, as SQL on `tr` (a `track_row`), pushing the
/// values they need onto `params` as numbered parameters. Each reads
/// `track_row.flags` or `format`, which every index holds.
pub(super) fn conditions(filters: &[TableFilter], params: &mut Vec<Value>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut formats: Vec<&str> = Vec::new();
    let has = |bit: i64| format!("(tr.flags & {bit}) <> 0");
    let lacks = |bit: i64| format!("(tr.flags & {bit}) = 0");
    for filter in filters {
        let sql = match filter {
            TableFilter::Missing => has(flag::MISSING),
            TableFilter::InRekordbox => has(flag::IN_REKORDBOX),
            TableFilter::NotInRekordbox => lacks(flag::IN_REKORDBOX),
            TableFilter::Format { format } => {
                if !formats.contains(&format.as_str()) {
                    formats.push(format);
                }
                continue;
            }
            TableFilter::QualityProblem => format!(
                "tr.recording_id IN ({})",
                crate::review::quality_problem_recordings()
            ),
            TableFilter::InCrate => has(flag::IN_CRATE),
            TableFilter::InNoCrate => lacks(flag::IN_CRATE),
            TableFilter::InLibrary => has(flag::IN_LIBRARY),
            TableFilter::NotInLibrary => lacks(flag::IN_LIBRARY),
        };
        if !out.contains(&sql) {
            out.push(sql);
        }
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
