PRAGMA foreign_keys = OFF;
BEGIN IMMEDIATE;

CREATE TABLE sources (
    id TEXT PRIMARY KEY,
    rule_id TEXT NOT NULL REFERENCES backup_rules(id) ON DELETE RESTRICT,
    volume_uuid TEXT NOT NULL,
    volume_uuid_normalized TEXT NOT NULL,
    legacy_slot TEXT CHECK (legacy_slot IS NULL OR legacy_slot IN ('TX01', 'TX02')),
    display_name TEXT NOT NULL,
    created_at TEXT NOT NULL,
    last_seen_at TEXT NOT NULL,
    UNIQUE (rule_id, volume_uuid_normalized),
    UNIQUE (rule_id, legacy_slot)
);

INSERT OR IGNORE INTO rule_device_bindings(
    rule_id, volume_uuid, volume_uuid_normalized, protocol, media_name,
    nominal_capacity, slot_label, paired_at
)
SELECT
    '6d784c99-8b0e-4a32-a0a2-d7730f68cf28', volume_uuid, lower(volume_uuid),
    protocol, media_name, nominal_capacity, transmitter, paired_at
FROM paired_devices;

INSERT INTO sources(
    id, rule_id, volume_uuid, volume_uuid_normalized, legacy_slot,
    display_name, created_at, last_seen_at
)
SELECT
    '10da26f2-f143-4e3c-b8fe-464a265755f1',
    '6d784c99-8b0e-4a32-a0a2-d7730f68cf28',
    COALESCE(
        (SELECT volume_uuid FROM paired_devices WHERE transmitter = 'TX01'),
        'legacy-unpaired-tx01'
    ),
    lower(COALESCE(
        (SELECT volume_uuid FROM paired_devices WHERE transmitter = 'TX01'),
        'legacy-unpaired-tx01'
    )),
    'TX01',
    'DJI Mic Mini 2S (TX01)',
    COALESCE(
        (SELECT paired_at FROM paired_devices WHERE transmitter = 'TX01'),
        strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
    ),
    strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
WHERE EXISTS(SELECT 1 FROM paired_devices WHERE transmitter = 'TX01')
   OR EXISTS(SELECT 1 FROM recordings WHERE transmitter = 'TX01')
   OR EXISTS(SELECT 1 FROM additional_files WHERE transmitter = 'TX01')
   OR EXISTS(SELECT 1 FROM activity WHERE transmitter = 'TX01')
   OR EXISTS(SELECT 1 FROM deletion_runs WHERE transmitter = 'TX01');

INSERT INTO sources(
    id, rule_id, volume_uuid, volume_uuid_normalized, legacy_slot,
    display_name, created_at, last_seen_at
)
SELECT
    '20da26f2-f143-4e3c-b8fe-464a265755f2',
    '6d784c99-8b0e-4a32-a0a2-d7730f68cf28',
    COALESCE(
        (SELECT volume_uuid FROM paired_devices WHERE transmitter = 'TX02'),
        'legacy-unpaired-tx02'
    ),
    lower(COALESCE(
        (SELECT volume_uuid FROM paired_devices WHERE transmitter = 'TX02'),
        'legacy-unpaired-tx02'
    )),
    'TX02',
    'DJI Mic Mini 2S (TX02)',
    COALESCE(
        (SELECT paired_at FROM paired_devices WHERE transmitter = 'TX02'),
        strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
    ),
    strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
WHERE EXISTS(SELECT 1 FROM paired_devices WHERE transmitter = 'TX02')
   OR EXISTS(SELECT 1 FROM recordings WHERE transmitter = 'TX02')
   OR EXISTS(SELECT 1 FROM additional_files WHERE transmitter = 'TX02')
   OR EXISTS(SELECT 1 FROM activity WHERE transmitter = 'TX02')
   OR EXISTS(SELECT 1 FROM deletion_runs WHERE transmitter = 'TX02');

ALTER TABLE recordings RENAME TO recordings_v5;
ALTER TABLE additional_files RENAME TO additional_files_v5;
ALTER TABLE deletion_runs RENAME TO deletion_runs_v5;
ALTER TABLE deletion_items RENAME TO deletion_items_v5;
ALTER TABLE conversion_cohort_items RENAME TO conversion_cohort_items_v5;
ALTER TABLE additional_deletion_items RENAME TO additional_deletion_items_v5;

