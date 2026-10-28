use std::path::PathBuf;

use backup_core::{
    error::CoreError,
    ledger::Ledger,
    transcription::{
        ModelChoice, TRANSCRIPTION_SETTING, TranscriptionSettings, TranslationSettings,
        WhisperOptions,
    },
};

#[test]
fn transcription_settings_survive_reopening_the_ledger() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("ledger.sqlite3");
    let mut ledger = Ledger::open(&path).unwrap();
    assert_eq!(
        ledger.read_transcription_settings().unwrap(),
        TranscriptionSettings::default(),
        "a ledger that never saved transcription settings reads the defaults"
    );

    let settings = TranscriptionSettings {
        enabled: true,
        enabled_since: Some("2026-10-28T01:00:00Z".to_owned()),
        models_directory: Some(PathBuf::from("/Users/example/Models")),
        model: ModelChoice::Catalog {
            id: "large-v3-turbo".to_owned(),
        },
        whisper: WhisperOptions {
            language: "ko".to_owned(),
            ..WhisperOptions::default()
        },
        translation: TranslationSettings {
            enabled: true,
            target_language: "en".to_owned(),
            ollama_model: "qwen2.5:7b".to_owned(),
            ..TranslationSettings::default()
        },
        ..TranscriptionSettings::default()
    };
    ledger
        .set_transcription_settings(&settings, "2026-10-28T01:00:00Z")
        .unwrap();
    drop(ledger);

    let reopened = Ledger::open(&path).unwrap();
    assert_eq!(reopened.read_transcription_settings().unwrap(), settings);
}

#[test]
fn refused_transcription_settings_leave_the_stored_row_untouched() {
    let directory = tempfile::tempdir().unwrap();
    let mut ledger = Ledger::open(directory.path().join("ledger.sqlite3")).unwrap();
    let refused = TranscriptionSettings {
        model: ModelChoice::Catalog {
            id: "not-a-model".to_owned(),
        },
        ..TranscriptionSettings::default()
    };

    assert!(matches!(
        ledger.set_transcription_settings(&refused, "2026-10-28T01:00:00Z"),
        Err(CoreError::InvalidRequest)
    ));
    assert_eq!(ledger.setting(TRANSCRIPTION_SETTING).unwrap(), None);
}
