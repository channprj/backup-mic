PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS schema_migrations (
    version INTEGER PRIMARY KEY,
    applied_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS paired_devices (
    transmitter TEXT PRIMARY KEY CHECK (transmitter IN ('TX01', 'TX02')),
    volume_uuid TEXT NOT NULL UNIQUE,
    protocol TEXT NOT NULL,
    media_name TEXT NOT NULL,
    nominal_capacity INTEGER NOT NULL CHECK (nominal_capacity >= 0),
    paired_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS settings (
    key TEXT PRIMARY KEY,
    value_json TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS backup_runs (
    id TEXT PRIMARY KEY,
    started_at TEXT NOT NULL,
    finished_at TEXT,
    outcome TEXT NOT NULL,
    required_copy_bytes INTEGER NOT NULL CHECK (required_copy_bytes >= 0),
    error_code TEXT
);

CREATE TABLE IF NOT EXISTS recordings (
    id TEXT PRIMARY KEY,
    transmitter TEXT NOT NULL CHECK (transmitter IN ('TX01', 'TX02')),
    source_relative_path TEXT NOT NULL,
    source_size INTEGER NOT NULL CHECK (source_size >= 0),
    source_mtime_ns TEXT NOT NULL,
    source_sha256 TEXT NOT NULL,
    destination_relative_path TEXT NOT NULL,
    destination_size INTEGER NOT NULL CHECK (destination_size >= 0),
    destination_sha256 TEXT NOT NULL,
    verified_at TEXT NOT NULL,
    source_deleted_at TEXT,
    deletion_error_code TEXT,
    backup_run_id TEXT NOT NULL REFERENCES backup_runs(id),
    UNIQUE (transmitter, source_relative_path, source_size, source_mtime_ns, source_sha256),
    UNIQUE (destination_relative_path)
);

CREATE TABLE IF NOT EXISTS deletion_runs (
    id TEXT PRIMARY KEY,
    transmitter TEXT NOT NULL CHECK (transmitter IN ('TX01', 'TX02')),
    started_at TEXT NOT NULL,
    finished_at TEXT,
    outcome TEXT NOT NULL,
    proposed_file_count INTEGER NOT NULL CHECK (proposed_file_count >= 0),
    proposed_bytes INTEGER NOT NULL CHECK (proposed_bytes >= 0),
    error_code TEXT
);

CREATE TABLE IF NOT EXISTS deletion_items (
    deletion_run_id TEXT NOT NULL REFERENCES deletion_runs(id),
    recording_id TEXT NOT NULL REFERENCES recordings(id),
    outcome TEXT NOT NULL,
    removed_at TEXT,
    error_code TEXT,
    PRIMARY KEY (deletion_run_id, recording_id)
);

CREATE TABLE IF NOT EXISTS activity (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    occurred_at TEXT NOT NULL,
    code TEXT NOT NULL,
    transmitter TEXT CHECK (transmitter IN ('TX01', 'TX02')),
    count_value INTEGER CHECK (count_value >= 0),
    byte_value INTEGER CHECK (byte_value >= 0),
    severity TEXT NOT NULL
);

INSERT OR IGNORE INTO schema_migrations(version, applied_at)
VALUES (1, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'));
