//! The version parser's tests. Every name here is invented.
//!
//! - `corpus`: the table of names and the parse each must give.
//! - `spellings`: every label spelling, in every bracket style.
//! - `comparisons`: the table of name pairs and what they say about each
//!   other.
//! - `rules`: one test per hard rule of ROADMAP §1.5.
//! - `tokens`: the tokenizer by itself.
//! - `properties`: whatever the input, no panic, the original kept, the
//!   same answer twice.

use super::*;

mod comparisons;
mod corpus;
mod properties;
mod rules;
mod spellings;
mod tokens;

/// `T` for a title tag, `F` for a file name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Src {
    T,
    F,
}

pub(super) fn parsed(src: Src, input: &str) -> ParsedName {
    match src {
        Src::T => parse_title(input),
        Src::F => parse_file_name(input),
    }
}

/// Markers as `kind` or `kind=detail`, with `?` after a doubtful one,
/// joined by " | ".
pub(super) fn show_markers(markers: &[Marker]) -> String {
    markers
        .iter()
        .map(|m| {
            let mut out = m.kind.as_str().to_string();
            if let Some(detail) = &m.detail {
                out.push('=');
                out.push_str(detail);
            }
            if m.ambiguous {
                out.push('?');
            }
            out
        })
        .collect::<Vec<_>>()
        .join(" | ")
}

/// Credits joined by " | ", featured ones with a `+` in front.
pub(super) fn show_credits(credits: &[Credit]) -> String {
    credits
        .iter()
        .map(|c| match c.role {
            CreditRole::Main => c.name.clone(),
            CreditRole::Featured => format!("+{}", c.name),
        })
        .collect::<Vec<_>>()
        .join(" | ")
}

pub(super) fn junk_kind(kind: JunkKind) -> &'static str {
    match kind {
        JunkKind::HtmlEntity => "html_entity",
        JunkKind::UrlEscape => "url_escape",
        JunkKind::StoreId => "store_id",
        JunkKind::SiteName => "site_name",
        JunkKind::RipTag => "rip_tag",
        JunkKind::TrackNumber => "track_number",
        JunkKind::Underscores => "underscores",
        JunkKind::FileExtension => "file_extension",
        JunkKind::FolderPath => "folder_path",
        JunkKind::Duration => "duration",
    }
}

/// Junk as `kind:text`, joined by " | ".
pub(super) fn show_junk(junk: &[Junk]) -> String {
    junk.iter()
        .map(|j| format!("{}:{}", junk_kind(j.kind), j.text))
        .collect::<Vec<_>>()
        .join(" | ")
}
