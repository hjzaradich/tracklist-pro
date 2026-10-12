//! What a `Group` job is asked to do, as a typed value.
//!
//! One job kind, [`JobKind::Group`](crate::jobs::JobKind::Group), does three
//! pieces of work, told apart by the job's target (stored as JSON). The
//! target is read and written only through [`GroupTarget`], so nothing
//! compares JSON by hand:
//!
//! | [`GroupTarget`] | Stored target | What the job does | Priority |
//! |---|---|---|---|
//! | `Quick` | none | one grouping pass ([`super::group_job`]) | background |
//! | `Decide` | `{"decide": true}` | the deciding job ([`super::decide_job`]) | matching's |
//! | `Roles` | `{"roles": true}` | ranks the files of the tracks that changed and sets their roles ([`crate::file_roles`]) | below matching's |
//!
//! The three are different jobs for the queue ([`crate::scan::chain::queue_once`]
//! keys on kind and target), so one waiting never stands in for another. A
//! target this build can't read is a quick job, as before.

use serde::Deserialize;

use crate::jobs::Priority;

/// The stored shape of a target: the flags that are set.
#[derive(Deserialize, Default)]
struct Stored {
    #[serde(default)]
    decide: bool,
    #[serde(default)]
    roles: bool,
}

/// Which piece of work a `Group` job does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupTarget {
    /// One grouping pass.
    Quick,
    /// The deciding job.
    Decide,
    /// The best-file roles.
    Roles,
}

impl GroupTarget {
    /// The work a job with this stored `target` does.
    pub fn of(target: Option<&serde_json::Value>) -> GroupTarget {
        // Only an object is a target (serde would also read a list).
        let stored: Stored = match target {
            Some(object @ serde_json::Value::Object(_)) => {
                serde_json::from_value(object.clone()).unwrap_or_default()
            }
            _ => Stored::default(),
        };
        match (stored.roles, stored.decide) {
            (true, _) => GroupTarget::Roles,
            (false, true) => GroupTarget::Decide,
            (false, false) => GroupTarget::Quick,
        }
    }

    /// The target to store for a job of this work.
    pub fn to_json(self) -> Option<serde_json::Value> {
        match self {
            GroupTarget::Quick => None,
            GroupTarget::Decide => Some(serde_json::json!({ "decide": true })),
            GroupTarget::Roles => Some(serde_json::json!({ "roles": true })),
        }
    }

    /// The priority a job of this work is queued at, if it isn't the
    /// chain's background one.
    pub fn priority(self) -> Option<Priority> {
        match self {
            GroupTarget::Quick => None,
            GroupTarget::Decide => Some(super::DECIDE_PRIORITY),
            GroupTarget::Roles => Some(crate::file_roles::PRIORITY),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_target_is_read_back_as_the_work_it_was_made_for() {
        for work in [GroupTarget::Quick, GroupTarget::Decide, GroupTarget::Roles] {
            assert_eq!(GroupTarget::of(work.to_json().as_ref()), work);
        }
    }

    #[test]
    fn a_target_this_build_cannot_read_is_a_quick_job() {
        for odd in [
            serde_json::json!("decide"),
            serde_json::json!({ "decide": "yes" }),
            serde_json::json!({ "roles": 1 }),
            serde_json::json!([true]),
            serde_json::json!({}),
        ] {
            assert_eq!(GroupTarget::of(Some(&odd)), GroupTarget::Quick, "{odd}");
        }
    }
}
