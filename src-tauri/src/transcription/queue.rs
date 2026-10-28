//! Which verified recordings still need a transcript, and transcribing one of them.
//!
//! The queue is derived, not stored: a recording is pending while one of its selected
//! transcript files is missing. That keeps the ledger free of transcription state, lets a user
//! re-run a recording by deleting its transcript, and means a crash mid-file loses nothing but
//! that file's progress.

use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use backup_core::{
    artifact::ConversionStatus,
    dictionary::DictionaryContext,
    filesystem::is_safe_relative_path,
    ledger::VerifiedRecording,
    transcription::{TranscriptFormat, TranscriptionSettings, WhisperOptions},
};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

use super::{
    engine::{EngineError, PcmDecoder, TranscriptionEngine},
    output::{missing_formats, write_transcript},
};

/// One recording waiting for a transcript.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub recording_id: String,
    pub relative_path: PathBuf,
    pub audio_path: PathBuf,
    pub missing: Vec<TranscriptFormat>,
}

impl Candidate {
    /// The file name alone, which is all the window and the logs ever show.
    pub fn display_name(&self) -> String {
        self.relative_path
            .file_name()
            .map(|name| crate::dto::safe_label(&name.to_string_lossy()))
            .unwrap_or_default()
    }
}

/// The recordings that still need a transcript, oldest verification first.
///
/// A recording is left out while its M4A conversion is pending (the WAV is about to be
/// superseded), when it holds no audio, when it was verified before transcription was switched
/// on and the backlog was not asked for, or when it already failed during this app session.
pub fn pending_candidates(
    recordings: Vec<VerifiedRecording>,
    settings: &TranscriptionSettings,
    destination: &Path,
    output_root: &Path,
    skipped: &HashSet<String>,
) -> Vec<Candidate> {
    let formats = settings.formats.selected();
    let since = if settings.include_existing_recordings {
        None
    } else {
        match settings.enabled_since.as_deref().map(parse_time) {
            Some(Some(since)) => Some(since),
            _ => return Vec::new(),
        }
    };
    let mut pending = recordings
        .into_iter()
        .filter(|recording| {
            !skipped.contains(&recording.id)
                && recording.conversion_status != ConversionStatus::Pending
                && recording
                    .artifact
                    .audio
                    .as_ref()
                    .is_none_or(|audio| audio.duration_micros > 0)
                && is_safe_relative_path(&recording.artifact.relative_path)
                && since.is_none_or(|since| {
                    parse_time(&recording.verified_at).is_some_and(|verified| verified >= since)
                })
        })
        .filter_map(|recording| {
            let audio_path = destination.join(&recording.artifact.relative_path);
            let is_file = fs::symlink_metadata(&audio_path)
                .is_ok_and(|metadata| metadata.file_type().is_file());
            let missing = missing_formats(output_root, &recording.artifact.relative_path, &formats);
            (is_file && !missing.is_empty()).then(|| {
                (
                    recording.verified_at.clone(),
                    Candidate {
                        recording_id: recording.id,
                        relative_path: recording.artifact.relative_path,
                        audio_path,
                        missing,
                    },
                )
            })
        })
        .collect::<Vec<_>>();
    pending.sort_by(|left, right| {
        (&left.0, &left.1.relative_path).cmp(&(&right.0, &right.1.relative_path))
    });
    pending
        .into_iter()
        .map(|(_, candidate)| candidate)
        .collect()
}

fn parse_time(value: &str) -> Option<OffsetDateTime> {
    OffsetDateTime::parse(value, &Rfc3339).ok()
}

/// What one finished recording produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finished {
    pub segments: usize,
    pub files_written: usize,
}

/// Everything one transcription needs besides the recording itself.
pub struct Job<'a> {
    pub output_root: &'a Path,
    pub model_path: &'a Path,
    pub model_label: &'a str,
    pub options: &'a WhisperOptions,
    pub dictionary: &'a DictionaryContext,
    pub cancel: Arc<AtomicBool>,
}

/// Decodes, transcribes, applies the dictionary, and writes the missing transcript files for
/// one recording.
pub fn transcribe_one(
    candidate: &Candidate,
    job: &Job<'_>,
    decoder: &dyn PcmDecoder,
    engine: &mut dyn TranscriptionEngine,
    progress: Box<dyn FnMut(i32) + Send>,
) -> Result<Finished, EngineError> {
    let samples = decoder.decode(&candidate.audio_path)?;
    if job.cancel.load(Ordering::SeqCst) {
        return Err(EngineError::Cancelled);
    }
    let mut transcript = engine.transcribe(
        job.model_path,
        job.options,
        job.dictionary.prompt(),
        &samples,
        job.cancel.clone(),
        progress,
    )?;
    drop(samples);
    for segment in &mut transcript.segments {
        segment.text = job.dictionary.correct(&segment.text);
    }
    let files_written = write_transcript(
        job.output_root,
        &candidate.relative_path,
        &candidate.missing,
        &transcript,
        job.model_label,
    )
    .map_err(|_| EngineError::OutputFailed)?;
    Ok(Finished {
        segments: transcript.segments.len(),
        files_written,
    })
}

