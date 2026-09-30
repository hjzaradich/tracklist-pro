# Database migrations

Each file here changes the database schema one step. The app embeds every
file in its binary, and the writer applies the ones a database hasn't had
yet, in number order, when it opens the database at startup
(`src/db/migrations/mod.rs`).

## Rules

- **Append-only.** Once a migration is on `main`, never edit, rename or
  delete it. To change the schema again, add a new migration. CI enforces
  this: `checksums.txt` records every migration's SHA-256, and a test fails
  if a recorded migration is missing or its content changed. The app also
  refuses to open a database whose recorded migrations don't match the ones
  it was built with.
- **Numbering.** Files are named `NNNN_short_name.sql`: four digits, then
  lowercase words joined by `_`. Numbers start at `0001` and have no gaps
  or repeats. Take the **next free number at merge time**: if `main` took
  your number while your branch was open, renumber yours before merging.
  Never renumber a migration that's already on `main`.
- **Resolving a conflict** when two branches both added the same number:
  keep both migrations. The one already on `main` keeps its number; renumber
  the later one to the next free number (rename the file, its `MIGRATIONS`
  entry and its name in `checksums.txt`). The checksum covers only the SQL,
  so its hash stays the same.
- **Adding one** takes three edits, all in this folder and the runner:
  1. Add the `.sql` file.
  2. Add it to `MIGRATIONS` in `src/db/migrations/mod.rs`.
  3. Add its line to `checksums.txt`. The failing test prints the exact
     line to add.

  Until your branch is merged the migration isn't applied anywhere that
  matters, so you may still edit it; update its checksum line to match.
  If a local dev database already ran the draft, delete that database.
- **One transaction each.** The runner wraps every migration in its own
  transaction and records it in the `schema_migration` table in the same
  transaction. If any statement fails, the whole migration is rolled back
  and the database stays at the previous version. So a migration file must
  not contain `BEGIN`, `COMMIT`, `ROLLBACK`, `END`, `SAVEPOINT` or `RELEASE`
  of its own, nor statements SQLite can't run inside a transaction
  (`VACUUM`, `PRAGMA journal_mode`).
- **Foreign keys** are switched off while migrations run, so a table can be
  rebuilt (create new, copy, drop old, rename) without cascades firing.
  Before each migration commits, the runner runs `PRAGMA foreign_key_check`
  and rolls back if the migration left any reference broken.
