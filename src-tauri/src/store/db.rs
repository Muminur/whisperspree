//! SQLite connection opening + the forward-only migration runner (§11 names
//! `db.rs`; §8.1 migration policy).
//!
//! **Every rusqlite `Connection` the app uses MUST come from [`open`].**
//! `PRAGMA foreign_keys` is a **per-connection** setting that SQLite resets to
//! OFF on every new connection — the in-file `PRAGMA foreign_keys = ON` at the
//! top of `0001_init.sql` only ever runs once, on the connection that performs
//! the migration, and is worthless on every later `open()` call unless `open`
//! re-applies it itself. Any code path that opens a bare `rusqlite::Connection`
//! elsewhere silently disables `dictation_transforms … ON DELETE CASCADE` and
//! `app_rules.custom_prompt_id … ON DELETE SET NULL` with NO error — rows
//! orphan instead of cascading/nulling. `foreign_keys_on_per_connection` and
//! `history::fk_cascade_deletes_transforms` / `app_rules::custom_prompt_delete_sets_null`
//! exist specifically to fail loudly if this invariant is ever broken.
//!
//! PRD refs:
//! - §8.1 — schema (`0001_init.sql`, embedded via `include_str!`), migration
//!   policy: forward-only `NNNN_name.sql` files, "never edit an applied
//!   migration — add a new one", applied inside a transaction.
//! - §4.2 — `rusqlite` 0.31, feature `bundled`.
//! - §4.4 — the DB lives at `<app-data-dir>/whisperspree.db`.
//! - §14 `DB-IO` — every rusqlite failure maps here (OPEN_QUESTIONS Q8).
//! - OPEN_QUESTIONS Q10 — `schema_version` has no PK/single-row constraint;
//!   the runner always reads `MAX(version)`, and the runner (not the migration
//!   file) owns the version bump after each applied file. A DB whose version
//!   exceeds every known migration is left untouched (forward-compatible; a
//!   newer app wrote it) — never downgraded, never an error.

use crate::error::Error;
use rusqlite::Connection;
use std::path::{Path, PathBuf};

/// Every known migration, in ascending version order: `(version, sql)`.
/// New migrations are appended here — never edit an applied one (§8.1).
const MIGRATIONS: &[(i64, &str)] = &[(1, include_str!("../../migrations/0001_init.sql"))];

/// Open (creating if absent) the SQLite database at `path`, apply the
/// per-connection PRAGMAs (`journal_mode=WAL`, `foreign_keys=ON`,
/// `secure_delete=ON` — §12 P-6, security review S4: deleted transcripts must
/// not linger recoverable in freed pages — see the module-level
/// "per-connection trap" doc above), run [`migrate`], lock the file down to
/// `0600` (security review S1: this DB holds raw transcripts, more sensitive
/// than `settings.json`, which T0.3 already restricts), and return the ready
/// connection. Any rusqlite failure maps to `DB-IO` (§14).
pub fn open(path: &Path) -> Result<Connection, Error> {
    let conn =
        Connection::open(path).map_err(|e| super::db_io(&format!("open {}", path.display()), e))?;
    conn.execute_batch(
        "PRAGMA journal_mode = WAL; PRAGMA foreign_keys = ON; PRAGMA secure_delete = ON;",
    )
    .map_err(|e| super::db_io("apply per-connection PRAGMAs", e))?;
    migrate(&conn)?;
    restrict_db_file_permissions(path)?;
    Ok(conn)
}

/// Security review S1: `whisperspree.db` (and its WAL-mode companion files,
/// `-wal`/`-shm`, when present) must be `0600` — owner read/write only, never
/// group/world-readable — since it holds raw transcripts (§12 P-6). No-op on
/// non-unix targets (v1 is macOS-only per §4.1, but this keeps the crate
/// portable for `cargo check` elsewhere).
#[cfg(unix)]
fn restrict_db_file_permissions(path: &Path) -> Result<(), Error> {
    use std::os::unix::fs::PermissionsExt;

    let mode = std::fs::Permissions::from_mode(0o600);
    std::fs::set_permissions(path, mode.clone())
        .map_err(|e| super::db_io("restrict db file permissions", e))?;

    for suffix in ["-wal", "-shm"] {
        let mut companion = path.as_os_str().to_os_string();
        companion.push(suffix);
        let companion = PathBuf::from(companion);
        if companion.exists() {
            std::fs::set_permissions(&companion, mode.clone())
                .map_err(|e| super::db_io("restrict db companion file permissions", e))?;
        }
    }
    Ok(())
}

