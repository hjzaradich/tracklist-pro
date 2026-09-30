//! The append-only guard (ROADMAP §5.6): once a migration is on `main`,
//! CI fails if it is edited, renamed or removed.
//!
//! `src-tauri/migrations/checksums.txt` records every migration's name and
//! SHA-256. Adding a migration means adding a line, which is fine; any
//! change to a recorded migration fails [`the_committed_checksums_match_the_migrations`].

use std::collections::BTreeMap;
use std::fmt;

use super::{Migration, MIGRATIONS};

const CHECKSUMS: &str = include_str!("../../../migrations/checksums.txt");

/// One way the migrations disagree with the recorded checksums.
#[derive(Debug, PartialEq, Eq)]
enum Violation {
    /// A recorded migration is gone: it was removed or renamed.
    Missing { name: String },
    /// A recorded migration's content changed.
    Edited { name: String },
    /// A migration has no line in the list yet. `line` is the one to add.
    Unrecorded { line: String },
    /// A line in the list isn't `<name> <sha256>`.
    Malformed { line: String },
    /// A name is recorded twice.
    Duplicate { name: String },
}

impl fmt::Display for Violation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Violation::Missing { name } => write!(
                f,
                "{name} is recorded but gone. Migrations on main are never renamed or removed; restore it."
            ),
            Violation::Edited { name } => write!(
                f,
                "{name} changed after it was recorded. Migrations on main are never edited; restore it and add a new migration instead."
            ),
            Violation::Unrecorded { line } => write!(
                f,
                "a migration isn't recorded. Add this line to src-tauri/migrations/checksums.txt:\n    {line}"
            ),
            Violation::Malformed { line } => {
                write!(f, "checksums.txt line isn't `<name> <sha256>`: {line:?}")
            }
            Violation::Duplicate { name } => write!(f, "{name} is recorded twice"),
        }
    }
}

/// The line recording `m` in `checksums.txt`.
fn line_for(m: &Migration) -> String {
    format!("{} {}", m.name, m.checksum())
}

/// Compares the checksum list against the migrations. Empty means every
/// recorded migration is present and unchanged, and every migration is
/// recorded. Blank lines and `#` comments in the list are ignored.
fn check(checksums: &str, migrations: &[Migration]) -> Vec<Violation> {
    let mut violations = Vec::new();
    let mut recorded = BTreeMap::new();
    for line in checksums.lines().map(str::trim) {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let fields: Vec<&str> = line.split_whitespace().collect();
        let [name, sum] = fields[..] else {
            violations.push(Violation::Malformed { line: line.into() });
            continue;
        };
        if recorded.insert(name, sum).is_some() {
            violations.push(Violation::Duplicate { name: name.into() });
        }
    }
    for (&name, &sum) in &recorded {
        match migrations.iter().find(|m| m.name == name) {
            None => violations.push(Violation::Missing { name: name.into() }),
            Some(m) if m.checksum() != sum => {
                violations.push(Violation::Edited { name: name.into() })
            }
            Some(_) => {}
        }
    }
    for m in migrations {
        if !recorded.contains_key(m.name) {
            violations.push(Violation::Unrecorded { line: line_for(m) });
        }
    }
    violations
}

#[test]
fn the_committed_checksums_match_the_migrations() {
    let violations = check(CHECKSUMS, MIGRATIONS);
    let report: Vec<String> = violations.iter().map(ToString::to_string).collect();
    assert!(
        violations.is_empty(),
        "migrations are append-only (src-tauri/migrations/README.md):\n{}",
        report.join("\n")
    );
}

mod tests {
    use super::*;

    const FIRST: Migration = Migration {
        name: "0001_init.sql",
        sql: "-- empty\n",
    };
    const SECOND: Migration = Migration {
        name: "0002_add_t.sql",
        sql: "CREATE TABLE t (x INTEGER);\n",
    };

    /// The list as it would be committed on `main` with just `FIRST`.
    fn list_with_first() -> String {
        format!("# comment\n\n{}\n", line_for(&FIRST))
    }

    #[test]
    fn unchanged_migrations_pass() {
        assert_eq!(check(&list_with_first(), &[FIRST]), vec![]);
    }

    #[test]
    fn adding_a_new_migration_with_its_line_passes() {
        let list = format!("{}{}\n", list_with_first(), line_for(&SECOND));
        assert_eq!(check(&list, &[FIRST, SECOND]), vec![]);
    }

    #[test]
    fn adding_a_new_migration_without_its_line_fails_and_says_which_line_to_add() {
        let violations = check(&list_with_first(), &[FIRST, SECOND]);
        assert_eq!(
            violations,
            vec![Violation::Unrecorded {
                line: line_for(&SECOND)
            }]
        );
        assert!(violations[0].to_string().contains(&line_for(&SECOND)));
    }

    #[test]
    fn editing_a_recorded_migration_fails() {
        let edited = Migration {
            sql: "-- empty, but now with a different comment\n",
            ..FIRST
        };
        assert_eq!(
            check(&list_with_first(), &[edited]),
            vec![Violation::Edited {
                name: FIRST.name.into()
            }]
        );
    }

    #[test]
    fn renaming_a_recorded_migration_fails() {
        let renamed = Migration {
            name: "0001_start.sql",
            ..FIRST
        };
        assert_eq!(
            check(&list_with_first(), &[renamed]),
            vec![
                Violation::Missing {
                    name: FIRST.name.into()
                },
                Violation::Unrecorded {
                    line: line_for(&renamed)
                },
            ]
        );
    }

    #[test]
    fn removing_a_recorded_migration_fails() {
        assert_eq!(
            check(&list_with_first(), &[]),
            vec![Violation::Missing {
                name: FIRST.name.into()
            }]
        );
    }

    #[test]
    fn a_malformed_or_duplicated_line_fails() {
        let list = format!("{}{}\n0002_x.sql\n", list_with_first(), line_for(&FIRST));
        assert_eq!(
            check(&list, &[FIRST]),
            vec![
                Violation::Duplicate {
                    name: FIRST.name.into()
                },
                Violation::Malformed {
                    line: "0002_x.sql".into()
                },
            ]
        );
    }

    #[test]
    fn a_crlf_checkout_of_the_list_and_migrations_still_passes() {
        let list = list_with_first().replace('\n', "\r\n");
        let crlf = Migration {
            sql: "-- empty\r\n",
            ..FIRST
        };
        assert_eq!(check(&list, &[crlf]), vec![]);
    }
}
