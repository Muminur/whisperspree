//! `snippets` CRUD store (FR-3.2 voice snippet macros; the whole-utterance
//! matching logic itself lives in the future top-level `snippets.rs`, T5.2 —
//! this is the persistence layer only, mirroring OPEN_QUESTIONS Q9's split for
//! `dictionary_entries`).
//!
//! PRD refs:
//! - §8.1 — `snippets` schema: `trigger TEXT NOT NULL UNIQUE` — "stored
//!   lowercase" (prose comment, no SQL CHECK: normalization is app-layer).
//! - §9.1 — `list/add/update/delete_snippet`; "trigger uniqueness enforced".
//! - §14 `DB-IO` — a UNIQUE(`trigger`) violation, or any other rusqlite
//!   failure, maps here.
//! - Plan edge cases — `insert_snippet` lowercases `trigger` before binding
//!   (no DB guard exists for this; documented here and in the plan risks).
//!
//! Every `&Connection` passed here MUST come from [`super::db::open`].

use crate::error::Error;
use rusqlite::Connection;

/// A `snippets` row (§8.1 column-for-column).
#[derive(Debug, Clone, PartialEq)]
pub struct SnippetRow {
    pub id: String,
    pub trigger: String,
    pub content: String,
    pub created_at: String,
    pub usage_count: i64,
}

/// §9.1 `list_snippet`.
pub fn list_snippets(conn: &Connection) -> Result<Vec<SnippetRow>, Error> {
    let mut stmt = conn
        .prepare("SELECT id, trigger, content, created_at, usage_count FROM snippets")
        .map_err(|e| super::db_io("list_snippets prepare", e))?;
    let rows = stmt
        .query_map([], |row| {
            Ok(SnippetRow {
                id: row.get(0)?,
                trigger: row.get(1)?,
                content: row.get(2)?,
                created_at: row.get(3)?,
                usage_count: row.get(4)?,
            })
        })
        .map_err(|e| super::db_io("list_snippets query", e))?;

    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(|e| super::db_io("list_snippets row", e))?);
    }
    Ok(out)
}

/// §9.1 `add_snippet`. `row.trigger` is lowercased before binding (§8.1
/// "stored lowercase" — app-layer normalization, no SQL guard). A duplicate
/// (post-lowercasing) trigger maps to `DB-IO`.
pub fn insert_snippet(conn: &Connection, row: &SnippetRow) -> Result<(), Error> {
    let trigger = row.trigger.to_lowercase();
    conn.execute(
        "INSERT INTO snippets (id, trigger, content, created_at, usage_count)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        rusqlite::params![
            row.id,
            trigger,
            row.content,
            row.created_at,
            row.usage_count
        ],
    )
    .map_err(|e| super::db_io("insert_snippet", e))?;
    Ok(())
}

/// §9.1 `update_snippet`. `row.trigger` is lowercased before binding, same as
/// `insert_snippet` (§8.1 "stored lowercase" — no SQL CHECK enforces it).
pub fn update_snippet(conn: &Connection, row: &SnippetRow) -> Result<(), Error> {
    let trigger = row.trigger.to_lowercase();
    conn.execute(
        "UPDATE snippets SET trigger = ?2, content = ?3, created_at = ?4, usage_count = ?5 \
         WHERE id = ?1",
        rusqlite::params![
            row.id,
            trigger,
            row.content,
            row.created_at,
            row.usage_count
        ],
    )
    .map_err(|e| super::db_io("update_snippet", e))?;
    Ok(())
}

/// §9.1 `delete_snippet`. Returns whether a row was actually deleted.
pub fn delete_snippet(conn: &Connection, id: &str) -> Result<bool, Error> {
    let changed = conn
        .execute("DELETE FROM snippets WHERE id = ?1", [id])
        .map_err(|e| super::db_io("delete_snippet", e))?;
    Ok(changed > 0)
}

