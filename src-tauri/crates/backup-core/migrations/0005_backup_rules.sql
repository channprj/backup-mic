BEGIN IMMEDIATE;

CREATE TABLE backup_rules (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    normalized_name TEXT NOT NULL,
    archive_directory_name TEXT NOT NULL,
    enabled INTEGER NOT NULL CHECK (enabled IN (0, 1)),
    volume_name_glob TEXT NOT NULL,
    filename_prefix TEXT NOT NULL,
    filename_suffix TEXT NOT NULL,
    filename_profile TEXT NOT NULL
        CHECK (filename_profile IN ('preserve', 'dji_tx_short')),
    device_constraint_profile TEXT NOT NULL
        CHECK (device_constraint_profile IN ('generic_external', 'dji_mic_mini_2s')),
    preset_kind TEXT,
    preset_revision INTEGER CHECK (preset_revision IS NULL OR preset_revision > 0),
    archive_directory_locked INTEGER NOT NULL DEFAULT 0
        CHECK (archive_directory_locked IN (0, 1)),
    archived_at TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    CHECK ((preset_kind IS NULL) = (preset_revision IS NULL))
);

CREATE UNIQUE INDEX backup_rules_active_normalized_name
    ON backup_rules(normalized_name)
    WHERE archived_at IS NULL;

CREATE UNIQUE INDEX backup_rules_unique_preset_kind
    ON backup_rules(preset_kind)
    WHERE preset_kind IS NOT NULL;

CREATE TABLE backup_rule_patterns (
    rule_id TEXT NOT NULL REFERENCES backup_rules(id) ON DELETE CASCADE,
    kind TEXT NOT NULL
        CHECK (kind IN ('required_path', 'backup_file', 'session_directory')),
    ordinal INTEGER NOT NULL CHECK (ordinal >= 0),
    pattern TEXT NOT NULL,
    PRIMARY KEY (rule_id, kind, ordinal)
);

CREATE TABLE rule_device_bindings (
    rule_id TEXT NOT NULL REFERENCES backup_rules(id) ON DELETE RESTRICT,
    volume_uuid TEXT NOT NULL,
    volume_uuid_normalized TEXT NOT NULL UNIQUE,
    protocol TEXT NOT NULL,
    media_name TEXT NOT NULL,
    nominal_capacity INTEGER NOT NULL CHECK (nominal_capacity >= 0),
    slot_label TEXT,
    paired_at TEXT NOT NULL,
    PRIMARY KEY (rule_id, volume_uuid_normalized)
);

INSERT INTO backup_rules(
    id, name, normalized_name, archive_directory_name, enabled, volume_name_glob,
    filename_prefix, filename_suffix, filename_profile, device_constraint_profile,
    preset_kind, preset_revision, archive_directory_locked, archived_at,
    created_at, updated_at
) VALUES (
    '6d784c99-8b0e-4a32-a0a2-d7730f68cf28',
    'DJI Mic Mini 2S',
    'dji mic mini 2s',
    'DJI Mic Mini 2S',
    1,
    '*',
    '',
    '',
    'dji_tx_short',
    'dji_mic_mini_2s',
    'dji_mic_mini_2s',
    1,
    0,
    NULL,
    strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
    strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
);

INSERT INTO backup_rule_patterns(rule_id, kind, ordinal, pattern) VALUES
    ('6d784c99-8b0e-4a32-a0a2-d7730f68cf28', 'backup_file', 0, '*.WAV'),
    ('6d784c99-8b0e-4a32-a0a2-d7730f68cf28', 'backup_file', 1, 'TX_MIC*/*.WAV'),
    ('6d784c99-8b0e-4a32-a0a2-d7730f68cf28', 'session_directory', 0, 'TX_MIC*');

INSERT INTO schema_migrations(version, applied_at)
VALUES (5, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'));

COMMIT;
