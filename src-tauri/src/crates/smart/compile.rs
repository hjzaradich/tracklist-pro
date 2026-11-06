//! A smart crate's rule as SQL on `tr` (a `track_row`, the track table's
//! index). The SQL text is made only of the fixed strings here: a field
//! names its column, an operator its comparison. Every value the rule holds
//! (a number, a key, a genre, a crate id) is pushed onto `params` and named
//! by its number (`?N`), so no text the user typed is ever part of the SQL.

use rusqlite::types::Value;

use super::{ChoiceOp, Combine, Condition, NumberOp, SmartRule, TextOp, MAX_CONDITIONS};
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
            "tr.rating IS NOT NULL",
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
            let mark = bind(params, Value::Text(crate::overview::genre_group(value)));
            match op {
                TextOp::Is => format!("(tr.genre_group = {mark})"),
                TextOp::IsNot => format!("(tr.genre_group <> {mark})"),
                TextOp::Contains => format!("(instr(tr.genre_group, {mark}) > 0)"),
            }
        }
        Condition::InCrate { op, crate_id } => {
            let mark = bind(params, Value::Integer(crate_id.0));
            let held = format!(
                "EXISTS (SELECT 1 FROM crate_entry ice
                         JOIN library_track ilt ON ilt.id = ice.library_track_id
                         WHERE ice.crate_id = {mark} AND ice.kind = 'member'
                           AND ilt.recording_id = tr.recording_id)"
            );
            match op {
                ChoiceOp::Is => format!("({held})"),
                ChoiceOp::IsNot => format!("(NOT {held})"),
            }
        }
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