// ---------------------------------------------------------------------------
// Tests (colocated so `cargo test store::` / `cargo test snippets::` filters
// here). Real SQLite in tempdirs, opened via `super::db::open`.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn open_test_db(tmp: &std::path::Path) -> Connection {
        super::super::db::open(&tmp.join("whisperspree.db")).expect("db::open a fresh test DB")
    }

    fn sample(id: &str, trigger: &str) -> SnippetRow {
        SnippetRow {
            id: id.to_string(),
            trigger: trigger.to_string(),
            content: "Best regards,\nAlice".to_string(),
            created_at: "2026-01-01T00:00:00Z".to_string(),
            usage_count: 0,
        }
    }

    /// AC: §9.1 snippet CRUD — insert, list, update, delete round-trip
    /// through the real table.
    #[test]
    fn snippets_insert_get_update_delete_roundtrip() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let conn = open_test_db(tmp.path());

        let row = sample("snip-1", "sign off");
        insert_snippet(&conn, &row).expect("insert_snippet");

        let listed = list_snippets(&conn).expect("list_snippets");
        assert_eq!(listed, vec![row.clone()]);

        let mut updated = row.clone();
        updated.content = "Warm regards,\nAlice".to_string();
        updated.usage_count = 5;
        update_snippet(&conn, &updated).expect("update_snippet");

        assert_eq!(
            list_snippets(&conn).expect("list after update"),
            vec![updated]
        );

        let deleted = delete_snippet(&conn, "snip-1").expect("delete_snippet");
        assert!(deleted);
        assert!(list_snippets(&conn).expect("list after delete").is_empty());

        let deleted_again =
            delete_snippet(&conn, "snip-1").expect("delete on a missing id must not error");
        assert!(!deleted_again);
    }

    /// AC: §8.1 `trigger TEXT NOT NULL UNIQUE` / §14 `DB-IO` — inserting a
    /// second snippet with a duplicate trigger maps to `DB-IO`.
    #[test]
    fn trigger_unique_violation_maps_db_io() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let conn = open_test_db(tmp.path());

        insert_snippet(&conn, &sample("snip-a", "sign off")).expect("first insert must succeed");

        let err = insert_snippet(&conn, &sample("snip-b", "sign off"))
            .expect_err("a duplicate trigger must violate the UNIQUE constraint");
        assert_eq!(
            err.code(),
            "DB-IO",
            "UNIQUE(trigger) violation must map to DB-IO (§14)"
        );
    }

    /// AC: §8.1 "stored lowercase" (plan edge case) — `insert_snippet`
    /// lowercases `trigger` before binding, regardless of the case the caller
    /// supplied.
    #[test]
    fn insert_normalizes_trigger_lowercase() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let conn = open_test_db(tmp.path());

        insert_snippet(&conn, &sample("snip-mixed", "New Paragraph"))
            .expect("insert_snippet with mixed-case trigger");

        let listed = list_snippets(&conn).expect("list_snippets");
        assert_eq!(listed.len(), 1);
        assert_eq!(
            listed[0].trigger, "new paragraph",
            "insert_snippet must lowercase the trigger before persisting (§8.1)"
        );
    }

    /// AC: §8.1 "stored lowercase" (code-review SHOULD-FIX) — `update_snippet`
    /// must normalize `trigger` to lowercase the same way `insert_snippet`
    /// does. Without this, a user editing an existing snippet to mixed case
    /// persists mixed case and silently breaks the whole-utterance matcher
    /// (FR-3.2), with no DB CHECK to catch it.
    #[test]
    fn update_normalizes_trigger_lowercase() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let conn = open_test_db(tmp.path());

        let row = sample("snip-update-case", "sign off");
        insert_snippet(&conn, &row).expect("insert_snippet");

        let mut updated = row.clone();
        updated.trigger = "MyTrigger".to_string();
        update_snippet(&conn, &updated).expect("update_snippet");

        let listed = list_snippets(&conn).expect("list_snippets");
        assert_eq!(listed.len(), 1);
        assert_eq!(
            listed[0].trigger, "mytrigger",
            "update_snippet must lowercase the trigger before persisting (§8.1), \
             same as insert_snippet"
        );
    }
}
