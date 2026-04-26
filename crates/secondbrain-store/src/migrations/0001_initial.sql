-- Initial schema for secondbrain.
-- See PLAN.md for design rationale.
-- WAL + foreign_keys are set on the connection (lib.rs), not here:
-- sqlx wraps migrations in a transaction and `journal_mode = WAL` cannot
-- be changed inside one.

CREATE TABLE segments (
    id            INTEGER PRIMARY KEY,
    started_at    INTEGER NOT NULL,
    ended_at      INTEGER,
    app_bundle    TEXT NOT NULL,
    app_name      TEXT NOT NULL,
    window_title  TEXT,
    url           TEXT,
    monitor_id    INTEGER,
    focused       INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX idx_segments_started_at ON segments (started_at);
CREATE INDEX idx_segments_app_bundle ON segments (app_bundle);

CREATE TABLE extractions (
    id           INTEGER PRIMARY KEY,
    segment_id   INTEGER NOT NULL REFERENCES segments(id) ON DELETE CASCADE,
    captured_at  INTEGER NOT NULL,
    source       TEXT NOT NULL,
    text         TEXT NOT NULL,
    confidence   REAL,
    raw_json     TEXT
);
CREATE INDEX idx_extractions_segment ON extractions (segment_id);
CREATE INDEX idx_extractions_captured_at ON extractions (captured_at);
CREATE INDEX idx_extractions_source ON extractions (source);

CREATE VIRTUAL TABLE extractions_fts USING fts5(
    text,
    content='extractions',
    content_rowid='id'
);
CREATE TRIGGER extractions_fts_insert AFTER INSERT ON extractions BEGIN
    INSERT INTO extractions_fts (rowid, text) VALUES (new.id, new.text);
END;
CREATE TRIGGER extractions_fts_delete AFTER DELETE ON extractions BEGIN
    INSERT INTO extractions_fts (extractions_fts, rowid, text) VALUES ('delete', old.id, old.text);
END;
CREATE TRIGGER extractions_fts_update AFTER UPDATE ON extractions BEGIN
    INSERT INTO extractions_fts (extractions_fts, rowid, text) VALUES ('delete', old.id, old.text);
    INSERT INTO extractions_fts (rowid, text) VALUES (new.id, new.text);
END;

CREATE TABLE image_descriptions (
    id           INTEGER PRIMARY KEY,
    segment_id   INTEGER NOT NULL REFERENCES segments(id) ON DELETE CASCADE,
    captured_at  INTEGER NOT NULL,
    description  TEXT NOT NULL,
    model        TEXT NOT NULL
);
CREATE INDEX idx_image_descriptions_segment ON image_descriptions (segment_id);

CREATE TABLE audio_transcriptions (
    id            INTEGER PRIMARY KEY,
    segment_id    INTEGER NOT NULL REFERENCES segments(id) ON DELETE CASCADE,
    started_at    INTEGER NOT NULL,
    ended_at      INTEGER NOT NULL,
    speaker_hint  TEXT,
    text          TEXT NOT NULL,
    engine        TEXT NOT NULL
);
CREATE INDEX idx_audio_segment ON audio_transcriptions (segment_id);
CREATE INDEX idx_audio_started_at ON audio_transcriptions (started_at);

CREATE TABLE notes (
    id                 INTEGER PRIMARY KEY,
    slug               TEXT UNIQUE NOT NULL,
    title              TEXT NOT NULL,
    body_md            TEXT NOT NULL,
    created_at         INTEGER NOT NULL,
    updated_at         INTEGER NOT NULL,
    source_segment_ids TEXT
);
CREATE INDEX idx_notes_updated_at ON notes (updated_at);

CREATE TABLE note_links (
    src_slug  TEXT NOT NULL,
    dst_slug  TEXT NOT NULL,
    context   TEXT,
    PRIMARY KEY (src_slug, dst_slug)
);
CREATE INDEX idx_note_links_dst ON note_links (dst_slug);

CREATE TABLE entities (
    id    INTEGER PRIMARY KEY,
    kind  TEXT NOT NULL,
    name  TEXT NOT NULL,
    slug  TEXT UNIQUE NOT NULL
);
CREATE INDEX idx_entities_kind ON entities (kind);

CREATE TABLE entity_mentions (
    entity_id  INTEGER NOT NULL REFERENCES entities(id) ON DELETE CASCADE,
    note_slug  TEXT NOT NULL,
    PRIMARY KEY (entity_id, note_slug)
);
CREATE INDEX idx_entity_mentions_note ON entity_mentions (note_slug);

CREATE TABLE extractor_watermarks (
    extractor    TEXT PRIMARY KEY,
    watermark    INTEGER NOT NULL,
    updated_at   INTEGER NOT NULL
);
