//! Which verified recordings still need a transcript, and transcribing one of them.
//!
//! The queue is derived, not stored: a recording is pending while one of its selected
//! transcript files is missing. That keeps the ledger free of transcription state, lets a user
//! re-run a recording by deleting its transcript, and means a crash mid-file loses nothing but
//! that file's progress.

use std::{
    collections::{HashMap, HashSet},
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
    source::SourceId,
    transcription::{TranscriptFormat, TranscriptionSettings, WhisperOptions},
};
use time::{OffsetDateTime, PrimitiveDateTime, UtcOffset, format_description::well_known::Rfc3339};

use crate::platform::macos::audio::TRANSCRIPTION_SAMPLE_RATE_HZ;

use super::{
    engine::{EngineError, PcmDecoder, Segment, Transcript, TranscriptionEngine},
    output::{TranscriptMeta, missing_formats, write_transcript},
    timeline::{StartSource, local_modified_time, recording_start, source_label, start_from_name},
    translate::{TranslateError, Translator},
};

/// What the queue needs to stamp transcripts: each source's recorder rule name, for the bracket
/// label, and the local offset a modification time is read in.
#[derive(Debug, Clone, Default)]
pub struct SourceLabels {
    pub rule_names: HashMap<SourceId, String>,
    pub offset: Option<UtcOffset>,
}

/// One recording waiting for a transcript.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub recording_id: String,
    pub relative_path: PathBuf,
    pub audio_path: PathBuf,
    pub missing: Vec<TranscriptFormat>,
    /// The selected formats with no translation file yet; empty when translation is off.
    pub missing_translations: Vec<TranscriptFormat>,
    /// The bracketed label after each line's stamp.
    pub source: String,
    /// The start time the original file name encodes.
    pub name_start: Option<PrimitiveDateTime>,
    /// The original file's modification time on the local clock, the fallback start.
    pub modified_at: Option<PrimitiveDateTime>,
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
    labels: &SourceLabels,
) -> Vec<Candidate> {
    let formats = settings.formats.selected();
    let translated_into = settings
        .translation
        .is_configured()
        .then_some(settings.translation.target_language.as_str());
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
            let relative = &recording.artifact.relative_path;
            let missing = missing_formats(output_root, relative, &formats, None);
            let missing_translations = translated_into.map_or_else(Vec::new, |language| {
                missing_formats(output_root, relative, &formats, Some(language))
            });
            (is_file && !(missing.is_empty() && missing_translations.is_empty())).then(|| {
                let source_name = recording
                    .source_relative_path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let artifact_name = recording
                    .artifact
                    .relative_path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let candidate = Candidate {
                    source: source_label(
                        &source_name,
                        labels
                            .rule_names
                            .get(&recording.source_id)
                            .map(String::as_str),
                    ),
                    name_start: start_from_name(&source_name)
                        .or_else(|| start_from_name(&artifact_name)),
                    modified_at: local_modified_time(
                        recording.source_mtime_ns,
                        labels.offset.unwrap_or(UtcOffset::UTC),
                    ),
                    recording_id: recording.id,
                    relative_path: recording.artifact.relative_path,
                    audio_path,
                    missing,
                    missing_translations,
                };
                (recording.verified_at, candidate)
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
    /// How many of `files_written` are translations.
    pub files_translated: usize,
    /// Where the line stamps' start time came from, if one was found.
    pub start_source: Option<StartSource>,
}

/// Everything one transcription needs besides the recording itself.
pub struct Job<'a> {
    pub output_root: &'a Path,
    pub model_path: &'a Path,
    pub model_label: &'a str,
    pub options: &'a WhisperOptions,
    pub dictionary: &'a DictionaryContext,
    /// Where translations go, when translation is set up.
    pub translation: Option<TranslationJob<'a>>,
    pub cancel: Arc<AtomicBool>,
}

#[derive(Clone, Copy)]
pub struct TranslationJob<'a> {
    pub translator: &'a dyn Translator,
    pub target_language: &'a str,
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
    // The decoded length is exact, unlike the ledger's, and is what the modification-time
    // fallback subtracts.
    let duration_ms = samples.len() as u64 * 1_000 / u64::from(TRANSCRIPTION_SAMPLE_RATE_HZ);
    drop(samples);
    for segment in &mut transcript.segments {
        segment.text = job.dictionary.correct(&segment.text);
    }
    let start = recording_start(candidate.name_start, candidate.modified_at, duration_ms);
    let meta = TranscriptMeta {
        model: job.model_label,
        source: &candidate.source,
        started_at: start.map(|(started_at, _)| started_at),
        translated_into: None,
    };
    let mut files_written = 0;
    if !candidate.missing.is_empty() {
        files_written = write_transcript(
            job.output_root,
            &candidate.relative_path,
            &candidate.missing,
            &transcript,
            &meta,
        )
        .map_err(|_| EngineError::OutputFailed)?;
    }
    // Written after the transcript, so a translation that fails never costs the transcript.
    let mut files_translated = 0;
    if let Some(translation) = job
        .translation
        .filter(|_| !candidate.missing_translations.is_empty())
    {
        let translated = translate_transcript(&transcript, translation, job)?;
        files_translated = write_transcript(
            job.output_root,
            &candidate.relative_path,
            &candidate.missing_translations,
            &translated,
            &TranscriptMeta {
                translated_into: Some(translation.target_language),
                ..meta
            },
        )
        .map_err(|_| EngineError::OutputFailed)?;
    }
    Ok(Finished {
        segments: transcript.segments.len(),
        files_written: files_written + files_translated,
        files_translated,
        start_source: start.map(|(_, source)| source),
    })
}

