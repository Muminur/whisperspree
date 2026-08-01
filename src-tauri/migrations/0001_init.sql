PRAGMA journal_mode = WAL;
PRAGMA foreign_keys = ON;

CREATE TABLE schema_version (version INTEGER NOT NULL);
INSERT INTO schema_version VALUES (1);

CREATE TABLE dictations (
  id             TEXT PRIMARY KEY,            -- uuid v4
  created_at     TEXT NOT NULL,               -- ISO-8601 UTC
  app_bundle_id  TEXT,
  app_name       TEXT,
  style_id       TEXT NOT NULL,               -- resolved AppStyle
  engine         TEXT NOT NULL CHECK (engine IN ('cloud','local')),
  language       TEXT,                        -- detected/forced ISO code
  duration_ms    INTEGER NOT NULL,
  raw_transcript TEXT NOT NULL,
  processed_text TEXT NOT NULL,
  persona_id     TEXT NOT NULL DEFAULT 'clean',
  fallback_used  INTEGER NOT NULL DEFAULT 0,  -- FR-2.7 ran
  inject_method  TEXT,                        -- type|paste|clipboard_only|none
  audio_path     TEXT,                        -- NULL unless saveAudio
  word_timings   TEXT                         -- JSON [{w,s,e}] ms, NULL if unavailable
);
CREATE INDEX idx_dictations_created ON dictations(created_at DESC);

CREATE TABLE dictation_transforms (
  id           TEXT PRIMARY KEY,
  dictation_id TEXT NOT NULL REFERENCES dictations(id) ON DELETE CASCADE,
  template_id  TEXT NOT NULL,
  created_at   TEXT NOT NULL,
  output_text  TEXT NOT NULL
);

CREATE TABLE dictionary_entries (
  id             TEXT PRIMARY KEY,
  phrase         TEXT NOT NULL UNIQUE,
  sounds_like    TEXT NOT NULL DEFAULT '[]',  -- JSON string array
  case_sensitive INTEGER NOT NULL DEFAULT 0,
  created_at     TEXT NOT NULL,
  last_used_at   TEXT
);

CREATE TABLE snippets (
  id          TEXT PRIMARY KEY,
  trigger     TEXT NOT NULL UNIQUE,           -- stored lowercase
  content     TEXT NOT NULL,
  created_at  TEXT NOT NULL,
  usage_count INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE custom_prompts (
  id          TEXT PRIMARY KEY,
  name        TEXT NOT NULL,
  prompt_text TEXT NOT NULL,
  created_at  TEXT NOT NULL
);

CREATE TABLE app_rules (
  id               TEXT PRIMARY KEY,
  bundle_id        TEXT NOT NULL,
  title_regex      TEXT,                      -- optional
  style_id         TEXT,                      -- one of §7.2 ids
  persona_id       TEXT,                      -- one of §7.3 ids
  custom_prompt_id TEXT REFERENCES custom_prompts(id) ON DELETE SET NULL,
  priority         INTEGER NOT NULL DEFAULT 0 -- higher wins
);