#[cfg(not(unix))]
fn restrict_db_file_permissions(_path: &Path) -> Result<(), Error> {
    Ok(())
}

/// Apply every `0001_init.sql`-and-later migration whose version is greater
/// than the DB's current `MAX(schema_version.version)` (treated as `0` if the
/// `schema_version` table does not exist yet), in ascending order, each inside
/// its own transaction that also sets `schema_version` to that file's version
/// (OPEN_QUESTIONS Q10 — the runner owns the bump, not the migration file).
/// A version already at or beyond the highest known migration is a no-op
/// (`Ok`, no rewrite, never downgraded).
// PRD-QUESTION(Q10): §8.1 doesn't say how schema_version is bumped for
// migrations beyond 0001 (which self-seeds); the runner owns the bump after
// each applied file, and always reads MAX(version) — never a bare
// `SELECT version`, since the table has no PK/single-row constraint.
pub fn migrate(conn: &Connection) -> Result<(), Error> {
    let table_exists: bool = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'schema_version'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|e| super::db_io("check schema_version table", e))?
        > 0;

    let current: i64 = if table_exists {
        conn.query_row("SELECT MAX(version) FROM schema_version", [], |row| {
            row.get(0)
        })
        .map_err(|e| super::db_io("read MAX(schema_version.version)", e))?
    } else {
        0
    };

    for (version, sql) in MIGRATIONS {
        if *version <= current {
            continue;
        }
        // `PRAGMA journal_mode`/`foreign_keys` cannot run inside a transaction
        // (SQLite rejects `journal_mode` changes mid-transaction) and are
        // already applied per-connection by `open` above — strip any PRAGMA
        // lines from the embedded migration text before wrapping the rest in
        // a transaction. The on-disk `.sql` file itself stays byte-identical
        // to §8.1 (spec fidelity); only the in-memory text executed here is
        // filtered.
        let ddl = strip_pragma_lines(sql)?;
        conn.execute_batch("BEGIN;")
            .map_err(|e| super::db_io("begin migration transaction", e))?;
        let result: Result<(), Error> = (|| {
            conn.execute_batch(&ddl)
                .map_err(|e| super::db_io(&format!("apply migration {version}"), e))?;
            conn.execute("UPDATE schema_version SET version = ?1", [*version])
                .map_err(|e| super::db_io("bump schema_version", e))?;
            Ok(())
        })();
        match result {
            Ok(()) => {
                conn.execute_batch("COMMIT;")
                    .map_err(|e| super::db_io("commit migration transaction", e))?;
            }
            Err(err) => {
                let _ = conn.execute_batch("ROLLBACK;");
                return Err(err);
            }
        }
    }

    Ok(())
}