/// The transcript in the target language, keeping every segment's timing.
///
/// A recording already spoken in the target language is copied rather than sent to the model.
fn translate_transcript(
    transcript: &Transcript,
    translation: TranslationJob<'_>,
    job: &Job<'_>,
) -> Result<Transcript, EngineError> {
    let texts = transcript
        .segments
        .iter()
        .map(|segment| segment.text.clone())
        .collect::<Vec<_>>();
    let translated = if transcript.language.as_deref() == Some(translation.target_language)
        || texts.is_empty()
    {
        texts
    } else {
        translation
            .translator
            .translate(
                &texts,
                translation.target_language,
                job.dictionary.prompt(),
                &job.cancel,
            )
            .map_err(|error| match error {
                TranslateError::Cancelled => EngineError::Cancelled,
                TranslateError::Unavailable => EngineError::TranslationUnavailable,
                TranslateError::InvalidResponse => EngineError::TranslationFailed,
            })?
    };
    Ok(Transcript {
        language: Some(translation.target_language.to_owned()),
        segments: transcript
            .segments
            .iter()
            .zip(translated)
            .map(|(segment, text)| Segment {
                start_ms: segment.start_ms,
                end_ms: segment.end_ms,
                text,
            })
            .collect(),
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
                &SourceLabels::default(),
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
            &SourceLabels::default(),
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
        let name = "DJI/TX01_MIC001_20251028_143022.m4a";
        let fixture = Fixture::new(&[name]);
        let settings = enabled_since("2026-10-28T01:00:00Z");
        let recordings = vec![recording("a", name, "2026-10-28T02:00:00Z")];
        let candidates = pending_candidates(
            recordings.clone(),
            &settings,
            fixture.destination.path(),
            fixture.output.path(),
            &HashSet::new(),
            &SourceLabels::default(),
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
            translation: None,
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
                files_written: 2,
                files_translated: 0,
                start_source: Some(StartSource::FileName),
            }
        );
        assert_eq!(
            fs::read_to_string(
                fixture
                    .output
                    .path()
                    .join("DJI/TX01_MIC001_20251028_143022.txt")
            )
            .unwrap(),
            "2025-10-28 14:30:22 [TX01] - 백업 마이크 테스트\n",
            "lines carry the recorder's clock, and aliases become the canonical term"
        );
        assert_eq!(engine.prompt.as_deref(), Some("백업 마이크"));
        assert!(fixture.pending(recordings, &settings).is_empty());
    }

    #[test]
    fn a_name_without_a_time_is_stamped_from_the_modification_time_and_rule_name() {
        let fixture = Fixture::new(&["Zoom/ZOOM0001.m4a"]);
        let settings = enabled_since("2026-10-28T01:00:00Z");
        let mut zoom = recording("a", "Zoom/ZOOM0001.m4a", "2026-10-28T02:00:00Z");
        zoom.source_relative_path = PathBuf::from("RECORD/FOLDER01/ZOOM0001.WAV");
        // 2025-10-28 15:30:22 at +09:00, written when the one-second take ended.
        zoom.source_mtime_ns = 1_761_633_022_000_000_000;
        let labels = SourceLabels {
            rule_names: HashMap::from([(zoom.source_id.clone(), "Zoom H1n".to_owned())]),
            offset: Some(UtcOffset::from_hms(9, 0, 0).unwrap()),
        };
        let candidates = pending_candidates(
            vec![zoom],
            &settings,
            fixture.destination.path(),
            fixture.output.path(),
            &HashSet::new(),
            &labels,
        );
        let job = Job {
            output_root: fixture.output.path(),
            model_path: Path::new("/models/ggml-medium.bin"),
            model_label: "Medium",
            options: &settings.whisper,
            dictionary: &DictionaryContext::default(),
            translation: None,
            cancel: Arc::new(AtomicBool::new(false)),
        };

        let finished = transcribe_one(
            &candidates[0],
            &job,
            &SilenceDecoder,
            &mut ScriptedEngine {
                calls: 0,
                prompt: None,
            },
            Box::new(|_| {}),
        )
        .unwrap();

        assert_eq!(finished.start_source, Some(StartSource::ModifiedTime));
        assert_eq!(
            fs::read_to_string(fixture.output.path().join("Zoom/ZOOM0001.txt")).unwrap(),
            "2025-10-28 15:30:21 [Zoom H1n] - 백업마이크 테스트\n"
        );
    }

    /// The segments, target language, and glossary one translation request carried.
    type TranslatorCall = (Vec<String>, String, Option<String>);

    /// Marks every segment as English, or fails the way it is told to.
    struct FakeTranslator {
        failure: Option<TranslateError>,
        calls: std::sync::Mutex<Vec<TranslatorCall>>,
    }

    impl Translator for FakeTranslator {
        fn translate(
            &self,
            texts: &[String],
            target_language: &str,
            glossary: Option<&str>,
            _cancel: &AtomicBool,
        ) -> Result<Vec<String>, TranslateError> {
            self.calls.lock().unwrap().push((
                texts.to_vec(),
                target_language.to_owned(),
                glossary.map(str::to_owned),
            ));
            match self.failure {
                Some(error) => Err(error),
                None => Ok(texts.iter().map(|text| format!("EN: {text}")).collect()),
            }
        }
    }

    fn translating(target: &str) -> TranscriptionSettings {
        TranscriptionSettings {
            translation: backup_core::transcription::TranslationSettings {
                enabled: true,
                target_language: target.to_owned(),
                ollama_model: "qwen2.5:7b".to_owned(),
                ..Default::default()
            },
            ..enabled_since("2026-10-28T01:00:00Z")
        }
    }

    #[test]
    fn a_translation_is_written_beside_the_transcript_with_the_same_stamps() {
        let name = "DJI/TX01_MIC001_20251028_143022.m4a";
        let fixture = Fixture::new(&[name]);
        let settings = translating("en");
        let candidates = pending_candidates(
            vec![recording("a", name, "2026-10-28T02:00:00Z")],
            &settings,
            fixture.destination.path(),
            fixture.output.path(),
            &HashSet::new(),
            &SourceLabels::default(),
        );
        assert_eq!(
            candidates[0].missing_translations,
            [TranscriptFormat::Txt, TranscriptFormat::Srt]
        );
        let translator = FakeTranslator {
            failure: None,
            calls: std::sync::Mutex::new(Vec::new()),
        };
        let dictionary = DictionaryContext::compile(
            true,
            &[DictionaryEntry {
                id: "dictionary-1".to_owned(),
                term: "백업 마이크".to_owned(),
                aliases: Vec::new(),
                enabled: true,
            }],
        );
        let job = Job {
            output_root: fixture.output.path(),
            model_path: Path::new("/models/ggml-medium.bin"),
            model_label: "Medium",
            options: &settings.whisper,
            dictionary: &dictionary,
            translation: Some(TranslationJob {
                translator: &translator,
                target_language: "en",
            }),
            cancel: Arc::new(AtomicBool::new(false)),
        };

        let finished = transcribe_one(
            &candidates[0],
            &job,
            &SilenceDecoder,
            &mut ScriptedEngine {
                calls: 0,
                prompt: None,
            },
            Box::new(|_| {}),
        )
        .unwrap();

        assert_eq!((finished.files_written, finished.files_translated), (4, 2));
        let output = fixture.output.path().join("DJI");
        assert_eq!(
            fs::read_to_string(output.join("TX01_MIC001_20251028_143022.en.txt")).unwrap(),
            "2025-10-28 14:30:22 [TX01] - EN: 백업마이크 테스트\n"
        );
        assert!(output.join("TX01_MIC001_20251028_143022.txt").is_file());
        let calls = translator.calls.lock().unwrap();
        assert_eq!(calls[0].1, "en");
        assert_eq!(
            calls[0].2.as_deref(),
            Some("백업 마이크"),
            "the glossary is passed along"
        );
        assert!(
            pending_candidates(
                vec![recording("a", name, "2026-10-28T02:00:00Z")],
                &settings,
                fixture.destination.path(),
                fixture.output.path(),
                &HashSet::new(),
                &SourceLabels::default(),
            )
            .is_empty()
        );
    }

    #[test]
    fn a_failed_translation_keeps_the_transcript_and_reports_why() {
        let fixture = Fixture::new(&["a.m4a"]);
        let settings = translating("en");
        let candidates = pending_candidates(
            vec![recording("a", "a.m4a", "2026-10-28T02:00:00Z")],
            &settings,
            fixture.destination.path(),
            fixture.output.path(),
            &HashSet::new(),
            &SourceLabels::default(),
        );
        for (failure, expected) in [
            (
                TranslateError::Unavailable,
                EngineError::TranslationUnavailable,
            ),
            (
                TranslateError::InvalidResponse,
                EngineError::TranslationFailed,
            ),
        ] {
            let translator = FakeTranslator {
                failure: Some(failure),
                calls: std::sync::Mutex::new(Vec::new()),
            };
            let job = Job {
                output_root: fixture.output.path(),
                model_path: Path::new("/models/ggml-medium.bin"),
                model_label: "Medium",
                options: &settings.whisper,
                dictionary: &DictionaryContext::default(),
                translation: Some(TranslationJob {
                    translator: &translator,
                    target_language: "en",
                }),
                cancel: Arc::new(AtomicBool::new(false)),
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
            assert_eq!(result.unwrap_err(), expected);
            assert!(fixture.output.path().join("a.txt").is_file());
            assert!(!fixture.output.path().join("a.en.txt").exists());
        }
    }

    #[test]
    fn translation_without_a_model_is_not_queued() {
        let fixture = Fixture::new(&["a.m4a"]);
        let mut settings = translating("en");
        settings.translation.ollama_model.clear();
        fs::write(fixture.output.path().join("a.txt"), "done").unwrap();
        fs::write(fixture.output.path().join("a.srt"), "done").unwrap();
        assert!(
            fixture
                .pending(
                    vec![recording("a", "a.m4a", "2026-10-28T02:00:00Z")],
                    &settings
                )
                .is_empty()
        );
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
            &SourceLabels::default(),
        );
        let job = Job {
            output_root: fixture.output.path(),
            model_path: Path::new("/models/ggml-medium.bin"),
            model_label: "Medium",
            options: &settings.whisper,
            dictionary: &DictionaryContext::default(),
            translation: None,
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
