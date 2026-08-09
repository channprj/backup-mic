BEGIN IMMEDIATE;

ALTER TABLE backup_runs ADD COLUMN batch_phase TEXT NOT NULL DEFAULT 'completed'
    CHECK (batch_phase IN (
        'inventory', 'copying', 'copies_verified', 'converting',
        'm4a_cohort_verified', 'sources_revalidated', 'completed', 'failed'
    ));
ALTER TABLE backup_runs ADD COLUMN frozen_automatic_backup INTEGER NOT NULL DEFAULT 1
    CHECK (frozen_automatic_backup IN (0, 1));
ALTER TABLE backup_runs ADD COLUMN frozen_m4a_conversion INTEGER NOT NULL DEFAULT 1
    CHECK (frozen_m4a_conversion IN (0, 1));
ALTER TABLE backup_runs ADD COLUMN frozen_automatic_trash INTEGER NOT NULL DEFAULT 0
    CHECK (frozen_automatic_trash IN (0, 1));
ALTER TABLE backup_runs ADD COLUMN m4a_profile_id TEXT;

ALTER TABLE recordings ADD COLUMN superseded_wav_relative_path TEXT;
ALTER TABLE recordings ADD COLUMN superseded_wav_size INTEGER
    CHECK (superseded_wav_size IS NULL OR superseded_wav_size >= 0);
ALTER TABLE recordings ADD COLUMN superseded_wav_sha256 TEXT;

CREATE TABLE additional_files (
    id TEXT PRIMARY KEY,
    transmitter TEXT NOT NULL CHECK (transmitter IN ('TX01', 'TX02')),
    source_relative_path TEXT NOT NULL,
    source_size INTEGER NOT NULL CHECK (source_size >= 0),
    source_mtime_ns TEXT NOT NULL,
    source_sha256 TEXT NOT NULL,
    artifact_relative_path TEXT NOT NULL UNIQUE,
    artifact_size INTEGER NOT NULL CHECK (artifact_size >= 0),
    artifact_sha256 TEXT NOT NULL,
    classification TEXT NOT NULL CHECK (classification IN ('m4a', 'apple_double', 'other')),
    backup_run_id TEXT NOT NULL REFERENCES backup_runs(id),
    UNIQUE (transmitter, source_relative_path, source_size, source_mtime_ns, source_sha256)
);

CREATE TABLE conversion_cohort_items (
    backup_run_id TEXT NOT NULL REFERENCES backup_runs(id),
    recording_id TEXT NOT NULL REFERENCES recordings(id),
    profile_id TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('pending', 'verified_m4a', 'failed')),
    error_code TEXT,
    PRIMARY KEY (backup_run_id, recording_id)
);

CREATE TABLE additional_deletion_items (
    deletion_run_id TEXT NOT NULL REFERENCES deletion_runs(id),
    additional_file_id TEXT NOT NULL REFERENCES additional_files(id),
    outcome TEXT NOT NULL,
    removed_at TEXT,
    error_code TEXT,
    PRIMARY KEY (deletion_run_id, additional_file_id)
);

INSERT INTO schema_migrations(version, applied_at)
VALUES (3, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'));

COMMIT;