// PRD-QUESTION(Q12): a migration file must not depend on a PRAGMA taking
// effect, because PRAGMA statements are not executed as part of the
// migration — `journal_mode`/`foreign_keys` cannot run inside the migration
// transaction (SQLite rejects a `journal_mode` change mid-transaction) and
// are already applied per-connection by `open` above, which is strictly
// stronger than a one-time PRAGMA embedded in a migration file. Any OTHER
// PRAGMA a migration author writes (e.g. the standard SQLite table-rebuild
// idiom `PRAGMA legacy_alter_table=ON; … PRAGMA legacy_alter_table=OFF;`, or
// `writable_schema`) is silently unexecuted if merely dropped — that is
// exactly the shape of bug that lets rows get deleted or rejected in
// production with nobody noticing until the data is already gone. So this
// filter REJECTS (loudly, at migration-apply time) any PRAGMA it does not
// already know is safe to drop, instead of silently stripping it.
/// Drop the two sanctioned `PRAGMA ...;` statements (`journal_mode`,
/// `foreign_keys`) from an embedded migration's SQL text — both are already
/// applied per-connection by [`open`] and cannot execute inside the migration
/// transaction. Any other `PRAGMA` is rejected as `DB-IO` (§14) rather than
/// silently dropped. Non-PRAGMA statements (the DDL) pass through untouched.
///
/// Statement-based (splits on `;`), not line-based (security review S5): a
/// line-based check can be bypassed by a PRAGMA sharing a line with other SQL
/// after a semicolon (`CREATE TABLE t(id TEXT); PRAGMA writable_schema=ON;`),
/// which a `starts_with`-on-trimmed-line check never sees.
fn strip_pragma_lines(sql: &str) -> Result<String, Error> {
    let mut out = String::new();
    for stmt in sql.split(';') {
        let trimmed = stmt.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.to_ascii_uppercase().starts_with("PRAGMA") {
            match pragma_name(trimmed).as_deref() {
                Some("journal_mode") | Some("foreign_keys") => continue,
                other => {
                    return Err(super::db_io(
                        "migration PRAGMA filter",
                        format!(
                            "migration text uses PRAGMA {:?}, which is not applied \
                             per-connection by db::open and cannot execute inside the \
                             migration transaction — a migration must not depend on a \
                             PRAGMA taking effect (OPEN_QUESTIONS Q12)",
                            other.unwrap_or("<unnamed>")
                        ),
                    ));
                }
            }
        }
        out.push_str(trimmed);
        out.push_str(";\n");
    }
    Ok(out)
}

/// Extract the pragma name from a line already known to start with `PRAGMA`
/// (case-insensitive), e.g. `"pragma   Foreign_Keys = off"` -> `"foreign_keys"`.
/// Matching is case- and whitespace-insensitive; the name is lowercased for
/// comparison.
fn pragma_name(trimmed_line: &str) -> Option<String> {
    let after_keyword = trimmed_line.get(6..)?; // len("PRAGMA") == 6 bytes (ASCII)
    let name: String = after_keyword
        .trim_start()
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect();
    if name.is_empty() {
        None
    } else {
        Some(name.to_ascii_lowercase())
    }
}

