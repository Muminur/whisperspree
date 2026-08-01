//! `dictations` + `dictation_transforms` CRUD and the §8.1 retention job (§11
//! names `history.rs`; the plan colocates retention here because "retention
//! lives with the history tables").
//!
//! PRD refs:
//! - §8.1 — `dictations` / `dictation_transforms` schema; `dictation_transforms
//!   .dictation_id … ON DELETE CASCADE`; retention job deletes dictations
//!   older than `history.retentionDays`, cascading transforms, unlinking
//!   `audio_path`.
//! - §9.1 — `list_dictations` (`LIKE` search on both `raw_transcript` and
//!   `processed_text`, `limit` default 50, `beforeId` keyset paging),
//!   `get_dictation`/`delete_dictation`/`clear_history` (delete unlinks
//!   audio), `export_history` (JSON lines).
//! - §12 P-6 — delete/clear are immediate and unlink audio; export is
//!   user-initiated.
//! - §14 `DB-IO` — every rusqlite failure maps here.
//! - `store::history` does NOT decide `history.saveText=false` — that's a
//!   caller-side (T0.5 pipeline) decision; these functions stay pure CRUD.
//! - OPEN_QUESTIONS Q11 — `retention_sweep(_, 0)` is a no-op (retention
//!   disabled), never "delete everything".
//!
//! **Every `&Connection` passed into this module's functions MUST come from
//! [`super::db::open`]** — `ON DELETE CASCADE` on `dictation_transforms` only
//! fires when `PRAGMA foreign_keys` is ON, which is a per-connection setting
//! `db::open` re-applies on every call (see `db.rs` module docs).

use crate::error::Error;
use rusqlite::{Connection, OptionalExtension};
use std::path::{Path, PathBuf};

/// A `dictations` row (§8.1 column-for-column).
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DictationRow {
    pub id: String,
    pub created_at: String,
    pub app_bundle_id: Option<String>,
    pub app_name: Option<String>,
    pub style_id: String,
    pub engine: String,
    pub language: Option<String>,
    pub duration_ms: i64,
    pub raw_transcript: String,
    pub processed_text: String,
    pub persona_id: String,
    pub fallback_used: bool,
    pub inject_method: Option<String>,
    pub audio_path: Option<String>,
    pub word_timings: Option<String>,
}

/// A `dictation_transforms` row (§8.1 column-for-column).
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TransformRow {
    pub id: String,
    pub dictation_id: String,
    pub template_id: String,
    pub created_at: String,
    pub output_text: String,
}

/// §9.1 dictation history write path: insert one `dictations` row. A UNIQUE
/// (`id`) or CHECK (`engine`) violation, or any other rusqlite failure, maps
/// to `DB-IO` (§14).
pub fn insert_dictation(conn: &Connection, row: &DictationRow) -> Result<(), Error> {
    conn.execute(
        "INSERT INTO dictations (
            id, created_at, app_bundle_id, app_name, style_id, engine, language,
            duration_ms, raw_transcript, processed_text, persona_id, fallback_used,
            inject_method, audio_path, word_timings
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
        rusqlite::params![
            row.id,
            row.created_at,
            row.app_bundle_id,
            row.app_name,
            row.style_id,
            row.engine,
            row.language,
            row.duration_ms,
            row.raw_transcript,
            row.processed_text,
            row.persona_id,
            row.fallback_used,
            row.inject_method,
            row.audio_path,
            row.word_timings,
        ],
    )
    .map_err(|e| super::db_io("insert_dictation", e))?;
    Ok(())
}

const DICTATION_COLUMNS: &str = "id, created_at, app_bundle_id, app_name, style_id, engine, \
     language, duration_ms, raw_transcript, processed_text, persona_id, fallback_used, \
     inject_method, audio_path, word_timings";

fn row_to_dictation(row: &rusqlite::Row<'_>) -> rusqlite::Result<DictationRow> {
    Ok(DictationRow {
        id: row.get(0)?,
        created_at: row.get(1)?,
        app_bundle_id: row.get(2)?,
        app_name: row.get(3)?,
        style_id: row.get(4)?,
        engine: row.get(5)?,
        language: row.get(6)?,
        duration_ms: row.get(7)?,
        raw_transcript: row.get(8)?,
        processed_text: row.get(9)?,
        persona_id: row.get(10)?,
        fallback_used: row.get(11)?,
        inject_method: row.get(12)?,
        audio_path: row.get(13)?,
        word_timings: row.get(14)?,
    })
}

/// §9.1 `get_dictation`: fetch one row by `id`, or `None` if absent.
pub fn get_dictation(conn: &Connection, id: &str) -> Result<Option<DictationRow>, Error> {
    conn.query_row(
        &format!("SELECT {DICTATION_COLUMNS} FROM dictations WHERE id = ?1"),
        [id],
        row_to_dictation,
    )
    .optional()
    .map_err(|e| super::db_io("get_dictation", e))
}

/// Escape `\`, `%`, and `_` (SQLite `LIKE` metacharacters) so a user-supplied
/// search term matches only literally, paired with `LIKE ... ESCAPE '\'` at
/// the call site (security review S2).
fn escape_like_term(term: &str) -> String {
    let mut escaped = String::with_capacity(term.len());
    for c in term.chars() {
        if matches!(c, '\\' | '%' | '_') {
            escaped.push('\\');
        }
        escaped.push(c);
    }
    escaped
}

