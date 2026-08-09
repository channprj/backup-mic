BEGIN IMMEDIATE;

ALTER TABLE recordings ADD COLUMN artifact_format TEXT NOT NULL DEFAULT 'wav'
    CHECK (artifact_format IN ('wav', 'm4a'));
ALTER TABLE recordings ADD COLUMN artifact_codec TEXT;
ALTER TABLE recordings ADD COLUMN artifact_sample_rate_hz INTEGER
    CHECK (artifact_sample_rate_hz IS NULL OR artifact_sample_rate_hz > 0);
ALTER TABLE recordings ADD COLUMN artifact_channel_count INTEGER
    CHECK (artifact_channel_count IS NULL OR artifact_channel_count > 0);
ALTER TABLE recordings ADD COLUMN artifact_valid_frames INTEGER
    CHECK (artifact_valid_frames IS NULL OR artifact_valid_frames > 0);
ALTER TABLE recordings ADD COLUMN artifact_duration_micros INTEGER
    CHECK (artifact_duration_micros IS NULL OR artifact_duration_micros > 0);
ALTER TABLE recordings ADD COLUMN conversion_status TEXT NOT NULL DEFAULT 'not_required'
    CHECK (conversion_status IN ('not_required', 'pending', 'complete', 'failed'));
ALTER TABLE recordings ADD COLUMN conversion_error_code TEXT;
ALTER TABLE recordings ADD COLUMN retirement_status TEXT NOT NULL DEFAULT 'present'
    CHECK (retirement_status IN ('present', 'trash_pending', 'moved_to_trash', 'legacy_deleted', 'failed'));
ALTER TABLE recordings ADD COLUMN retired_session_relative_path TEXT;

UPDATE recordings
SET retirement_status = 'legacy_deleted'
WHERE source_deleted_at IS NOT NULL;

INSERT INTO schema_migrations(version, applied_at)
VALUES (2, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'));

COMMIT;