#[cfg(test)]
mod tests {
    use backup_core::{
        artifact::{OutputFormat, RetirementStatus, VerifiedArtifact, VerifiedAudioProperties},
        dictionary::DictionaryEntry,
        source::{LEGACY_TX01_SOURCE_ID, SourceId},
        transcription::TranscriptFormats,
    };

    use super::super::engine::{Segment, Transcript};
    use super::*;

    fn recording(id: &str, relative: &str, verified_at: &str) -> VerifiedRecording {
        VerifiedRecording {
            id: id.to_owned(),
            source_id: SourceId::parse(LEGACY_TX01_SOURCE_ID).unwrap(),
            source_relative_path: PathBuf::from(relative),
            source_size: 4,
            source_mtime_ns: 0,
            source_sha256: "0".repeat(64),
            artifact: VerifiedArtifact {
                relative_path: PathBuf::from(relative),
                format: OutputFormat::M4a,
                byte_count: 4,
                sha256: "0".repeat(64),
                audio: Some(VerifiedAudioProperties {
                    codec: "aac".to_owned(),
                    sample_rate_hz: 48_000,
                    channel_count: 1,
                    valid_frames: 48_000,
                    duration_micros: 1_000_000,
                }),
            },
            conversion_status: ConversionStatus::Complete,
            conversion_error_code: None,
            retirement_status: RetirementStatus::Present,
            retired_session_relative_path: None,
            verified_at: verified_at.to_owned(),
            backup_run_id: "run".to_owned(),
        }
    }

    struct Fixture {
        destination: tempfile::TempDir,
        output: tempfile::TempDir,
    }

    impl Fixture {
        fn new(files: &[&str]) -> Self {
            let destination = tempfile::tempdir().unwrap();
            for file in files {
                let path = destination.path().join(file);
                fs::create_dir_all(path.parent().unwrap()).unwrap();
                fs::write(path, b"audio").unwrap();
            }
            Self {
                destination,
                output: tempfile::tempdir().unwrap(),
            }
        }

        fn pending(
            &self,
            recordings: Vec<VerifiedRecording>,
            settings: &TranscriptionSettings,
        ) -> Vec<String> {
            pending_candidates(
                recordings,
                settings,
                self.destination.path(),
                self.output.path(),
                &HashSet::new(),
            )
            .into_iter()
            .map(|candidate| candidate.recording_id)
            .collect()
        }
    }

    fn enabled_since(since: &str) -> TranscriptionSettings {
        TranscriptionSettings {
            enabled: true,
            enabled_since: Some(since.to_owned()),
            ..TranscriptionSettings::default()
        }
    }

    #[test]
    fn only_recordings_verified_after_enabling_are_queued_unless_the_backlog_is_asked_for() {
        let fixture = Fixture::new(&["DJI/old.m4a", "DJI/new.m4a"]);
        let recordings = vec![
            recording("new", "DJI/new.m4a", "2026-10-28T02:00:00Z"),
            recording("old", "DJI/old.m4a", "2026-10-27T00:00:00Z"),
        ];
        let mut settings = enabled_since("2026-10-28T01:00:00Z");
        assert_eq!(fixture.pending(recordings.clone(), &settings), ["new"]);

        settings.include_existing_recordings = true;
        assert_eq!(
            fixture.pending(recordings, &settings),
            ["old", "new"],
            "the backlog runs oldest first"
        );
    }

    #[test]
    fn pending_conversions_silent_files_missing_audio_and_finished_ones_wait_or_skip() {
        let fixture = Fixture::new(&["a.wav", "b.m4a", "d.m4a"]);
        let mut converting = recording("converting", "a.wav", "2026-10-28T02:00:00Z");
        converting.conversion_status = ConversionStatus::Pending;
        let mut silent = recording("silent", "b.m4a", "2026-10-28T02:00:00Z");
        silent.artifact.audio.as_mut().unwrap().duration_micros = 0;
        let missing = recording("missing", "c.m4a", "2026-10-28T02:00:00Z");
        let done = recording("done", "d.m4a", "2026-10-28T02:00:00Z");
        let settings = TranscriptionSettings {
            formats: TranscriptFormats {
                txt: true,
                srt: false,
                vtt: false,
                json: false,
            },
            ..enabled_since("2026-10-28T01:00:00Z")
        };
        fs::write(fixture.output.path().join("d.txt"), "done").unwrap();

        assert!(
            fixture
                .pending(vec![converting, silent, missing, done], &settings)
                .is_empty()
        );
    }