/// §9.1 `list_dictations`: `LIKE` search on BOTH `raw_transcript` and
/// `processed_text` when `q` is `Some`; `limit` bounds the result count;
/// `before_id` keyset-pages strictly before that row's `created_at`
/// (`idx_dictations_created`, newest-first).
pub fn list_dictations(
    conn: &Connection,
    q: Option<&str>,
    limit: i64,
    before_id: Option<&str>,
) -> Result<Vec<DictationRow>, Error> {
    let mut clauses: Vec<String> = Vec::new();
    let mut params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

    if let Some(term) = q {
        // Security review S2: a user-supplied `%`/`_` (or `\`) must be
        // matched literally, not act as an unescaped SQL LIKE wildcard.
        clauses.push(
            "(raw_transcript LIKE ?1 ESCAPE '\\' OR processed_text LIKE ?1 ESCAPE '\\')"
                .to_string(),
        );
        params.push(Box::new(format!("%{}%", escape_like_term(term))));
    }
    if let Some(id) = before_id {
        clauses.push(format!(
            "created_at < (SELECT created_at FROM dictations WHERE id = ?{})",
            params.len() + 1
        ));
        params.push(Box::new(id.to_string()));
    }

    let where_clause = if clauses.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", clauses.join(" AND "))
    };
    let limit_placeholder = params.len() + 1;
    // Security review S3: SQLite treats a non-positive LIMIT as "unlimited"
    // (`LIMIT -1`/`LIMIT 0` semantics vary but neither bounds the result),
    // silently returning the entire history table. Clamp to the §9.1 default
    // page size rather than pass a non-positive caller value through.
    const DEFAULT_LIMIT: i64 = 50;
    let effective_limit = if limit > 0 { limit } else { DEFAULT_LIMIT };
    params.push(Box::new(effective_limit));

    let sql = format!(
        "SELECT {DICTATION_COLUMNS} FROM dictations {where_clause} \
         ORDER BY created_at DESC LIMIT ?{limit_placeholder}"
    );

    let mut stmt = conn
        .prepare(&sql)
        .map_err(|e| super::db_io("list_dictations prepare", e))?;
    let param_refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|p| p.as_ref()).collect();
    let rows = stmt
        .query_map(param_refs.as_slice(), row_to_dictation)
        .map_err(|e| super::db_io("list_dictations query", e))?;

    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(|e| super::db_io("list_dictations row", e))?);
    }
    Ok(out)
}

/// §9.1 `delete_dictation` / §12 P-6: delete the row (transforms cascade via
/// FK, §8.1) and return its `audio_path` (if any) so the caller can unlink it.
/// `None` if the id did not exist.
pub fn delete_dictation(conn: &Connection, id: &str) -> Result<Option<PathBuf>, Error> {
    let audio_path: Option<Option<String>> = conn
        .query_row(
            "SELECT audio_path FROM dictations WHERE id = ?1",
            [id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| super::db_io("delete_dictation lookup", e))?;

    let Some(audio_path) = audio_path else {
        return Ok(None);
    };

    conn.execute("DELETE FROM dictations WHERE id = ?1", [id])
        .map_err(|e| super::db_io("delete_dictation", e))?;

    Ok(audio_path.map(PathBuf::from))
}

/// §9.1 `clear_history` / §12 P-6: delete every `dictations` row (transforms
/// cascade) and return every non-null `audio_path` for the caller to unlink.
pub fn clear_history(conn: &Connection) -> Result<Vec<PathBuf>, Error> {
    let mut stmt = conn
        .prepare("SELECT audio_path FROM dictations WHERE audio_path IS NOT NULL")
        .map_err(|e| super::db_io("clear_history select", e))?;
    let paths: Vec<PathBuf> = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|e| super::db_io("clear_history select", e))?
        .map(|r| r.map(PathBuf::from))
        .collect::<rusqlite::Result<_>>()
        .map_err(|e| super::db_io("clear_history select", e))?;
    drop(stmt);

    conn.execute("DELETE FROM dictations", [])
        .map_err(|e| super::db_io("clear_history delete", e))?;

    Ok(paths)
}

/// Insert one `dictation_transforms` row (FR-2.6 `reprocess_dictation`
/// write path).
pub fn insert_transform(conn: &Connection, row: &TransformRow) -> Result<(), Error> {
    conn.execute(
        "INSERT INTO dictation_transforms (id, dictation_id, template_id, created_at, output_text)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        rusqlite::params![
            row.id,
            row.dictation_id,
            row.template_id,
            row.created_at,
            row.output_text,
        ],
    )
    .map_err(|e| super::db_io("insert_transform", e))?;
    Ok(())
}

/// List every transform recorded for one dictation.
pub fn list_transforms(conn: &Connection, dictation_id: &str) -> Result<Vec<TransformRow>, Error> {
    let mut stmt = conn
        .prepare(
            "SELECT id, dictation_id, template_id, created_at, output_text \
             FROM dictation_transforms WHERE dictation_id = ?1",
        )
        .map_err(|e| super::db_io("list_transforms prepare", e))?;
    let rows = stmt
        .query_map([dictation_id], |row| {
            Ok(TransformRow {
                id: row.get(0)?,
                dictation_id: row.get(1)?,
                template_id: row.get(2)?,
                created_at: row.get(3)?,
                output_text: row.get(4)?,
            })
        })
        .map_err(|e| super::db_io("list_transforms query", e))?;

    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(|e| super::db_io("list_transforms row", e))?);
    }
    Ok(out)
}

/// §9.1 `export_history`: write every `dictations` row as one JSON object per
/// line (JSON-lines) to `w`, oldest/newest order unspecified by §9.1; returns
/// the row count written.
pub fn export_history(conn: &Connection, w: &mut dyn std::io::Write) -> Result<usize, Error> {
    let mut stmt = conn
        .prepare(&format!("SELECT {DICTATION_COLUMNS} FROM dictations"))
        .map_err(|e| super::db_io("export_history prepare", e))?;
    let rows = stmt
        .query_map([], row_to_dictation)
        .map_err(|e| super::db_io("export_history query", e))?;

    let mut count = 0usize;
    for row in rows {
        let row = row.map_err(|e| super::db_io("export_history row", e))?;
        let line =
            serde_json::to_string(&row).map_err(|e| super::db_io("export_history serialize", e))?;
        writeln!(w, "{line}").map_err(|e| super::db_io("export_history write", e))?;
        count += 1;
    }
    Ok(count)
}

