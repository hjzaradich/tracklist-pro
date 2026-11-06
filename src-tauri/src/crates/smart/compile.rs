//! A smart crate's rule as SQL on `tr` (a `track_row`, the track table's
//! index). The SQL text is made only of the fixed strings here: a field
//! names its column, an operator its comparison. Every value the rule holds
//! (a number, a key, a genre, a crate id) is pushed onto `params` and named
//! by its number (`?N`), so no text the user typed is ever part of the SQL.

use rusqlite::types::Value;

use super::{ChoiceOp, Combine, Condition, NumberOp, SmartRule, TagOp, TextOp, MAX_CONDITIONS};
use crate::crates::fit::Camelot;
use crate::search::flag;

/// SQL that holds for the tracks `rule` matches, pushing its values onto
/// `params`. A rule with no condition matches nothing.
pub fn compile(rule: &SmartRule, params: &mut Vec<Value>) -> String {
    let parts: Vec<String> = rule
        .conditions
        .iter()
        .take(MAX_CONDITIONS)
        .map(|c| condition(c, params))
        .collect();
    if parts.is_empty() {
        return "0".to_owned();
    }
    let joiner = match rule.combine {
        Combine::All => " AND ",
        Combine::Any => " OR ",
    };
    format!("({})", parts.join(joiner))
}

/// Pushes `value` and answers its placeholder.
fn bind(params: &mut Vec<Value>, value: Value) -> String {
    params.push(value);
    format!("?{}", params.len())
}

/// One condition's SQL.
pub(super) fn condition(condition: &Condition, params: &mut Vec<Value>) -> String {
    match condition {
        Condition::Energy { op, value, to } => {
            let bits = format!("(tr.flags & {})", flag::ENERGY);
            let level = |l: u8| Value::Integer(flag::energy(l));
            numeric(
                &bits,
                &format!("{bits} <> 0"),
                *op,
                level(*value),
                to.map(level),
                params,
            )
        }
        Condition::Rating { op, value, to } => numeric(
            "tr.rating",
            // 0 stars is "no rating" (rekordbox's unrated), like a missing one.
            "tr.rating > 0",
            *op,
            Value::Integer(i64::from(*value)),
            to.map(|t| Value::Integer(i64::from(t))),
            params,
        ),
        Condition::Bpm {
            op,
            value,
            to,
            half_double,
        } => bpm(*op, *value, *to, *half_double, params),
        Condition::Key { op, value } => match Camelot::parse(value) {
            Some(key) => {
                let order = i64::from(key.number) * 2 + i64::from(key.major);
                let mark = bind(params, Value::Integer(order));
                match op {
                    ChoiceOp::Is => format!("(tr.key_order = {mark})"),
                    ChoiceOp::IsNot => format!("(tr.key_order <> {mark})"),
                }
            }
            // Not a key: nothing has it, and nothing is compared with it.
            None => "0".to_owned(),
        },
        Condition::Genre { op, value } => {
            // Nothing typed yet matches nothing (`instr(x, '')` is 1 for every
            // track that has a genre).
            if *op == TextOp::Contains && value.trim().is_empty() {
                return "0".to_owned();
            }
            let mark = bind(params, Value::Text(crate::overview::genre_group(value)));
            match op {
                TextOp::Is => format!("(tr.genre_group = {mark})"),
                TextOp::IsNot => format!("(tr.genre_group <> {mark})"),
                TextOp::Contains => format!("(instr(tr.genre_group, {mark}) > 0)"),
            }
        }
        Condition::InCrate { op, crate_id } => {
            let mark = bind(params, Value::Integer(crate_id.0));
            // Uncorrelated, so SQLite builds the crate's recordings once.
            let held = format!(
                "(SELECT ilt.recording_id FROM crate_entry ice
                  JOIN library_track ilt ON ilt.id = ice.library_track_id
                  WHERE ice.crate_id = {mark} AND ice.kind = 'member')"
            );
            match op {
                ChoiceOp::Is => format!("(tr.recording_id IN {held})"),
                ChoiceOp::IsNot => format!("(tr.recording_id NOT IN {held})"),
            }
        }
        Condition::Tag { op, tag_id } => {
            let mark = bind(params, Value::Integer(tag_id.0));
            has_tag(
                *op,
                &format!("SELECT 1 FROM tag t WHERE t.id = {mark}"),
                &format!(
                    "SELECT 1 FROM tag_applied ta WHERE ta.recording_id = tr.recording_id
                     AND ta.tag_id = {mark}"
                ),
            )
        }
        Condition::TagGroup { op, group_id } => {
            let mark = bind(params, Value::Integer(group_id.0));
            has_tag(
                *op,
                &format!("SELECT 1 FROM tag_group g WHERE g.id = {mark}"),
                &format!(
                    "SELECT 1 FROM tag_applied ta JOIN tag t ON t.id = ta.tag_id
                     WHERE ta.recording_id = tr.recording_id AND t.group_id = {mark}"
                ),
            )
        }
    }
}

