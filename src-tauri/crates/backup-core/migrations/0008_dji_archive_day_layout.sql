BEGIN IMMEDIATE;

UPDATE backup_rules
SET date_folder_layout = CASE
        WHEN date_folder_layout = 'year_month' THEN 'year_month_day'
        ELSE date_folder_layout
    END,
    preset_revision = 2,
    updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
WHERE preset_kind = 'dji_mic_mini_2s'
  AND (preset_revision IS NULL OR preset_revision < 2);

INSERT INTO schema_migrations(version, applied_at)
VALUES (8, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'));

COMMIT;
