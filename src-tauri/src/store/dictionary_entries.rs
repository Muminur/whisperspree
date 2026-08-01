//! `dictionary_entries` CRUD store (FR-3.1 personalization dictionary; the
//! bias/replacer logic itself is the separate top-level `dictionary.rs`
//! module, T5.1 — see OPEN_QUESTIONS Q9 for why these are deliberately two
//! different files/modules).
//!
//! PRD refs:
//! - §8.1 — `dictionary_entries` schema: `phrase TEXT NOT NULL UNIQUE`.
//! - §9.1 — `list/add/update/delete_dictionary_entry` command logic.
//! - §14 `DB-IO` — a UNIQUE(`phrase`) violation, or any other rusqlite
//!   failure, maps here (OPEN_QUESTIONS Q8/plan edge cases).
//! - OPEN_QUESTIONS Q9 — this store lives at `store::dictionary_entries`, not
//!   `store::dictionary`, to stay distinct from the future FR-3.1 module.
//!
//! Every `&Connection` passed here MUST come from [`super::db::open`].

use crate::error::Error;
use rusqlite::Connection;

/// A `dictionary_entries` row (§8.1 column-for-column).
#[derive(Debug, Clone, PartialEq)]
pub struct DictionaryEntryRow {
    pub id: String,
    pub phrase: String,
    pub sounds_like: String, // JSON string array, e.g. `["sowndz","layk"]`
    pub case_sensitive: bool,
    pub created_at: String,
    pub last_used_at: Option<String>,
}

/// §9.1 `list_dictionary_entry`.
pub fn list_dictionary_entries(conn: &Connection) -> Result<Vec<DictionaryEntryRow>, Error> {
    let mut stmt = conn
        .prepare(
            "SELECT id, phrase, sounds_like, case_sensitive, created_at, last_used_at \
             FROM dictionary_entries",
        )
        .map_err(|e| super::db_io("list_dictionary_entries prepare", e))?;
    let rows = stmt
        .query_map([], |row| {
            Ok(DictionaryEntryRow {
                id: row.get(0)?,
                phrase: row.get(1)?,
                sounds_like: row.get(2)?,
                case_sensitive: row.get(3)?,
                created_at: row.get(4)?,
                last_used_at: row.get(5)?,
            })
        })
        .map_err(|e| super::db_io("list_dictionary_entries query", e))?;

    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(|e| super::db_io("list_dictionary_entries row", e))?);
    }
    Ok(out)
}

/// §9.1 `add_dictionary_entry`. A duplicate `phrase` (UNIQUE) maps to `DB-IO`
/// (§14/plan edge cases).
pub fn insert_dictionary_entry(conn: &Connection, row: &DictionaryEntryRow) -> Result<(), Error> {
    conn.execute(
        "INSERT INTO dictionary_entries (id, phrase, sounds_like, case_sensitive, created_at, last_used_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        rusqlite::params![
            row.id,
            row.phrase,
            row.sounds_like,
            row.case_sensitive,
            row.created_at,
            row.last_used_at,
        ],
    )
    .map_err(|e| super::db_io("insert_dictionary_entry", e))?;
    Ok(())
}

/// §9.1 `update_dictionary_entry`.
pub fn update_dictionary_entry(conn: &Connection, row: &DictionaryEntryRow) -> Result<(), Error> {
    conn.execute(
        "UPDATE dictionary_entries SET phrase = ?2, sounds_like = ?3, case_sensitive = ?4, \
         created_at = ?5, last_used_at = ?6 WHERE id = ?1",
        rusqlite::params![
            row.id,
            row.phrase,
            row.sounds_like,
            row.case_sensitive,
            row.created_at,
            row.last_used_at,
        ],
    )
    .map_err(|e| super::db_io("update_dictionary_entry", e))?;
    Ok(())
}

/// §9.1 `delete_dictionary_entry`. Returns whether a row was actually deleted.
pub fn delete_dictionary_entry(conn: &Connection, id: &str) -> Result<bool, Error> {
    let changed = conn
        .execute("DELETE FROM dictionary_entries WHERE id = ?1", [id])
        .map_err(|e| super::db_io("delete_dictionary_entry", e))?;
    Ok(changed > 0)
}

// ---------------------------------------------------------------------------
// Tests (colocated so `cargo test store::` / `cargo test dictionary_entries::`
// filters here). Real SQLite in tempdirs, opened via `super::db::open`.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn open_test_db(tmp: &std::path::Path) -> Connection {
        super::super::db::open(&tmp.join("whisperspree.db")).expect("db::open a fresh test DB")
    }

    fn sample(id: &str, phrase: &str) -> DictionaryEntryRow {
        DictionaryEntryRow {
            id: id.to_string(),
            phrase: phrase.to_string(),
            sounds_like: "[]".to_string(),
            case_sensitive: false,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            last_used_at: None,
        }
    }

    /// AC: §9.1 dictionary CRUD — insert, list, update, delete round-trip
    /// through the real table.
    #[test]
    fn dictionary_entries_insert_get_update_delete_roundtrip() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let conn = open_test_db(tmp.path());

        let row = sample("entry-1", "kubernetes");
        insert_dictionary_entry(&conn, &row).expect("insert_dictionary_entry");

        let listed = list_dictionary_entries(&conn).expect("list_dictionary_entries");
        assert_eq!(listed, vec![row.clone()]);

        let mut updated = row.clone();
        updated.sounds_like = r#"["koo-ber-net-eez"]"#.to_string();
        updated.last_used_at = Some("2026-01-02T00:00:00Z".to_string());
        update_dictionary_entry(&conn, &updated).expect("update_dictionary_entry");

        let listed_after_update = list_dictionary_entries(&conn).expect("list after update");
        assert_eq!(listed_after_update, vec![updated]);

        let deleted = delete_dictionary_entry(&conn, "entry-1").expect("delete_dictionary_entry");
        assert!(
            deleted,
            "delete_dictionary_entry must report a row was deleted"
        );
        assert!(list_dictionary_entries(&conn)
            .expect("list after delete")
            .is_empty());

        let deleted_again = delete_dictionary_entry(&conn, "entry-1")
            .expect("delete on a missing id must not error");
        assert!(
            !deleted_again,
            "deleting an already-gone id must report false, not error"
        );
    }

    /// AC: §8.1 `phrase TEXT NOT NULL UNIQUE` / §14 `DB-IO` — inserting a
    /// second entry with a duplicate phrase maps to `DB-IO`.
    #[test]
    fn phrase_unique_violation_maps_db_io() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let conn = open_test_db(tmp.path());

        insert_dictionary_entry(&conn, &sample("entry-a", "duplicate-phrase"))
            .expect("first insert must succeed");

        let err = insert_dictionary_entry(&conn, &sample("entry-b", "duplicate-phrase"))
            .expect_err("a duplicate phrase must violate the UNIQUE constraint");
        assert_eq!(
            err.code(),
            "DB-IO",
            "UNIQUE(phrase) violation must map to DB-IO (§14)"
        );
    }
}