/// §8.1 retention job: delete `dictations` rows older than `retention_days`
/// (cascading `dictation_transforms` — requires `foreign_keys=ON`, see the
/// module doc), returning every collected `audio_path` for the caller/logger
/// (best-effort unlink happens here too, confined to the app's own audio
/// directory — see [`confine_to_audio_dir`]; scheduling itself is T0.5+).
///
// PRD-QUESTION(Q11): §8.1 doesn't define what `retentionDays = 0` means.
// Conservative resolution (OPEN_QUESTIONS Q11): 0 = retention DISABLED, a
// no-op — never destroy the user's entire history on an off-by-one/unset
// field.
//
// Code review S2: a plain SQL `created_at < cutoff` string comparison is only
// chronologically correct for fixed-width, zero-padded, whole-second,
// same-timezone timestamps. A fractional-seconds row can lexicographically
// sort before a whole-second cutoff (`.` < `Z`), and an empty/malformed
// `created_at` sorts before everything and would always be "expired". So
// retention fetches every row and only deletes ones it can POSITIVELY parse
// and prove are older than the cutoff (`parse_iso8601_utc_seconds`); anything
// unparseable is kept — never destroy data we cannot confidently prove is old
// (§12 privacy).
pub fn retention_sweep(conn: &Connection, retention_days: u32) -> Result<Vec<PathBuf>, Error> {
    if retention_days == 0 {
        return Ok(Vec::new());
    }

    let cutoff_secs = unix_seconds_now() - i64::from(retention_days) * 86_400;

    let mut stmt = conn
        .prepare("SELECT id, created_at, audio_path FROM dictations")
        .map_err(|e| super::db_io("retention_sweep select", e))?;
    let rows: Vec<(String, String, Option<String>)> = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        })
        .map_err(|e| super::db_io("retention_sweep select", e))?
        .collect::<rusqlite::Result<_>>()
        .map_err(|e| super::db_io("retention_sweep select", e))?;
    drop(stmt);

    let mut expired_ids: Vec<String> = Vec::new();
    let mut audio_paths: Vec<PathBuf> = Vec::new();
    for (id, created_at, audio_path) in rows {
        let Some(secs) = parse_iso8601_utc_seconds(&created_at) else {
            continue; // unparseable: never destroy data we can't prove is old
        };
        if secs < cutoff_secs {
            expired_ids.push(id);
            if let Some(p) = audio_path {
                audio_paths.push(PathBuf::from(p));
            }
        }
    }

    for id in &expired_ids {
        conn.execute("DELETE FROM dictations WHERE id = ?1", [id])
            .map_err(|e| super::db_io("retention_sweep delete", e))?;
    }

    // Best-effort unlink (§12 P-6), confined to the app's own audio directory
    // (security review B1): a missing/unremovable/out-of-bounds file must
    // never fail the sweep, and confinement never blocks the row cleanup
    // above — it only guards the filesystem.
    for path in &audio_paths {
        if let Some(confined) = confine_to_audio_dir(conn, &path.to_string_lossy()) {
            let _ = std::fs::remove_file(confined);
        }
    }

    Ok(audio_paths)
}

/// Security review B1 (BLOCKING): confine a DB-supplied candidate path to the
/// app's own `audio/` directory (§4.4, sibling to the SQLite file) before any
/// caller unlinks it. Canonicalizes BOTH the audio directory and the
/// candidate — this resolves `..` traversal and symlinks — and returns the
/// canonicalized candidate only if it lies under the canonicalized audio
/// directory. Returns `None` (skip the unlink, never error) if: the
/// connection has no known file path, the audio directory doesn't exist, the
/// candidate doesn't exist, or the resolved candidate escapes the audio
/// directory. Every unlink site in this module (and any future caller that
/// unlinks a `delete_dictation`/`clear_history`-returned `audio_path`) MUST
/// route through this guard rather than calling `std::fs::remove_file`
/// directly on a DB-supplied path.
pub(crate) fn confine_to_audio_dir(conn: &Connection, candidate: &str) -> Option<PathBuf> {
    let audio_dir = audio_dir_for(conn)?;
    let canonical_dir = std::fs::canonicalize(audio_dir).ok()?;
    let canonical_candidate = std::fs::canonicalize(candidate).ok()?;
    if canonical_candidate.starts_with(&canonical_dir) {
        Some(canonical_candidate)
    } else {
        None
    }
}

/// The app's audio directory (§4.4): the `audio/` sibling of the SQLite file
/// backing `conn`. `None` if `conn` has no known file path (e.g. `:memory:`).
fn audio_dir_for(conn: &Connection) -> Option<PathBuf> {
    let db_path = conn.path()?;
    Path::new(db_path).parent().map(|p| p.join("audio"))
}

/// Parse a `dictations.created_at` value (§8.1 ISO-8601 UTC, optionally with
/// fractional seconds: `YYYY-MM-DDTHH:MM:SS[.fff...]Z`) into whole Unix
/// seconds (fractional seconds are truncated, not rounded). `None` for
/// anything that doesn't strictly match this shape — callers must treat that
/// as "cannot prove this is old" (code review S2), never as "very old".
/// Comparison against a cutoff (itself always whole-second, no fractional
/// part) uses a strict `<`, so truncating fractional seconds cannot make an
/// otherwise-newer row look older: a row at the same whole second as the
/// cutoff is never "less than" it, regardless of its fractional remainder.
fn parse_iso8601_utc_seconds(s: &str) -> Option<i64> {
    let bytes = s.as_bytes();
    if bytes.len() < 20 || !s.ends_with('Z') {
        return None;
    }
    if bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes[10] != b'T'
        || bytes[13] != b':'
        || bytes[16] != b':'
    {
        return None;
    }

    let year: i64 = s.get(0..4)?.parse().ok()?;
    let month: u32 = s.get(5..7)?.parse().ok()?;
    let day: u32 = s.get(8..10)?.parse().ok()?;
    let hour: i64 = s.get(11..13)?.parse().ok()?;
    let minute: i64 = s.get(14..16)?.parse().ok()?;
    let second: i64 = s.get(17..19)?.parse().ok()?;

    // Anything between the whole seconds and the trailing 'Z' must be empty
    // or a well-formed fractional-seconds suffix (`.` followed by digits).
    let rest = s.get(19..s.len() - 1)?;
    let fractional_ok = rest.is_empty()
        || (rest.len() > 1
            && rest.starts_with('.')
            && rest[1..].bytes().all(|b| b.is_ascii_digit()));
    if !fractional_ok {
        return None;
    }

    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    if !(0..24).contains(&hour) || !(0..60).contains(&minute) || !(0..60).contains(&second) {
        return None;
    }

    let days = days_from_civil(year, month, day);
    Some(days * 86_400 + hour * 3600 + minute * 60 + second)
}