// ---------------------------------------------------------------------------
// Tests (colocated so `cargo test store::` / `cargo test db::` filters here).
// Real SQLite in tempdirs throughout — no mocks (CLAUDE.md §3).
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn db_path(dir: &Path) -> std::path::PathBuf {
        dir.join("whisperspree.db")
    }

    /// AC: §8.1 schema + migration policy (plan algorithm steps 1-3) — opening
    /// a fresh, never-before-seen path creates every one of the 7 §8.1 tables
    /// (plus `schema_version`) and leaves `MAX(schema_version.version) == 1`.
    #[test]
    fn migrate_applies_and_sets_version_1() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let conn = open(&db_path(tmp.path())).expect("open+migrate a fresh DB");

        let mut stmt = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
            .expect("prepare table listing");
        let tables: Vec<String> = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .expect("query tables")
            .map(|r| r.expect("row"))
            .collect();

        for expected in [
            "app_rules",
            "custom_prompts",
            "dictation_transforms",
            "dictations",
            "dictionary_entries",
            "schema_version",
            "snippets",
        ] {
            assert!(
                tables.iter().any(|t| t == expected),
                "expected table {expected:?} after migration; got {tables:?}"
            );
        }

        let version: i64 = conn
            .query_row("SELECT MAX(version) FROM schema_version", [], |r| r.get(0))
            .expect("read schema_version");
        assert_eq!(version, 1, "0001_init.sql must leave schema_version at 1");
    }

    /// AC: plan algorithm step 4 — re-running `migrate` on an already
    /// up-to-date DB is a no-op: no error, version unchanged.
    #[test]
    fn migrate_is_idempotent() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = db_path(tmp.path());
        let conn = open(&path).expect("first open (runs migrate once)");

        migrate(&conn).expect("a second migrate() call on an up-to-date DB must not error");

        let version: i64 = conn
            .query_row("SELECT MAX(version) FROM schema_version", [], |r| r.get(0))
            .expect("read schema_version");
        assert_eq!(version, 1, "idempotent migrate must not change the version");
    }

    /// AC: plan algorithm step 5 / OPEN_QUESTIONS Q10 — a DB whose
    /// `schema_version` already exceeds every migration this build knows about
    /// is left completely untouched: `Ok`, and the version is never
    /// downgraded (forward-compatible with a newer app instance).
    #[test]
    fn migrate_future_version_left_untouched() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = db_path(tmp.path());
        let conn = open(&path).expect("open+migrate a fresh DB");

        conn.execute("UPDATE schema_version SET version = 999", [])
            .expect("seed a future schema_version");

        migrate(&conn).expect("a future schema_version must not error");

        let version: i64 = conn
            .query_row("SELECT MAX(version) FROM schema_version", [], |r| r.get(0))
            .expect("read schema_version");
        assert_eq!(
            version, 999,
            "a schema_version ahead of every known migration must never be downgraded"
        );
    }

    /// AC: module-level "per-connection trap" doc / plan "PRAGMA placement" —
    /// `db::open` re-applies `PRAGMA foreign_keys=ON` on EVERY connection it
    /// hands back (SQLite resets this PRAGMA to OFF by default on each new
    /// connection; the in-file PRAGMA in `0001_init.sql` only ever runs once).
    #[test]
    fn foreign_keys_on_per_connection() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let conn = open(&db_path(tmp.path())).expect("open");

        let fk_on: i64 = conn
            .query_row("PRAGMA foreign_keys", [], |r| r.get(0))
            .expect("read PRAGMA foreign_keys");
        assert_eq!(
            fk_on, 1,
            "db::open must leave PRAGMA foreign_keys ON for every connection"
        );
    }

    /// AC: §14 `DB-IO` — a rusqlite failure (opening a path that can never be
    /// a SQLite database file: an existing directory) maps to `DB-IO`, not a
    /// panic or an unmapped error.
    #[test]
    fn sqlite_failure_maps_db_io() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let dir_as_db = tmp.path().join("this_is_a_directory_not_a_db_file");
        std::fs::create_dir(&dir_as_db).expect("create blocking directory");

        let err = open(&dir_as_db).expect_err("opening a directory as a DB file must fail");
        assert_eq!(
            err.code(),
            "DB-IO",
            "sqlite open failure must map to DB-IO (§14)"
        );
    }

    /// AC: OPEN_QUESTIONS Q12 — `strip_pragma_lines` (the migration-time
    /// PRAGMA filter, needed because `journal_mode`/`foreign_keys` can't run
    /// inside the migration transaction) must REJECT any PRAGMA it does not
    /// already know is safe to drop (`journal_mode`, `foreign_keys`) rather
    /// than silently stripping it. A future migration using the standard
    /// SQLite table-rebuild idiom (`PRAGMA legacy_alter_table=ON; … PRAGMA
    /// legacy_alter_table=OFF;`, or `writable_schema`, or any other PRAGMA the
    /// app doesn't already apply per-connection) must fail loudly at
    /// migration-apply time — not silently run with that PRAGMA missing,
    /// which is exactly the shape of bug that deletes/rejects rows in
    /// production with nobody noticing until the data is already gone.
    ///
    /// This pins `strip_pragma_lines` to the signature it MUST have —
    /// `Result<String, Error>`, mapping the rejection to `DB-IO` (§14) — not
    /// the current infallible `String`. Intentionally red at compile time
    /// until the implementer reshapes the function to match (CLAUDE.md §3:
    /// tests-first; write the interface the way it should work).
    #[test]
    fn migration_with_unsupported_pragma_is_rejected() {
        let unsupported = "PRAGMA legacy_alter_table=ON;\n\
                            CREATE TABLE t (id INTEGER);\n\
                            PRAGMA legacy_alter_table=OFF;";

        let err = strip_pragma_lines(unsupported).expect_err(
            "a PRAGMA other than journal_mode/foreign_keys must be rejected \
             at migration-apply time, not silently stripped (OPEN_QUESTIONS Q12)",
        );
        assert_eq!(
            err.code(),
            "DB-IO",
            "an unrecognized PRAGMA in migration text must map to DB-IO (§14)"
        );
    }

    /// AC: guards the fix above against over-strictness — the two PRAGMAs
    /// `0001_init.sql` actually contains (`journal_mode`, `foreign_keys`) must
    /// still pass the filter without error, so real migrations keep applying.
    #[test]
    fn migration_pragma_journal_mode_and_foreign_keys_are_accepted() {
        let sanctioned = "PRAGMA journal_mode = WAL;\n\
                           PRAGMA foreign_keys = ON;\n\
                           CREATE TABLE t (id INTEGER);";

        let ddl = strip_pragma_lines(sanctioned).expect(
            "the two sanctioned PRAGMAs (journal_mode, foreign_keys) must still be accepted, \
             not rejected — 0001_init.sql relies on this",
        );
        assert!(
            !ddl.to_ascii_uppercase().contains("PRAGMA"),
            "sanctioned PRAGMA lines must still be stripped from the executed text: {ddl}"
        );
        assert!(
            ddl.contains("CREATE TABLE t"),
            "non-PRAGMA DDL must survive the filter untouched: {ddl}"
        );
    }

    /// AC: security review S5 — the PRAGMA guard (OPEN_QUESTIONS Q12) checks
    /// whether a TRIMMED LINE starts with `PRAGMA`, so a pragma sharing a
    /// line with other SQL after a `;` slips past the check unrecognized and
    /// would execute, defeating the Q12 guarantee entirely.
    #[test]
    fn migration_with_pragma_sharing_a_line_is_rejected() {
        let sneaky = "CREATE TABLE t(id TEXT); PRAGMA writable_schema=ON;";

        let err = strip_pragma_lines(sneaky).expect_err(
            "a PRAGMA sharing a line with other SQL must still be rejected — the line-based \
             guard must not be bypassable by putting a PRAGMA after a semicolon on the same \
             line as other DDL",
        );
        assert_eq!(
            err.code(),
            "DB-IO",
            "an unrecognized PRAGMA hidden mid-line must map to DB-IO (§14), same as one on its \
             own line"
        );
    }

    /// AC: security review S1 — `whisperspree.db` holds raw transcripts
    /// (more sensitive than `settings.json`, which T0.3 already writes
    /// 0600) and must not be created world-readable.
    #[cfg(unix)]
    #[test]
    fn db_file_permissions_are_0600() {
        use std::os::unix::fs::PermissionsExt;

        let tmp = tempfile::tempdir().expect("tempdir");
        let path = db_path(tmp.path());
        let _conn = open(&path).expect("open");

        let meta = std::fs::metadata(&path).expect("read db file metadata");
        assert_eq!(
            meta.permissions().mode() & 0o777,
            0o600,
            "whisperspree.db must be created 0600, matching the settings.json precedent (T0.3)"
        );
    }

    /// AC: security review S4 — deleted transcripts (§12 P-6: "delete is
    /// immediate") must not remain forensically recoverable from the SQLite
    /// file's freed pages. `PRAGMA secure_delete` must be enabled on every
    /// connection `db::open` hands back.
    #[test]
    fn secure_delete_pragma_is_enabled() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let conn = open(&db_path(tmp.path())).expect("open");

        let secure_delete: i64 = conn
            .query_row("PRAGMA secure_delete", [], |r| r.get(0))
            .expect("read PRAGMA secure_delete");
        assert_eq!(
            secure_delete, 1,
            "PRAGMA secure_delete must be enabled so deleted transcripts (§12 P-6) are not left \
             forensically recoverable in freed SQLite pages"
        );
    }
}
