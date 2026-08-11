BEGIN IMMEDIATE;

ALTER TABLE backup_rules ADD COLUMN date_folder_layout TEXT NOT NULL
    DEFAULT 'year_month'
    CHECK (date_folder_layout IN ('year_month_day', 'year_month', 'compact_date'));

INSERT INTO schema_migrations(version, applied_at)
VALUES (7, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'));

COMMIT;