CREATE TABLE recordings (
    id TEXT PRIMARY KEY,
    source_id TEXT NOT NULL REFERENCES sources(id),
    transmitter TEXT CHECK (transmitter IS NULL OR transmitter IN ('TX01', 'TX02')),
    source_relative_path TEXT NOT NULL,
    source_size INTEGER NOT NULL CHECK (source_size >= 0),
    source_mtime_ns TEXT NOT NULL,
    source_sha256 TEXT NOT NULL,
    destination_relative_path TEXT NOT NULL UNIQUE,
    destination_size INTEGER NOT NULL CHECK (destination_size >= 0),
    destination_sha256 TEXT NOT NULL,
    verified_at TEXT NOT NULL,
    source_deleted_at TEXT,
    deletion_error_code TEXT,
    backup_run_id TEXT NOT NULL REFERENCES backup_runs(id),
    artifact_format TEXT NOT NULL DEFAULT 'wav' CHECK (artifact_format IN ('wav', 'm4a')),
    artifact_codec TEXT,
    artifact_sample_rate_hz INTEGER
        CHECK (artifact_sample_rate_hz IS NULL OR artifact_sample_rate_hz > 0),
    artifact_channel_count INTEGER
        CHECK (artifact_channel_count IS NULL OR artifact_channel_count > 0),
    artifact_valid_frames INTEGER
        CHECK (artifact_valid_frames IS NULL OR artifact_valid_frames > 0),
    artifact_duration_micros INTEGER
        CHECK (artifact_duration_micros IS NULL OR artifact_duration_micros > 0),
    conversion_status TEXT NOT NULL DEFAULT 'not_required'
        CHECK (conversion_status IN ('not_required', 'pending', 'complete', 'failed')),
    conversion_error_code TEXT,
    retirement_status TEXT NOT NULL DEFAULT 'present'
        CHECK (retirement_status IN (
            'present', 'trash_pending', 'moved_to_trash', 'legacy_deleted', 'failed'
        )),
    retired_session_relative_path TEXT,
    superseded_wav_relative_path TEXT,
    superseded_wav_size INTEGER
        CHECK (superseded_wav_size IS NULL OR superseded_wav_size >= 0),
    superseded_wav_sha256 TEXT,
    superseded_wav_retirement_status TEXT NOT NULL DEFAULT 'none'
        CHECK (superseded_wav_retirement_status IN (
            'none', 'pending', 'moved_to_trash', 'absent_after_conversion'
        )),
    UNIQUE (source_id, source_relative_path, source_size, source_mtime_ns, source_sha256)
);

INSERT INTO recordings(
    id, source_id, transmitter, source_relative_path, source_size, source_mtime_ns,
    source_sha256, destination_relative_path, destination_size, destination_sha256,
    verified_at, source_deleted_at, deletion_error_code, backup_run_id,
    artifact_format, artifact_codec, artifact_sample_rate_hz, artifact_channel_count,
    artifact_valid_frames, artifact_duration_micros, conversion_status,
    conversion_error_code, retirement_status, retired_session_relative_path,
    superseded_wav_relative_path, superseded_wav_size, superseded_wav_sha256,
    superseded_wav_retirement_status
)
SELECT
    id,
    CASE transmitter
        WHEN 'TX01' THEN '10da26f2-f143-4e3c-b8fe-464a265755f1'
        WHEN 'TX02' THEN '20da26f2-f143-4e3c-b8fe-464a265755f2'
    END,
    transmitter, source_relative_path, source_size, source_mtime_ns, source_sha256,
    destination_relative_path, destination_size, destination_sha256, verified_at,
    source_deleted_at, deletion_error_code, backup_run_id, artifact_format,
    artifact_codec, artifact_sample_rate_hz, artifact_channel_count,
    artifact_valid_frames, artifact_duration_micros, conversion_status,
    conversion_error_code, retirement_status, retired_session_relative_path,
    superseded_wav_relative_path, superseded_wav_size, superseded_wav_sha256,
    superseded_wav_retirement_status
FROM recordings_v5;

CREATE TABLE additional_files (
    id TEXT PRIMARY KEY,
    source_id TEXT NOT NULL REFERENCES sources(id),
    transmitter TEXT CHECK (transmitter IS NULL OR transmitter IN ('TX01', 'TX02')),
    source_relative_path TEXT NOT NULL,
    source_size INTEGER NOT NULL CHECK (source_size >= 0),
    source_mtime_ns TEXT NOT NULL,
    source_sha256 TEXT NOT NULL,
    artifact_relative_path TEXT NOT NULL UNIQUE,
    artifact_size INTEGER NOT NULL CHECK (artifact_size >= 0),
    artifact_sha256 TEXT NOT NULL,
    classification TEXT NOT NULL CHECK (classification IN ('m4a', 'apple_double', 'other')),
    backup_run_id TEXT NOT NULL REFERENCES backup_runs(id),
    UNIQUE (source_id, source_relative_path, source_size, source_mtime_ns, source_sha256)
);