/// Howard Hinnant's `days_from_civil` algorithm (public domain): (year,
/// month, day) in the proleptic Gregorian calendar -> days since the Unix
/// epoch. Inverse of [`civil_from_days`].
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400; // [0, 399]
    let mp = if m > 2 { m as i64 - 3 } else { m as i64 + 9 };
    let doy = (153 * mp + 2) / 5 + d as i64 - 1; // [0, 365]
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // [0, 146096]
    era * 146_097 + doe - 719_468
}

fn unix_seconds_now() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

/// Render a Unix timestamp as `YYYY-MM-DDTHH:MM:SSZ`. `retention_sweep` no
/// longer builds a cutoff string for a lexicographic compare (code review
/// S2 — see `parse_iso8601_utc_seconds`), so this is test-only now: it lets
/// `retention_keeps_row_with_fractional_seconds_newer_than_cutoff` mirror
/// `retention_sweep`'s own cutoff-second computation exactly.
#[cfg(test)]
fn iso8601_from_unix_seconds(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    let secs_of_day = secs.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    let hh = secs_of_day / 3600;
    let mm = (secs_of_day % 3600) / 60;
    let ss = secs_of_day % 60;
    format!("{y:04}-{m:02}-{d:02}T{hh:02}:{mm:02}:{ss:02}Z")
}

/// Howard Hinnant's `civil_from_days` algorithm (public domain): days since
/// the Unix epoch -> (year, month, day) in the proleptic Gregorian calendar.
/// Test-only, see [`iso8601_from_unix_seconds`].
#[cfg(test)]
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