/// A tag condition. `exists` finds the tag (or group) itself and `applied`
/// the track's tags among it: a tag or group that is gone matches no track,
/// "doesn't have" included. A group that exists but holds no tags is "has
/// none" for every track and "has any" for none.
fn has_tag(op: TagOp, exists: &str, applied: &str) -> String {
    match op {
        TagOp::Has => format!("EXISTS ({applied})"),
        TagOp::HasNot => format!("(EXISTS ({exists}) AND NOT EXISTS ({applied}))"),
    }
}

/// A comparison of the column `expr`, for a track whose `present` holds.
fn numeric(
    expr: &str,
    present: &str,
    op: NumberOp,
    value: Value,
    to: Option<Value>,
    params: &mut Vec<Value>,
) -> String {
    let compare = |sign: &str, params: &mut Vec<Value>| {
        let mark = bind(params, value.clone());
        format!("({present} AND {expr} {sign} {mark})")
    };
    match op {
        NumberOp::Is => compare("=", params),
        NumberOp::IsNot => compare("<>", params),
        NumberOp::AtLeast => compare(">=", params),
        NumberOp::AtMost => compare("<=", params),
        NumberOp::Between => {
            let to = to.unwrap_or_else(|| value.clone());
            let low = bind(params, value.clone());
            let high = bind(params, to);
            format!("({present} AND {expr} BETWEEN min({low}, {high}) AND max({low}, {high}))")
        }
    }
}

/// The BPM condition. A whole BPM `v` stands for the BPMs that round to it,
/// `v - 0.5` up to (not including) `v + 0.5`. With `half_double`, the same
/// range at half and at double the BPM matches too.
fn bpm(
    op: NumberOp,
    value: u16,
    to: Option<u16>,
    half_double: bool,
    params: &mut Vec<Value>,
) -> String {
    let v = f64::from(value);
    let (low, high) = match op {
        NumberOp::Is | NumberOp::IsNot => (Some(v - 0.5), Some(v + 0.5)),
        NumberOp::AtLeast => (Some(v - 0.5), None),
        NumberOp::AtMost => (None, Some(v + 0.5)),
        NumberOp::Between => {
            let w = f64::from(to.unwrap_or(value));
            (Some(v.min(w) - 0.5), Some(v.max(w) + 0.5))
        }
    };
    let scales: &[f64] = if half_double {
        &[1.0, 0.5, 2.0]
    } else {
        &[1.0]
    };
    let ranges: Vec<String> = scales
        .iter()
        .map(|scale| {
            let mut parts = Vec::new();
            if let Some(low) = low {
                parts.push(format!(
                    "tr.bpm >= {}",
                    bind(params, Value::Real(low * scale))
                ));
            }
            if let Some(high) = high {
                parts.push(format!(
                    "tr.bpm < {}",
                    bind(params, Value::Real(high * scale))
                ));
            }
            format!("({})", parts.join(" AND "))
        })
        .collect();
    let any = ranges.join(" OR ");
    match op {
        NumberOp::IsNot => format!("(tr.bpm > 0 AND NOT ({any}))"),
        _ => format!("(tr.bpm > 0 AND ({any}))"),
    }
}