INSERT INTO additional_files(
    id, source_id, transmitter, source_relative_path, source_size, source_mtime_ns,
    source_sha256, artifact_relative_path, artifact_size, artifact_sha256,
    classification, backup_run_id
)
SELECT
    id,
    CASE transmitter
        WHEN 'TX01' THEN '10da26f2-f143-4e3c-b8fe-464a265755f1'
        WHEN 'TX02' THEN '20da26f2-f143-4e3c-b8fe-464a265755f2'
    END,
    transmitter, source_relative_path, source_size, source_mtime_ns, source_sha256,
    artifact_relative_path, artifact_size, artifact_sha256, classification, backup_run_id
FROM additional_files_v5;

CREATE TABLE deletion_runs (
    id TEXT PRIMARY KEY,
    source_id TEXT NOT NULL REFERENCES sources(id),
    transmitter TEXT CHECK (transmitter IS NULL OR transmitter IN ('TX01', 'TX02')),
    started_at TEXT NOT NULL,
    finished_at TEXT,
    outcome TEXT NOT NULL,
    proposed_file_count INTEGER NOT NULL CHECK (proposed_file_count >= 0),
    proposed_bytes INTEGER NOT NULL CHECK (proposed_bytes >= 0),
    error_code TEXT
);

INSERT INTO deletion_runs(
    id, source_id, transmitter, started_at, finished_at, outcome,
    proposed_file_count, proposed_bytes, error_code
)
SELECT
    id,
    CASE transmitter
        WHEN 'TX01' THEN '10da26f2-f143-4e3c-b8fe-464a265755f1'
        WHEN 'TX02' THEN '20da26f2-f143-4e3c-b8fe-464a265755f2'
    END,
    transmitter, started_at, finished_at, outcome, proposed_file_count,
    proposed_bytes, error_code
FROM deletion_runs_v5;

CREATE TABLE deletion_items (
    deletion_run_id TEXT NOT NULL REFERENCES deletion_runs(id),
    recording_id TEXT NOT NULL REFERENCES recordings(id),
    source_id TEXT NOT NULL REFERENCES sources(id),
    outcome TEXT NOT NULL,
    removed_at TEXT,
    error_code TEXT,
    PRIMARY KEY (deletion_run_id, recording_id)
);

INSERT INTO deletion_items(
    deletion_run_id, recording_id, source_id, outcome, removed_at, error_code
)
SELECT item.deletion_run_id, item.recording_id, recording.source_id,
       item.outcome, item.removed_at, item.error_code
FROM deletion_items_v5 AS item
JOIN recordings AS recording ON recording.id = item.recording_id;

CREATE TABLE conversion_cohort_items (
    backup_run_id TEXT NOT NULL REFERENCES backup_runs(id),
    recording_id TEXT NOT NULL REFERENCES recordings(id),
    source_id TEXT NOT NULL REFERENCES sources(id),
    profile_id TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('pending', 'verified_m4a', 'failed')),
    error_code TEXT,
    PRIMARY KEY (backup_run_id, recording_id)
);

INSERT INTO conversion_cohort_items(
    backup_run_id, recording_id, source_id, profile_id, status, error_code
)
SELECT item.backup_run_id, item.recording_id, recording.source_id,
       item.profile_id, item.status, item.error_code
FROM conversion_cohort_items_v5 AS item
JOIN recordings AS recording ON recording.id = item.recording_id;

CREATE TABLE additional_deletion_items (
    deletion_run_id TEXT NOT NULL REFERENCES deletion_runs(id),
    additional_file_id TEXT NOT NULL REFERENCES additional_files(id),
    source_id TEXT NOT NULL REFERENCES sources(id),
    outcome TEXT NOT NULL,
    removed_at TEXT,
    error_code TEXT,
    PRIMARY KEY (deletion_run_id, additional_file_id)
);

INSERT INTO additional_deletion_items(
    deletion_run_id, additional_file_id, source_id, outcome, removed_at, error_code
)
SELECT item.deletion_run_id, item.additional_file_id, file.source_id,
       item.outcome, item.removed_at, item.error_code
FROM additional_deletion_items_v5 AS item
JOIN additional_files AS file ON file.id = item.additional_file_id;

ALTER TABLE backup_runs ADD COLUMN source_id TEXT REFERENCES sources(id);
ALTER TABLE activity ADD COLUMN source_id TEXT REFERENCES sources(id);

UPDATE backup_runs
SET source_id = '10da26f2-f143-4e3c-b8fe-464a265755f1'
WHERE (
        id IN (SELECT backup_run_id FROM recordings WHERE source_id = '10da26f2-f143-4e3c-b8fe-464a265755f1')
        OR id IN (SELECT backup_run_id FROM additional_files WHERE source_id = '10da26f2-f143-4e3c-b8fe-464a265755f1')
      )
  AND id NOT IN (SELECT backup_run_id FROM recordings WHERE source_id = '20da26f2-f143-4e3c-b8fe-464a265755f2')
  AND id NOT IN (SELECT backup_run_id FROM additional_files WHERE source_id = '20da26f2-f143-4e3c-b8fe-464a265755f2');

