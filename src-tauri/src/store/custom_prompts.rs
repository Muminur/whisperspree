//! `custom_prompts` CRUD store (FR-2.5 custom system prompts).
//!
//! PRD refs:
//! - §8.1 — `custom_prompts` schema.
//! - §9.1 — custom-prompt CRUD commands.
//! - §8.1 — `app_rules.custom_prompt_id … ON DELETE SET NULL`: deleting a
//!   custom prompt must null out any `app_rules` row referencing it (proven in
//!   `app_rules::custom_prompt_delete_sets_null`, not here — this module only
//!   owns `custom_prompts` itself).
//! - §14 `DB-IO` — every rusqlite failure maps here.
//!
//! Every `&Connection` passed here MUST come from [`super::db::open`] — the
//! `ON DELETE SET NULL` cascade into `app_rules` (proven in `app_rules.rs`)
//! only fires when `PRAGMA foreign_keys` is ON.

use crate::error::Error;
use rusqlite::Connection;

/// A `custom_prompts` row (§8.1 column-for-column).
#[derive(Debug, Clone, PartialEq)]
pub struct CustomPromptRow {
    pub id: String,
    pub name: String,
    pub prompt_text: String,
    pub created_at: String,
}

/// §9.1 `list_custom_prompt`.
pub fn list_custom_prompts(conn: &Connection) -> Result<Vec<CustomPromptRow>, Error> {
    let mut stmt = conn
        .prepare("SELECT id, name, prompt_text, created_at FROM custom_prompts")
        .map_err(|e| super::db_io("list_custom_prompts prepare", e))?;
    let rows = stmt
        .query_map([], |row| {
            Ok(CustomPromptRow {
                id: row.get(0)?,
                name: row.get(1)?,
                prompt_text: row.get(2)?,
                created_at: row.get(3)?,
            })
        })
        .map_err(|e| super::db_io("list_custom_prompts query", e))?;

    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(|e| super::db_io("list_custom_prompts row", e))?);
    }
    Ok(out)
}

/// §9.1 `add_custom_prompt`.
pub fn insert_custom_prompt(conn: &Connection, row: &CustomPromptRow) -> Result<(), Error> {
    conn.execute(
        "INSERT INTO custom_prompts (id, name, prompt_text, created_at) VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params![row.id, row.name, row.prompt_text, row.created_at],
    )
    .map_err(|e| super::db_io("insert_custom_prompt", e))?;
    Ok(())
}

/// §9.1 `update_custom_prompt`.
pub fn update_custom_prompt(conn: &Connection, row: &CustomPromptRow) -> Result<(), Error> {
    conn.execute(
        "UPDATE custom_prompts SET name = ?2, prompt_text = ?3, created_at = ?4 WHERE id = ?1",
        rusqlite::params![row.id, row.name, row.prompt_text, row.created_at],
    )
    .map_err(|e| super::db_io("update_custom_prompt", e))?;
    Ok(())
}

/// §9.1 `delete_custom_prompt`. Returns whether a row was actually deleted.
/// Any `app_rules.custom_prompt_id` referencing this id is set to `NULL` by
/// the FK (§8.1 `ON DELETE SET NULL`) — see `app_rules::custom_prompt_delete_sets_null`.
pub fn delete_custom_prompt(conn: &Connection, id: &str) -> Result<bool, Error> {
    let changed = conn
        .execute("DELETE FROM custom_prompts WHERE id = ?1", [id])
        .map_err(|e| super::db_io("delete_custom_prompt", e))?;
    Ok(changed > 0)
}

// ---------------------------------------------------------------------------
// Tests (colocated so `cargo test store::` / `cargo test custom_prompts::`
// filters here). Real SQLite in tempdirs, opened via `super::db::open`.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn open_test_db(tmp: &std::path::Path) -> Connection {
        super::super::db::open(&tmp.join("whisperspree.db")).expect("db::open a fresh test DB")
    }

    fn sample(id: &str, name: &str) -> CustomPromptRow {
        CustomPromptRow {
            id: id.to_string(),
            name: name.to_string(),
            prompt_text: "Rewrite as a formal follow-up email.".to_string(),
            created_at: "2026-01-01T00:00:00Z".to_string(),
        }
    }

    /// AC: §9.1 custom-prompt CRUD — insert, list, update, delete round-trip
    /// through the real table.
    #[test]
    fn custom_prompts_insert_get_update_delete_roundtrip() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let conn = open_test_db(tmp.path());

        let row = sample("prompt-1", "Follow-up email");
        insert_custom_prompt(&conn, &row).expect("insert_custom_prompt");

        let listed = list_custom_prompts(&conn).expect("list_custom_prompts");
        assert_eq!(listed, vec![row.clone()]);

        let mut updated = row.clone();
        updated.prompt_text = "Rewrite as a casual follow-up email.".to_string();
        update_custom_prompt(&conn, &updated).expect("update_custom_prompt");

        assert_eq!(
            list_custom_prompts(&conn).expect("list after update"),
            vec![updated]
        );

        let deleted = delete_custom_prompt(&conn, "prompt-1").expect("delete_custom_prompt");
        assert!(deleted);
        assert!(list_custom_prompts(&conn)
            .expect("list after delete")
            .is_empty());

        let deleted_again =
            delete_custom_prompt(&conn, "prompt-1").expect("delete on a missing id must not error");
        assert!(!deleted_again);
    }
}
