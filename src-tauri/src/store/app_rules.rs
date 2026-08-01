//! `app_rules` CRUD store (FR-2.3 per-app style/persona rule resolution).
//!
//! PRD refs:
//! - §8.1 — `app_rules` schema, notably
//!   `custom_prompt_id TEXT REFERENCES custom_prompts(id) ON DELETE SET NULL`.
//! - §9.1 — app-rule CRUD commands.
//! - §14 `DB-IO` — every rusqlite failure maps here.
//!
//! Every `&Connection` passed here MUST come from [`super::db::open`] — the
//! `ON DELETE SET NULL` FK on `custom_prompt_id` only fires when
//! `PRAGMA foreign_keys` is ON, which is a per-connection setting `db::open`
//! re-applies on every call (see `db.rs` module docs and the plan's highest
//! risk item).

use crate::error::Error;
use rusqlite::Connection;

/// An `app_rules` row (§8.1 column-for-column).
#[derive(Debug, Clone, PartialEq)]
pub struct AppRuleRow {
    pub id: String,
    pub bundle_id: String,
    pub title_regex: Option<String>,
    pub style_id: Option<String>,
    pub persona_id: Option<String>,
    pub custom_prompt_id: Option<String>,
    pub priority: i64,
}

/// §9.1 `list_app_rule`.
pub fn list_app_rules(conn: &Connection) -> Result<Vec<AppRuleRow>, Error> {
    let mut stmt = conn
        .prepare(
            "SELECT id, bundle_id, title_regex, style_id, persona_id, custom_prompt_id, priority \
             FROM app_rules",
        )
        .map_err(|e| super::db_io("list_app_rules prepare", e))?;
    let rows = stmt
        .query_map([], |row| {
            Ok(AppRuleRow {
                id: row.get(0)?,
                bundle_id: row.get(1)?,
                title_regex: row.get(2)?,
                style_id: row.get(3)?,
                persona_id: row.get(4)?,
                custom_prompt_id: row.get(5)?,
                priority: row.get(6)?,
            })
        })
        .map_err(|e| super::db_io("list_app_rules query", e))?;

    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(|e| super::db_io("list_app_rules row", e))?);
    }
    Ok(out)
}

/// §9.1 `add_app_rule`.
pub fn insert_app_rule(conn: &Connection, row: &AppRuleRow) -> Result<(), Error> {
    conn.execute(
        "INSERT INTO app_rules (id, bundle_id, title_regex, style_id, persona_id, custom_prompt_id, priority)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        rusqlite::params![
            row.id,
            row.bundle_id,
            row.title_regex,
            row.style_id,
            row.persona_id,
            row.custom_prompt_id,
            row.priority,
        ],
    )
    .map_err(|e| super::db_io("insert_app_rule", e))?;
    Ok(())
}

/// §9.1 `update_app_rule`.
pub fn update_app_rule(conn: &Connection, row: &AppRuleRow) -> Result<(), Error> {
    conn.execute(
        "UPDATE app_rules SET bundle_id = ?2, title_regex = ?3, style_id = ?4, persona_id = ?5, \
         custom_prompt_id = ?6, priority = ?7 WHERE id = ?1",
        rusqlite::params![
            row.id,
            row.bundle_id,
            row.title_regex,
            row.style_id,
            row.persona_id,
            row.custom_prompt_id,
            row.priority,
        ],
    )
    .map_err(|e| super::db_io("update_app_rule", e))?;
    Ok(())
}

/// §9.1 `delete_app_rule`. Returns whether a row was actually deleted.
pub fn delete_app_rule(conn: &Connection, id: &str) -> Result<bool, Error> {
    let changed = conn
        .execute("DELETE FROM app_rules WHERE id = ?1", [id])
        .map_err(|e| super::db_io("delete_app_rule", e))?;
    Ok(changed > 0)
}

// ---------------------------------------------------------------------------
// Tests (colocated so `cargo test store::` / `cargo test app_rules::` filters
// here). Real SQLite in tempdirs, opened via `super::db::open` (never an
// inline `PRAGMA foreign_keys=ON` — see module doc / db.rs).
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::custom_prompts::{self, CustomPromptRow};

    fn open_test_db(tmp: &std::path::Path) -> Connection {
        super::super::db::open(&tmp.join("whisperspree.db")).expect("db::open a fresh test DB")
    }

    fn sample(id: &str, bundle_id: &str) -> AppRuleRow {
        AppRuleRow {
            id: id.to_string(),
            bundle_id: bundle_id.to_string(),
            title_regex: None,
            style_id: Some("professional".to_string()),
            persona_id: Some("clean".to_string()),
            custom_prompt_id: None,
            priority: 0,
        }
    }

    /// AC: §9.1 app-rule CRUD — insert, list, update, delete round-trip
    /// through the real table.
    #[test]
    fn app_rules_insert_get_update_delete_roundtrip() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let conn = open_test_db(tmp.path());

        let row = sample("rule-1", "com.tinyspeck.slackmacgap");
        insert_app_rule(&conn, &row).expect("insert_app_rule");

        let listed = list_app_rules(&conn).expect("list_app_rules");
        assert_eq!(listed, vec![row.clone()]);

        let mut updated = row.clone();
        updated.priority = 10;
        updated.title_regex = Some("^#general".to_string());
        update_app_rule(&conn, &updated).expect("update_app_rule");

        assert_eq!(
            list_app_rules(&conn).expect("list after update"),
            vec![updated]
        );

        let deleted = delete_app_rule(&conn, "rule-1").expect("delete_app_rule");
        assert!(deleted);
        assert!(list_app_rules(&conn).expect("list after delete").is_empty());

        let deleted_again =
            delete_app_rule(&conn, "rule-1").expect("delete on a missing id must not error");
        assert!(!deleted_again);
    }

    /// AC: §8.1 `app_rules.custom_prompt_id … ON DELETE SET NULL` — deleting a
    /// referenced `custom_prompts` row nulls out `app_rules.custom_prompt_id`
    /// instead of erroring or orphaning. Requires `PRAGMA foreign_keys=ON`
    /// (per-connection, `db::open`-only trap — same class of bug as
    /// `history::fk_cascade_deletes_transforms`); the connection here comes
    /// exclusively from `db::open`, never an inline PRAGMA in the test.
    #[test]
    fn custom_prompt_delete_sets_null() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let conn = open_test_db(tmp.path());

        let prompt = CustomPromptRow {
            id: "prompt-set-null".to_string(),
            name: "Follow-up".to_string(),
            prompt_text: "Rewrite as a follow-up.".to_string(),
            created_at: "2026-01-01T00:00:00Z".to_string(),
        };
        custom_prompts::insert_custom_prompt(&conn, &prompt).expect("insert_custom_prompt");

        let mut rule = sample("rule-with-prompt", "com.microsoft.VSCode");
        rule.custom_prompt_id = Some("prompt-set-null".to_string());
        insert_app_rule(&conn, &rule).expect("insert_app_rule");

        custom_prompts::delete_custom_prompt(&conn, "prompt-set-null")
            .expect("delete_custom_prompt");

        let after = list_app_rules(&conn).expect("list_app_rules after custom_prompt delete");
        assert_eq!(after.len(), 1);
        assert_eq!(
            after[0].custom_prompt_id, None,
            "ON DELETE SET NULL must null app_rules.custom_prompt_id when the referenced \
             custom_prompts row is deleted (requires PRAGMA foreign_keys=ON per db::open)"
        );
    }
}
