use backup_core::{
    artifact::{
        ConversionStatus, OutputFormat, RetirementStatus, VerifiedArtifact, VerifiedAudioProperties,
    },
    batch::{BatchPhase, FrozenPreferences, M4A_PROFILE_ID},
    error::CoreError,
    ledger::{Ledger, VerifiedRecording},
    state::Transmitter,
};
use dji_mic_backup_lib::artifact_pipeline::order_conversion_cohort;
use tempfile::tempdir;

fn wav(id: &str, destination: &str, hash: char, run: &str) -> VerifiedRecording {
    VerifiedRecording {
        id: id.to_owned(),
        transmitter: Transmitter::Tx01,
        source_relative_path: format!("TX_MIC001_20260810_001116/{id}.wav").into(),
        source_size: 4,
        source_mtime_ns: 1,
        source_sha256: hash.to_string().repeat(64),
        artifact: VerifiedArtifact {
            relative_path: destination.into(),
            format: OutputFormat::Wav,
            byte_count: 4,
            sha256: hash.to_string().repeat(64),
            audio: None,
        },
        conversion_status: ConversionStatus::NotRequired,
        conversion_error_code: None,
        retirement_status: RetirementStatus::Present,
        retired_session_relative_path: None,
        verified_at: "2026-08-10T00:00:00Z".to_owned(),
        backup_run_id: run.to_owned(),
    }
}

fn converted(mut recording: VerifiedRecording, run: &str) -> VerifiedRecording {
    recording.artifact = VerifiedArtifact {
        relative_path: recording.artifact.relative_path.with_extension("m4a"),
        format: OutputFormat::M4a,
        byte_count: 2,
        sha256: "f".repeat(64),
        audio: Some(VerifiedAudioProperties {
            codec: "aac".to_owned(),
            sample_rate_hz: 48_000,
            channel_count: 1,
            valid_frames: 48_000,
            duration_micros: 1_000_000,
        }),
    };
    recording.conversion_status = ConversionStatus::Complete;
    recording.backup_run_id = run.to_owned();
    recording
}

#[test]
fn current_and_historical_wavs_form_one_deterministic_all_or_nothing_cohort() {
    let directory = tempdir().unwrap();
    let mut ledger = Ledger::open(directory.path().join("ledger.sqlite3")).unwrap();
    ledger
        .begin_backup_run("historical", "2026-08-09T00:00:00Z", 4)
        .unwrap();
    let historical = wav(
        "historical",
        "2026/2026-08-08/TX01/historical.wav",
        'a',
        "historical",
    );
    ledger.commit_verified_recording(&historical).unwrap();
    ledger
        .finish_backup_run("historical", "2026-08-09T00:01:00Z", "completed", None)
        .unwrap();
    ledger
        .begin_batch_run(
            "current",
            "2026-08-10T00:00:00Z",
            8,
            FrozenPreferences {
                automatic_backup: true,
                m4a_conversion: true,
                automatic_trash: false,
            },
        )
        .unwrap();
    ledger
        .advance_batch_phase("current", BatchPhase::Copying)
        .unwrap();
    let later = wav("later", "2026/2026-08-10/TX01/later.wav", 'b', "current");
    let earlier = wav(
        "earlier",
        "2026/2026-08-10/TX01/earlier.wav",
        'c',
        "current",
    );
    ledger.commit_verified_recording(&later).unwrap();
    ledger.commit_verified_recording(&earlier).unwrap();
    assert_eq!(ledger.verified_recording_count().unwrap(), 3);
    ledger
        .advance_batch_phase("current", BatchPhase::CopiesVerified)
        .unwrap();

    let ordered = order_conversion_cohort(vec![later, historical, earlier]).unwrap();
    assert_eq!(
        ordered
            .iter()
            .map(|recording| recording.id.as_str())
            .collect::<Vec<_>>(),
        ["historical", "earlier", "later"]
    );
    let ids = ordered
        .iter()
        .map(|recording| recording.id.clone())
        .collect::<Vec<_>>();
    ledger
        .begin_conversion_cohort("current", &ids, M4A_PROFILE_ID)
        .unwrap();

    for recording in ordered.iter().take(2) {
        let m4a = converted(recording.clone(), "current");
        ledger
            .replace_verified_artifact_with_superseded_wav(
                &m4a,
                &recording.artifact.relative_path,
                recording.artifact.byte_count,
                &recording.artifact.sha256,
            )
            .unwrap();
        ledger
            .mark_conversion_item_verified("current", &recording.id)
            .unwrap();
    }
    assert!(matches!(
        ledger.commit_m4a_barrier("current"),
        Err(CoreError::InvalidRequest)
    ));
    assert_eq!(
        ledger.batch_run_evidence("current").unwrap().unwrap().phase,
        BatchPhase::Converting
    );

    let final_recording = ordered.last().unwrap();
    let final_m4a = converted(final_recording.clone(), "current");
    ledger
        .replace_verified_artifact_with_superseded_wav(
            &final_m4a,
            &final_recording.artifact.relative_path,
            final_recording.artifact.byte_count,
            &final_recording.artifact.sha256,
        )
        .unwrap();
    ledger
        .mark_conversion_item_verified("current", &final_recording.id)
        .unwrap();
    ledger.commit_m4a_barrier("current").unwrap();
    assert_eq!(
        ledger
            .superseded_wav_evidence("historical")
            .unwrap()
            .unwrap()
            .0,
        std::path::Path::new("2026/2026-08-08/TX01/historical.wav")
    );
    assert_eq!(ledger.pending_superseded_wavs().unwrap().len(), 3);
    ledger
        .mark_superseded_wav_moved_to_trash("historical")
        .unwrap();
    assert_eq!(ledger.pending_superseded_wavs().unwrap().len(), 2);
    assert!(
        ledger
            .superseded_wav_evidence("historical")
            .unwrap()
            .is_some()
    );
    assert_eq!(
        ledger.batch_run_evidence("current").unwrap().unwrap().phase,
        BatchPhase::M4aCohortVerified
    );
}