    #[test]
    fn a_recording_that_failed_this_session_is_not_retried_in_a_loop() {
        let fixture = Fixture::new(&["a.m4a"]);
        let skipped = HashSet::from(["a".to_owned()]);
        let pending = pending_candidates(
            vec![recording("a", "a.m4a", "2026-10-28T02:00:00Z")],
            &enabled_since("2026-10-28T01:00:00Z"),
            fixture.destination.path(),
            fixture.output.path(),
            &skipped,
        );
        assert!(pending.is_empty());
    }

    #[test]
    fn a_missing_enable_time_queues_nothing_rather_than_the_whole_archive() {
        let fixture = Fixture::new(&["a.m4a"]);
        let settings = TranscriptionSettings {
            enabled: true,
            ..TranscriptionSettings::default()
        };
        assert!(
            fixture
                .pending(
                    vec![recording("a", "a.m4a", "2026-10-28T02:00:00Z")],
                    &settings
                )
                .is_empty()
        );
    }

    struct SilenceDecoder;

    impl PcmDecoder for SilenceDecoder {
        fn decode(&self, _input: &Path) -> Result<Vec<f32>, EngineError> {
            Ok(vec![0.0; 16_000])
        }
    }

    struct ScriptedEngine {
        calls: usize,
        prompt: Option<String>,
    }

    impl TranscriptionEngine for ScriptedEngine {
        fn transcribe(
            &mut self,
            _model_path: &Path,
            _options: &WhisperOptions,
            prompt: Option<&str>,
            samples: &[f32],
            cancel: Arc<AtomicBool>,
            mut progress: Box<dyn FnMut(i32) + Send>,
        ) -> Result<Transcript, EngineError> {
            self.calls += 1;
            self.prompt = prompt.map(str::to_owned);
            assert_eq!(samples.len(), 16_000);
            if cancel.load(Ordering::SeqCst) {
                return Err(EngineError::Cancelled);
            }
            progress(100);
            Ok(Transcript {
                language: Some("ko".to_owned()),
                segments: vec![Segment {
                    start_ms: 0,
                    end_ms: 1_000,
                    text: "백업마이크 테스트".to_owned(),
                }],
            })
        }

        fn unload(&mut self) {}
    }

    #[test]
    fn a_transcribed_recording_writes_its_missing_files_and_leaves_the_queue() {
        let fixture = Fixture::new(&["DJI/a.m4a"]);
        let settings = enabled_since("2026-10-28T01:00:00Z");
        let recordings = vec![recording("a", "DJI/a.m4a", "2026-10-28T02:00:00Z")];
        let candidates = pending_candidates(
            recordings.clone(),
            &settings,
            fixture.destination.path(),
            fixture.output.path(),
            &HashSet::new(),
        );
        let mut engine = ScriptedEngine {
            calls: 0,
            prompt: None,
        };
        let dictionary = DictionaryContext::compile(
            true,
            &[DictionaryEntry {
                id: "dictionary-1".to_owned(),
                term: "백업 마이크".to_owned(),
                aliases: vec!["백업마이크".to_owned()],
                enabled: true,
            }],
        );
        let job = Job {
            output_root: fixture.output.path(),
            model_path: Path::new("/models/ggml-medium.bin"),
            model_label: "Medium",
            options: &settings.whisper,
            dictionary: &dictionary,
            cancel: Arc::new(AtomicBool::new(false)),
        };

        let finished = transcribe_one(
            &candidates[0],
            &job,
            &SilenceDecoder,
            &mut engine,
            Box::new(|_| {}),
        )
        .unwrap();

        assert_eq!(
            finished,
            Finished {
                segments: 1,
                files_written: 2
            }
        );
        assert_eq!(
            fs::read_to_string(fixture.output.path().join("DJI/a.txt")).unwrap(),
            "백업 마이크 테스트\n",
            "aliases become the canonical term before anything is written"
        );
        assert_eq!(engine.prompt.as_deref(), Some("백업 마이크"));
        assert!(fixture.pending(recordings, &settings).is_empty());
    }

    #[test]
    fn a_cancelled_recording_writes_nothing() {
        let fixture = Fixture::new(&["a.m4a"]);
        let settings = enabled_since("2026-10-28T01:00:00Z");
        let candidates = pending_candidates(
            vec![recording("a", "a.m4a", "2026-10-28T02:00:00Z")],
            &settings,
            fixture.destination.path(),
            fixture.output.path(),
            &HashSet::new(),
        );
        let job = Job {
            output_root: fixture.output.path(),
            model_path: Path::new("/models/ggml-medium.bin"),
            model_label: "Medium",
            options: &settings.whisper,
            dictionary: &DictionaryContext::default(),
            cancel: Arc::new(AtomicBool::new(true)),
        };

        let result = transcribe_one(
            &candidates[0],
            &job,
            &SilenceDecoder,
            &mut ScriptedEngine {
                calls: 0,
                prompt: None,
            },
            Box::new(|_| {}),
        );
        assert_eq!(result.unwrap_err(), EngineError::Cancelled);
        assert!(!fixture.output.path().join("a.txt").exists());
    }
}