// ---------------------------------------------------------------------------
// Tests (colocated so `cargo test store::` / `cargo test history::` filters
// here). Real SQLite in tempdirs throughout, opened via `super::db::open` so
// the per-connection `foreign_keys` PRAGMA trap is exercised for real
// (CLAUDE.md §3 — no mocks; no inline `PRAGMA foreign_keys=ON` in these
// tests, or the FK-off trap could never fail loudly).
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn open_test_db(tmp: &std::path::Path) -> Connection {
        super::super::db::open(&tmp.join("whisperspree.db")).expect("db::open a fresh test DB")
    }

    fn sample_dictation(id: &str, created_at: &str) -> DictationRow {
        DictationRow {
            id: id.to_string(),
            created_at: created_at.to_string(),
            app_bundle_id: Some("com.apple.Notes".to_string()),
            app_name: Some("Notes".to_string()),
            style_id: "default".to_string(),
            engine: "local".to_string(),
            language: Some("en".to_string()),
            duration_ms: 1234,
            raw_transcript: "raw text".to_string(),
            processed_text: "processed text".to_string(),
            persona_id: "clean".to_string(),
            fallback_used: false,
            inject_method: Some("type".to_string()),
            audio_path: None,
            word_timings: None,
        }
    }

    /// AC: §9.1 basic round trip — a row inserted via `insert_dictation` is
    /// readable back, byte-for-byte, via `get_dictation`.
    #[test]
    fn insert_and_get_dictation_roundtrip() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let conn = open_test_db(tmp.path());

        let row = sample_dictation("dict-1", "2026-01-01T00:00:00Z");
        insert_dictation(&conn, &row).expect("insert_dictation");

        let fetched = get_dictation(&conn, "dict-1")
            .expect("get_dictation")
            .expect("row must exist after insert");
        assert_eq!(fetched, row);

        assert!(get_dictation(&conn, "does-not-exist")
            .expect("get_dictation on a missing id must not error")
            .is_none());
    }

    /// AC: §8.1 `dictation_transforms.dictation_id … ON DELETE CASCADE` — the
    /// highest-value test in T0.4 (plan risk #1). Opens the connection
    /// EXCLUSIVELY via `db::open` (never sets the PRAGMA itself): if `db::open`
    /// ever regresses to not re-applying `foreign_keys=ON` per-connection,
    /// this test fails loudly instead of silently leaking orphaned transform
    /// rows in production.
    #[test]
    fn fk_cascade_deletes_transforms() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let conn = open_test_db(tmp.path());

        let dictation = sample_dictation("dict-fk", "2026-01-01T00:00:00Z");
        insert_dictation(&conn, &dictation).expect("insert_dictation");

        let transform = TransformRow {
            id: "transform-1".to_string(),
            dictation_id: "dict-fk".to_string(),
            template_id: "email".to_string(),
            created_at: "2026-01-01T00:01:00Z".to_string(),
            output_text: "transformed output".to_string(),
        };
        insert_transform(&conn, &transform).expect("insert_transform");
        assert_eq!(
            list_transforms(&conn, "dict-fk")
                .expect("list_transforms before delete")
                .len(),
            1
        );

        delete_dictation(&conn, "dict-fk").expect("delete_dictation");

        let remaining = list_transforms(&conn, "dict-fk").expect("list_transforms after delete");
        assert!(
            remaining.is_empty(),
            "ON DELETE CASCADE must remove dictation_transforms when the parent dictation is \
             deleted (requires PRAGMA foreign_keys=ON per db::open); got {remaining:?}"
        );
    }

    /// AC: §9.1 `insert_transform` / `list_transforms` — the transform-table
    /// analogue of the four lookup-table CRUD round-trips (transforms have no
    /// update/delete API of their own; they are removed only via the parent
    /// dictation's cascade, proven separately by `fk_cascade_deletes_transforms`).
    #[test]
    fn transform_insert_and_list_roundtrip() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let conn = open_test_db(tmp.path());
        insert_dictation(&conn, &sample_dictation("dict-t", "2026-01-01T00:00:00Z"))
            .expect("insert_dictation");

        let t1 = TransformRow {
            id: "t1".to_string(),
            dictation_id: "dict-t".to_string(),
            template_id: "summary".to_string(),
            created_at: "2026-01-01T00:02:00Z".to_string(),
            output_text: "summary output".to_string(),
        };
        let t2 = TransformRow {
            id: "t2".to_string(),
            dictation_id: "dict-t".to_string(),
            template_id: "email".to_string(),
            created_at: "2026-01-01T00:03:00Z".to_string(),
            output_text: "email output".to_string(),
        };
        insert_transform(&conn, &t1).expect("insert t1");
        insert_transform(&conn, &t2).expect("insert t2");

        let mut got = list_transforms(&conn, "dict-t").expect("list_transforms");
        got.sort_by(|a, b| a.id.cmp(&b.id));
        assert_eq!(got, vec![t1, t2]);
    }

    /// AC: §9.1 `list_dictations` — the `LIKE` search matches a term present
    /// ONLY in `raw_transcript` and, separately, a term present ONLY in
    /// `processed_text` (both columns are searched, not just one).
    #[test]
    fn list_dictations_like_searches_both_columns() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let conn = open_test_db(tmp.path());

        let mut raw_only = sample_dictation("dict-raw", "2026-01-01T00:00:00Z");
        raw_only.raw_transcript = "the quick zebra jumps".to_string();
        raw_only.processed_text = "an unrelated sentence".to_string();
        insert_dictation(&conn, &raw_only).expect("insert raw_only");

        let mut processed_only = sample_dictation("dict-processed", "2026-01-01T00:01:00Z");
        processed_only.raw_transcript = "totally different words".to_string();
        processed_only.processed_text = "a polished narwhal appears".to_string();
        insert_dictation(&conn, &processed_only).expect("insert processed_only");

        let via_raw = list_dictations(&conn, Some("zebra"), 50, None).expect("search raw");
        assert_eq!(via_raw.len(), 1);
        assert_eq!(via_raw[0].id, "dict-raw");

        let via_processed =
            list_dictations(&conn, Some("narwhal"), 50, None).expect("search processed");
        assert_eq!(via_processed.len(), 1);
        assert_eq!(via_processed[0].id, "dict-processed");
    }

    /// AC: security review S2 — `list_dictations`'s `q` search must treat SQL
    /// LIKE wildcards (`%`, `_`) present in the user's search term as LITERAL
    /// characters, not as pattern metacharacters. Today `format!("%{term}%")`
    /// with `term = "%"` builds the pattern `"%%%"`, which SQLite collapses to
    /// "match anything" — searching for a literal `%` must only match rows
    /// that actually contain one.
    #[test]
    fn list_dictations_treats_percent_as_literal() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let conn = open_test_db(tmp.path());

        let mut has_percent = sample_dictation("dict-percent", "2026-01-01T00:00:00Z");
        has_percent.raw_transcript = "fifty% off today".to_string();
        insert_dictation(&conn, &has_percent).expect("insert row with a literal percent");

        let mut no_percent = sample_dictation("dict-no-percent", "2026-01-01T00:01:00Z");
        no_percent.raw_transcript = "totally unrelated text".to_string();
        insert_dictation(&conn, &no_percent).expect("insert row without a percent");

        let results = list_dictations(&conn, Some("%"), 50, None).expect("search for a literal %");
        assert_eq!(
            results.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
            vec!["dict-percent"],
            "searching for a literal '%' must only match rows that actually contain '%', not \
             act as an unescaped SQL LIKE wildcard matching every row"
        );
    }

    /// AC: security review S3 — SQLite treats `LIMIT -1` (and any negative
    /// value) as "no limit", silently returning the entire table. `limit`
    /// must be clamped to a sane bound rather than passed through verbatim.
    #[test]
    fn list_dictations_clamps_nonpositive_limit() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let conn = open_test_db(tmp.path());

        let total = 60;
        for i in 0..total {
            let id = format!("dict-{i:03}");
            let ts = format!("2026-01-01T{:02}:{:02}:00Z", i / 60, i % 60);
            insert_dictation(&conn, &sample_dictation(&id, &ts)).expect("insert");
        }

        let via_negative = list_dictations(&conn, None, -1, None).expect("list with limit = -1");
        assert!(
            via_negative.len() < total,
            "a negative limit must be clamped to a sane bound, not treated as SQLite's own \
             \"unlimited\" LIMIT -1 semantics; got {} of {total} rows",
            via_negative.len()
        );

        let via_zero = list_dictations(&conn, None, 0, None).expect("list with limit = 0");
        assert!(
            via_zero.len() < total,
            "a zero limit must not return the full table either; got {} of {total} rows",
            via_zero.len()
        );
    }

    /// AC: §9.1 `list_dictations` — `limit` bounds the page size and
    /// `before_id` keyset-pages strictly older (by `created_at`) than that
    /// row, newest-first overall (`idx_dictations_created`).
    #[test]
    fn list_dictations_limit_and_before_id_paginate() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let conn = open_test_db(tmp.path());

        for (id, ts) in [
            ("d1", "2026-01-01T00:00:01Z"),
            ("d2", "2026-01-01T00:00:02Z"),
            ("d3", "2026-01-01T00:00:03Z"),
            ("d4", "2026-01-01T00:00:04Z"),
        ] {
            insert_dictation(&conn, &sample_dictation(id, ts)).expect("insert");
        }

        let first_page = list_dictations(&conn, None, 2, None).expect("first page");
        assert_eq!(
            first_page.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
            vec!["d4", "d3"],
            "newest-first, limited to 2"
        );

        let second_page =
            list_dictations(&conn, None, 2, Some("d3")).expect("second page, before d3");
        assert_eq!(
            second_page
                .iter()
                .map(|r| r.id.as_str())
                .collect::<Vec<_>>(),
            vec!["d2", "d1"],
            "keyset page strictly before d3's created_at"
        );
    }

    /// AC: §9.1 `delete_dictation` / §12 P-6 — returns the deleted row's
    /// `audio_path` so the caller can unlink it; `None` for a row with no
    /// saved audio, and `None` (not an error) for a missing id.
    #[test]
    fn delete_dictation_returns_audio_path() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let conn = open_test_db(tmp.path());

        let mut with_audio = sample_dictation("dict-audio", "2026-01-01T00:00:00Z");
        with_audio.audio_path = Some("/tmp/whisperspree-test/dict-audio.wav".to_string());
        insert_dictation(&conn, &with_audio).expect("insert with audio");

        let path = delete_dictation(&conn, "dict-audio")
            .expect("delete_dictation")
            .expect("row had an audio_path");
        assert_eq!(path, PathBuf::from("/tmp/whisperspree-test/dict-audio.wav"));
        assert!(get_dictation(&conn, "dict-audio")
            .expect("get after delete")
            .is_none());

        let no_audio = sample_dictation("dict-no-audio", "2026-01-01T00:01:00Z");
        insert_dictation(&conn, &no_audio).expect("insert without audio");
        assert!(delete_dictation(&conn, "dict-no-audio")
            .expect("delete_dictation without audio")
            .is_none());

        assert!(delete_dictation(&conn, "never-existed")
            .expect("deleting a missing id must not error")
            .is_none());
    }

    /// AC: §9.1 `clear_history` / §12 P-6 — every non-null `audio_path`
    /// across all rows is returned, and every `dictations` row is gone.
    #[test]
    fn clear_history_returns_all_audio_paths() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let conn = open_test_db(tmp.path());

        let mut a = sample_dictation("dict-a", "2026-01-01T00:00:00Z");
        a.audio_path = Some("/tmp/whisperspree-test/a.wav".to_string());
        let mut b = sample_dictation("dict-b", "2026-01-01T00:01:00Z");
        b.audio_path = Some("/tmp/whisperspree-test/b.wav".to_string());
        let c = sample_dictation("dict-c", "2026-01-01T00:02:00Z"); // no audio
        insert_dictation(&conn, &a).expect("insert a");
        insert_dictation(&conn, &b).expect("insert b");
        insert_dictation(&conn, &c).expect("insert c");

        let mut paths = clear_history(&conn).expect("clear_history");
        paths.sort();
        assert_eq!(
            paths,
            vec![
                PathBuf::from("/tmp/whisperspree-test/a.wav"),
                PathBuf::from("/tmp/whisperspree-test/b.wav"),
            ]
        );

        assert!(list_dictations(&conn, None, 50, None)
            .expect("list after clear")
            .is_empty());
    }

    /// AC: §9.1 `export_history` — writes exactly N JSON-lines for N rows,
    /// each line independently valid JSON.
    #[test]
    fn export_history_writes_json_lines() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let conn = open_test_db(tmp.path());

        for (id, ts) in [
            ("e1", "2026-01-01T00:00:00Z"),
            ("e2", "2026-01-01T00:01:00Z"),
            ("e3", "2026-01-01T00:02:00Z"),
        ] {
            insert_dictation(&conn, &sample_dictation(id, ts)).expect("insert");
        }

        let mut buf: Vec<u8> = Vec::new();
        let count = export_history(&conn, &mut buf).expect("export_history");
        assert_eq!(count, 3);

        let text = String::from_utf8(buf).expect("export_history output must be UTF-8");
        let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
        assert_eq!(lines.len(), 3, "must write exactly one JSON line per row");
        for line in lines {
            let _: serde_json::Value =
                serde_json::from_str(line).expect("each export_history line must be valid JSON");
        }
    }

    /// AC: §12 P-6 / security review B1 (BLOCKING) — `retention_sweep` must
    /// never unlink a file OUTSIDE the app's own `audio/` directory (§4.4:
    /// `<app-data>/audio/`), even though only app code writes
    /// `dictations.audio_path` today. This runs unattended on launch and
    /// every 6h; an unattended deletion primitive must be safe by
    /// construction before T6.2 starts writing real paths. Confinement must
    /// NOT block the DB row itself from being cleaned up.
    #[test]
    fn retention_never_deletes_file_outside_audio_dir() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let conn = open_test_db(tmp.path());

        // A completely separate tempdir — never a descendant of `<tmp>/audio`
        // — referenced by its absolute path, exactly the shape a corrupted or
        // future-buggy write could leave in `audio_path`.
        let outside_dir = tempfile::tempdir().expect("outside tempdir");
        let outside_file = outside_dir.path().join("not_app_audio.wav");
        std::fs::write(&outside_file, b"outside-app-audio-dir").expect("write outside file");
        assert!(outside_file.exists(), "precondition: outside file exists");

        let mut old_row = sample_dictation("dict-outside-audio", "2000-01-01T00:00:00Z");
        old_row.audio_path = Some(outside_file.to_string_lossy().into_owned());
        insert_dictation(&conn, &old_row).expect("insert expired row pointing outside audio_dir");

        retention_sweep(&conn, 30).expect("retention_sweep");

        assert!(
            outside_file.exists(),
            "retention_sweep must never unlink a file outside the app's own audio directory \
             (§4.4), even for an expired dictation row"
        );
        assert!(
            get_dictation(&conn, "dict-outside-audio")
                .expect("get after sweep")
                .is_none(),
            "the DB row itself must still be deleted — confinement must not block row cleanup"
        );
    }

    /// AC: §12 P-6 / security review B1 (BLOCKING) — same confinement
    /// guarantee, but via a relative `../`-style path that escapes the audio
    /// directory rather than an unrelated absolute path.
    #[test]
    fn retention_never_deletes_via_parent_traversal() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let conn = open_test_db(tmp.path());
        let audio_dir = tmp.path().join("audio");
        std::fs::create_dir_all(&audio_dir).expect("create audio dir");

        // A sibling directory to `audio/`, reached via a `../` escape rooted
        // AT the audio dir — exactly the shape a naive "starts_with(audio_dir)"
        // string check (without canonicalization) would still be fooled by.
        let sibling_dir = tmp.path().join("not_audio");
        std::fs::create_dir_all(&sibling_dir).expect("create sibling dir");
        let escaped_file = sibling_dir.join("secret.wav");
        std::fs::write(&escaped_file, b"escaped-via-traversal").expect("write escaped file");
        assert!(escaped_file.exists(), "precondition: escaped file exists");

        let traversal_path = audio_dir.join("../not_audio/secret.wav");

        let mut old_row = sample_dictation("dict-traversal", "2000-01-01T00:00:00Z");
        old_row.audio_path = Some(traversal_path.to_string_lossy().into_owned());
        insert_dictation(&conn, &old_row).expect("insert expired row with a traversal audio_path");

        retention_sweep(&conn, 30).expect("retention_sweep");

        assert!(
            escaped_file.exists(),
            "retention_sweep must never unlink a file reached via `../` traversal out of the \
             audio directory"
        );
        assert!(
            get_dictation(&conn, "dict-traversal")
                .expect("get after sweep")
                .is_none(),
            "the DB row itself must still be deleted — confinement must not block row cleanup"
        );
    }

    /// AC: security review B1 positive case — a file genuinely under the
    /// app's own `audio/` directory IS unlinked, so the confinement guard
    /// added for the two tests above is not simply refusing everything.
    #[test]
    fn retention_unlinks_audio_inside_audio_dir() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let conn = open_test_db(tmp.path());
        let audio_dir = tmp.path().join("audio");
        std::fs::create_dir_all(&audio_dir).expect("create audio dir");

        let inside_file = audio_dir.join("dict-inside.wav");
        std::fs::write(&inside_file, b"inside-app-audio-dir").expect("write inside file");
        assert!(inside_file.exists(), "precondition: inside file exists");

        let mut old_row = sample_dictation("dict-inside-audio", "2000-01-01T00:00:00Z");
        old_row.audio_path = Some(inside_file.to_string_lossy().into_owned());
        insert_dictation(&conn, &old_row).expect("insert expired row pointing inside audio_dir");

        retention_sweep(&conn, 30).expect("retention_sweep");

        assert!(
            !inside_file.exists(),
            "a file genuinely under the app's own audio directory must still be unlinked — the \
             confinement guard must not refuse everything"
        );
    }

    /// AC: §8.1 retention job — dictations older than the cutoff are deleted
    /// and their audio unlinked from REAL disk; dictations at/after the cutoff
    /// survive untouched.
    #[test]
    fn retention_deletes_old_rows_and_returns_audio() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let conn = open_test_db(tmp.path());

        let audio_dir = tmp.path().join("audio");
        std::fs::create_dir_all(&audio_dir).expect("create audio dir");
        let old_audio_path = audio_dir.join("old.wav");
        std::fs::write(&old_audio_path, b"RIFF-fake-wav-bytes")
            .expect("write real temp audio file");
        assert!(
            old_audio_path.exists(),
            "precondition: temp audio file exists on disk"
        );

        // `retention_sweep` computes its cutoff from `Utc::now() - retention_days`;
        // an old row dated far in the past and a recent row dated "now" straddle
        // any reasonable retention_days value, so this stays deterministic without
        // needing to mock the clock.
        let mut old_row = sample_dictation("dict-old", "2000-01-01T00:00:00Z");
        old_row.audio_path = Some(old_audio_path.to_string_lossy().into_owned());
        insert_dictation(&conn, &old_row).expect("insert old row");

        let recent_row = sample_dictation("dict-recent", &chrono_now_iso8601_for_test());
        insert_dictation(&conn, &recent_row).expect("insert recent row");

        let unlinked = retention_sweep(&conn, 30).expect("retention_sweep");
        assert_eq!(unlinked, vec![old_audio_path.clone()]);
        assert!(
            !old_audio_path.exists(),
            "retention_sweep must actually unlink the audio file from disk"
        );

        assert!(get_dictation(&conn, "dict-old").expect("get old").is_none());
        assert!(
            get_dictation(&conn, "dict-recent")
                .expect("get recent")
                .is_some(),
            "a recent dictation must survive the sweep"
        );
    }

    /// Minimal, dependency-free "now" ISO-8601 string for the retention test
    /// above — avoids pulling `chrono` into the test-only path merely to prove
    /// "recent survives, ancient doesn't"; any date far enough in the future of
    /// `2000-01-01` and not older than 30 days from the real wall clock works
    /// only if this literal tracks "now" year, so we compute it from
    /// `std::time::SystemTime` instead of hardcoding a year that will go stale.
    ///
    /// Calendar math (`civil_from_days`) is NOT duplicated here — it reuses
    /// `super::civil_from_days`, the same `#[cfg(test)]` calendar helper
    /// `iso8601_from_unix_seconds` uses, so there is exactly one copy of this
    /// date arithmetic in the crate (code review NIT: two independent copies
    /// could drift, and drift in calendar math is the kind of bug that only
    /// surfaces once a year, on a leap day).
    fn chrono_now_iso8601_for_test() -> String {
        use std::time::{SystemTime, UNIX_EPOCH};
        let secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before epoch")
            .as_secs();
        // Days since epoch, converted with a plain proleptic-Gregorian calc so we
        // don't need a date-time crate just for this test fixture.
        let days = secs / 86_400;
        let (y, m, d) = super::civil_from_days(days as i64);
        format!("{y:04}-{m:02}-{d:02}T00:00:00Z")
    }

    /// AC: §12 privacy / code-review SHOULD-FIX — `retention_sweep` compares
    /// `created_at < cutoff` LEXICOGRAPHICALLY. A row 500ms AFTER the exact
    /// cutoff instant (same whole second, plus a fractional suffix) is
    /// chronologically NEWER than the cutoff and must survive. A naive
    /// string compare puts it BEFORE the whole-second cutoff string, because
    /// `.` (0x2E) sorts below `Z` (0x5A) at the position where the two
    /// strings diverge — silently deleting a row that is not actually old.
    #[test]
    fn retention_keeps_row_with_fractional_seconds_newer_than_cutoff() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let conn = open_test_db(tmp.path());

        let retention_days: u32 = 30;
        // This test needs the row to land on the EXACT SAME whole second as
        // the cutoff `retention_sweep` computes for itself a moment later —
        // that is the only scenario where a naive lexicographic string
        // compare and a correct numeric-parse-then-`<` compare can disagree
        // (any gap at all makes an earlier date/time digit resolve the
        // ordering before either code path ever reaches the fractional
        // suffix, which would hide the very bug this test exists to catch —
        // see the assertion message below). So the "comfortable margin"
        // fix belongs on the TIME AXIS, not the row-vs-cutoff gap: instead of
        // reading "now" at an arbitrary instant (which could be a few
        // microseconds before the clock ticks over — the actual source of
        // the theoretical flake), wait for a FRESH tick first. That buys up
        // to a full second of headroom before the NEXT tick, so
        // `retention_sweep`'s own `unix_seconds_now()` call — made a moment
        // later, doing nothing more than one SQLite SELECT — is
        // overwhelmingly certain to land in the same whole second we just
        // observed, without weakening the exact-tie the test relies on.
        let now = wait_for_fresh_second_boundary();
        let cutoff_secs = now - i64::from(retention_days) * 86_400;
        let cutoff = iso8601_from_unix_seconds(cutoff_secs);
        assert!(cutoff.ends_with('Z'), "unexpected cutoff format: {cutoff}");
        // Same whole second as the cutoff, differing only in a trailing
        // fractional suffix `parse_iso8601_utc_seconds` must truncate away.
        let fractional_but_newer = format!("{}.500Z", &cutoff[..cutoff.len() - 1]);

        insert_dictation(
            &conn,
            &sample_dictation("dict-frac-newer", &fractional_but_newer),
        )
        .expect("insert a row at the exact cutoff second, with a fractional-second created_at");

        retention_sweep(&conn, retention_days).expect("retention_sweep");

        assert!(
            get_dictation(&conn, "dict-frac-newer")
                .expect("get after sweep")
                .is_some(),
            "a row at the exact cutoff second must survive retention_sweep: truncating its \
             fractional seconds and comparing with a strict `<` (not `<=`) must not treat it as \
             older, and a lexicographic string compare must not treat '.' < 'Z' as \"older\" \
             either"
        );
    }

    /// Spin until `unix_seconds_now()` ticks over to a NEW whole second, then
    /// return that value. Reading "now" at an arbitrary instant risks landing
    /// a few microseconds before the clock rolls over to the next second —
    /// the actual (tiny but nonzero) source of flakiness in
    /// `retention_keeps_row_with_fractional_seconds_newer_than_cutoff`, which
    /// needs its caller's "now" and `retention_sweep`'s own later
    /// `unix_seconds_now()` call to land in the same whole second. Waiting
    /// for a fresh tick instead buys close to a full second of headroom
    /// before the next one, which comfortably exceeds the sub-millisecond gap
    /// between the two calls (a SQLite insert plus a handful of function
    /// calls) — eliminating the race in practice without touching the
    /// row-vs-cutoff relationship the test depends on.
    fn wait_for_fresh_second_boundary() -> i64 {
        let start = unix_seconds_now();
        loop {
            let now = unix_seconds_now();
            if now != start {
                return now;
            }
            std::thread::yield_now();
        }
    }

    /// AC: §12 privacy / code-review SHOULD-FIX — a row whose `created_at` is
    /// empty or otherwise unparseable as a timestamp must never be deleted:
    /// retention must be conservative and never destroy data it cannot
    /// confidently prove is old. An empty string sorts before every
    /// non-empty cutoff string lexicographically, so the naive compare
    /// deletes it unconditionally.
    #[test]
    fn retention_keeps_row_with_malformed_created_at() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let conn = open_test_db(tmp.path());

        insert_dictation(&conn, &sample_dictation("dict-malformed", ""))
            .expect("insert a row with an empty/malformed created_at");

        retention_sweep(&conn, 30).expect("retention_sweep");

        assert!(
            get_dictation(&conn, "dict-malformed")
                .expect("get after sweep")
                .is_some(),
            "a row with an unparseable created_at must never be deleted by retention_sweep — \
             we must never destroy data we cannot confidently prove is old (§12 privacy)"
        );
    }

    /// AC: guards the fix above against over-strictness — a row that IS
    /// genuinely older than the cutoff, and also happens to carry fractional
    /// seconds, must still be deleted. The fix must be date-aware (correctly
    /// parse/compare), not "skip anything with a `.` in created_at".
    #[test]
    fn retention_still_deletes_genuinely_old_row_with_fractional_seconds() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let conn = open_test_db(tmp.path());

        insert_dictation(
            &conn,
            &sample_dictation("dict-frac-old", "2000-01-01T00:00:00.123Z"),
        )
        .expect("insert a genuinely ancient row with fractional-second created_at");

        retention_sweep(&conn, 30).expect("retention_sweep");

        assert!(
            get_dictation(&conn, "dict-frac-old")
                .expect("get after sweep")
                .is_none(),
            "a genuinely old row must still be deleted even though its created_at carries \
             fractional seconds — the fix must not become \"never delete anything unusual\""
        );
    }

    /// AC: OPEN_QUESTIONS Q11 — `retention_sweep(_, 0)` is a no-op: nothing is
    /// deleted, regardless of age.
    #[test]
    fn retention_days_zero_is_noop() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let conn = open_test_db(tmp.path());

        insert_dictation(
            &conn,
            &sample_dictation("dict-ancient", "1990-01-01T00:00:00Z"),
        )
        .expect("insert ancient row");

        let unlinked = retention_sweep(&conn, 0).expect("retention_sweep with 0");
        assert!(
            unlinked.is_empty(),
            "retention_days=0 must delete nothing (OPEN_QUESTIONS Q11)"
        );
        assert!(
            get_dictation(&conn, "dict-ancient")
                .expect("get ancient")
                .is_some(),
            "retention_days=0 must not delete even an ancient row"
        );
    }
}