UPDATE backup_runs
SET source_id = '20da26f2-f143-4e3c-b8fe-464a265755f2'
WHERE (
        id IN (SELECT backup_run_id FROM recordings WHERE source_id = '20da26f2-f143-4e3c-b8fe-464a265755f2')
        OR id IN (SELECT backup_run_id FROM additional_files WHERE source_id = '20da26f2-f143-4e3c-b8fe-464a265755f2')
      )
  AND id NOT IN (SELECT backup_run_id FROM recordings WHERE source_id = '10da26f2-f143-4e3c-b8fe-464a265755f1')
  AND id NOT IN (SELECT backup_run_id FROM additional_files WHERE source_id = '10da26f2-f143-4e3c-b8fe-464a265755f1');

UPDATE activity
SET source_id = CASE transmitter
    WHEN 'TX01' THEN '10da26f2-f143-4e3c-b8fe-464a265755f1'
    WHEN 'TX02' THEN '20da26f2-f143-4e3c-b8fe-464a265755f2'
END
WHERE transmitter IS NOT NULL;

CREATE INDEX recordings_source_path ON recordings(source_id, source_relative_path);
CREATE INDEX additional_files_source_path ON additional_files(source_id, source_relative_path);
CREATE INDEX backup_runs_source ON backup_runs(source_id, started_at);
CREATE INDEX activity_source ON activity(source_id, occurred_at);
CREATE INDEX deletion_runs_source ON deletion_runs(source_id, started_at);

CREATE TRIGGER backup_runs_require_source
BEFORE INSERT ON backup_runs
WHEN NEW.source_id IS NULL
BEGIN
    SELECT RAISE(ABORT, 'source_id is required');
END;

CREATE TRIGGER backup_runs_source_is_immutable
BEFORE UPDATE OF source_id ON backup_runs
WHEN NEW.source_id IS NOT OLD.source_id
BEGIN
    SELECT RAISE(ABORT, 'source_id is immutable');
END;

CREATE TRIGGER recordings_legacy_transmitter_is_read_only
BEFORE INSERT ON recordings
WHEN NEW.transmitter IS NOT NULL
BEGIN
    SELECT RAISE(ABORT, 'legacy transmitter writes are forbidden');
END;

CREATE TRIGGER recordings_legacy_transmitter_update_is_forbidden
BEFORE UPDATE OF transmitter ON recordings
WHEN NEW.transmitter IS NOT OLD.transmitter
BEGIN
    SELECT RAISE(ABORT, 'legacy transmitter writes are forbidden');
END;

CREATE TRIGGER additional_files_legacy_transmitter_is_read_only
BEFORE INSERT ON additional_files
WHEN NEW.transmitter IS NOT NULL
BEGIN
    SELECT RAISE(ABORT, 'legacy transmitter writes are forbidden');
END;

CREATE TRIGGER additional_files_legacy_transmitter_update_is_forbidden
BEFORE UPDATE OF transmitter ON additional_files
WHEN NEW.transmitter IS NOT OLD.transmitter
BEGIN
    SELECT RAISE(ABORT, 'legacy transmitter writes are forbidden');
END;

CREATE TRIGGER deletion_runs_legacy_transmitter_is_read_only
BEFORE INSERT ON deletion_runs
WHEN NEW.transmitter IS NOT NULL
BEGIN
    SELECT RAISE(ABORT, 'legacy transmitter writes are forbidden');
END;

CREATE TRIGGER deletion_runs_legacy_transmitter_update_is_forbidden
BEFORE UPDATE OF transmitter ON deletion_runs
WHEN NEW.transmitter IS NOT OLD.transmitter
BEGIN
    SELECT RAISE(ABORT, 'legacy transmitter writes are forbidden');
END;

CREATE TRIGGER activity_legacy_source_consistency
BEFORE INSERT ON activity
WHEN NEW.transmitter IS NOT NULL
BEGIN
    SELECT RAISE(ABORT, 'legacy transmitter writes are forbidden');
END;

CREATE TRIGGER activity_legacy_transmitter_update_is_forbidden
BEFORE UPDATE OF transmitter ON activity
WHEN NEW.transmitter IS NOT OLD.transmitter
BEGIN
    SELECT RAISE(ABORT, 'legacy transmitter writes are forbidden');
END;

DROP TABLE deletion_items_v5;
DROP TABLE additional_deletion_items_v5;
DROP TABLE conversion_cohort_items_v5;
DROP TABLE recordings_v5;
DROP TABLE additional_files_v5;
DROP TABLE deletion_runs_v5;

INSERT INTO schema_migrations(version, applied_at)
VALUES (6, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'));

COMMIT;
PRAGMA foreign_keys = ON;
