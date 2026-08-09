BEGIN IMMEDIATE;

ALTER TABLE recordings ADD COLUMN superseded_wav_retirement_status TEXT NOT NULL DEFAULT 'none'
    CHECK (superseded_wav_retirement_status IN (
        'none', 'pending', 'moved_to_trash', 'absent_after_conversion'
    ));

UPDATE recordings
   SET superseded_wav_retirement_status = 'pending'
 WHERE superseded_wav_relative_path IS NOT NULL
   AND superseded_wav_size IS NOT NULL
   AND superseded_wav_sha256 IS NOT NULL;

UPDATE recordings
   SET superseded_wav_relative_path =
           substr(destination_relative_path, 1, length(destination_relative_path) - 3) || 'wav',
       superseded_wav_size = source_size,
       superseded_wav_sha256 = source_sha256,
       superseded_wav_retirement_status = 'absent_after_conversion'
 WHERE artifact_format = 'm4a'
   AND conversion_status = 'complete'
   AND lower(destination_relative_path) LIKE '%.m4a'
   AND superseded_wav_relative_path IS NULL
   AND superseded_wav_size IS NULL
   AND superseded_wav_sha256 IS NULL;

INSERT INTO schema_migrations(version, applied_at)
VALUES (4, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'));

COMMIT;
