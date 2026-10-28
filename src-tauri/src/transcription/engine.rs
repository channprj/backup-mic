//! Turning a recording into timed text: decoding to 16 kHz PCM, then running whisper.cpp.
//!
//! Both halves sit behind traits so the queue can be tested without Apple's audio tools or a
//! multi-gigabyte model.

use std::{
    fs::File,
    io::{BufReader, Read, Seek as _, SeekFrom},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use backup_core::{
    error::CoreError,
    transcription::{AUTO_LANGUAGE, SamplingStrategy, WhisperOptions},
};
use whisper_rs::{FullParams, WhisperContext, WhisperContextParameters};

use crate::platform::macos::audio::{TRANSCRIPTION_SAMPLE_RATE_HZ, decode_for_transcription};

/// One timed stretch of speech.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment {
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transcript {
    /// The language whisper.cpp transcribed in, detected or as configured.
    pub language: Option<String>,
    pub segments: Vec<Segment>,
}

/// Why one recording could not be transcribed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EngineError {
    Cancelled,
    ModelUnavailable,
    DecodeFailed,
    TranscriptionFailed,
    OutputFailed,
}

impl EngineError {
    pub const fn message_code(self) -> &'static str {
        match self {
            Self::Cancelled => "transcription_cancelled",
            Self::ModelUnavailable => "whisper_model_unavailable",
            Self::DecodeFailed => "transcription_decode_failed",
            Self::TranscriptionFailed => "transcription_failed",
            Self::OutputFailed => "transcription_output_failed",
        }
    }
}

/// Decodes a recording to the mono 16 kHz samples whisper.cpp takes.
pub trait PcmDecoder: Send + Sync {
    fn decode(&self, input: &Path) -> Result<Vec<f32>, EngineError>;
}

/// Decodes through `afconvert` into a private temporary WAV and reads it back.
#[derive(Debug, Clone, Copy, Default)]
pub struct AppleDecoder;

impl PcmDecoder for AppleDecoder {
    fn decode(&self, input: &Path) -> Result<Vec<f32>, EngineError> {
        let directory = tempfile::tempdir().map_err(|_| EngineError::DecodeFailed)?;
        let wav = directory.path().join("transcription.wav");
        decode_for_transcription(input, &wav).map_err(|_| EngineError::DecodeFailed)?;
        read_pcm16_mono(&wav).map_err(|_| EngineError::DecodeFailed)
    }
}

/// Reads a 16 kHz mono 16-bit PCM WAV into `[-1, 1)` samples.
///
/// Walks the chunk list rather than assuming a 44-byte header, because `afconvert` inserts a
/// `FLLR` padding chunk before the data.
pub fn read_pcm16_mono(path: &Path) -> Result<Vec<f32>, CoreError> {
    let mut reader = BufReader::new(File::open(path).map_err(CoreError::CopyFailed)?);
    let mut header = [0_u8; 12];
    reader
        .read_exact(&mut header)
        .map_err(|_| CoreError::ArtifactInvalid)?;
    if &header[0..4] != b"RIFF" || &header[8..12] != b"WAVE" {
        return Err(CoreError::ArtifactInvalid);
    }
    let mut format_checked = false;
    loop {
        let mut chunk = [0_u8; 8];
        reader
            .read_exact(&mut chunk)
            .map_err(|_| CoreError::ArtifactInvalid)?;
        let size = u32::from_le_bytes([chunk[4], chunk[5], chunk[6], chunk[7]]);
        match &chunk[0..4] {
            b"fmt " => {
                let mut format = vec![0_u8; size as usize];
                reader
                    .read_exact(&mut format)
                    .map_err(|_| CoreError::ArtifactInvalid)?;
                format_checked = is_pcm16_mono_16k(&format);
                if !format_checked {
                    return Err(CoreError::ArtifactInvalid);
                }
            }
            b"data" if format_checked => {
                let mut samples = Vec::with_capacity(size as usize / 2);
                let mut bytes = [0_u8; 2];
                let mut data = reader.take(u64::from(size));
                while data.read_exact(&mut bytes).is_ok() {
                    samples.push(f32::from(i16::from_le_bytes(bytes)) / 32_768.0);
                }
                return Ok(samples);
            }
            _ => {
                let padded = i64::from(size) + i64::from(size % 2);
                reader
                    .seek(SeekFrom::Current(padded))
                    .map_err(|_| CoreError::ArtifactInvalid)?;
            }
        }
    }
}

fn is_pcm16_mono_16k(format: &[u8]) -> bool {
    if format.len() < 16 {
        return false;
    }
    let tag = u16::from_le_bytes([format[0], format[1]]);
    let channels = u16::from_le_bytes([format[2], format[3]]);
    let rate = u32::from_le_bytes([format[4], format[5], format[6], format[7]]);
    let bits = u16::from_le_bytes([format[14], format[15]]);
    matches!(tag, 1 | 0xFFFE) && channels == 1 && rate == TRANSCRIPTION_SAMPLE_RATE_HZ && bits == 16
}

/// Runs a model over decoded samples.
pub trait TranscriptionEngine: Send {
    /// `progress` receives whisper.cpp's percentage; `cancel` stops the run between decoder
    /// steps.
    fn transcribe(
        &mut self,
        model_path: &Path,
        options: &WhisperOptions,
        samples: &[f32],
        cancel: Arc<AtomicBool>,
        progress: Box<dyn FnMut(i32) + Send>,
    ) -> Result<Transcript, EngineError>;

    /// Drops a loaded model, giving its memory back while the queue is idle.
    fn unload(&mut self);
}

/// whisper.cpp through `whisper-rs`, keeping the last model loaded between recordings.
#[derive(Default)]
pub struct WhisperEngine {
    loaded: Option<LoadedModel>,
}

struct LoadedModel {
    path: PathBuf,
    use_gpu: bool,
    flash_attention: bool,
    context: WhisperContext,
}

impl WhisperEngine {
    fn context_for(
        &mut self,
        model_path: &Path,
        options: &WhisperOptions,
    ) -> Result<&WhisperContext, EngineError> {
        let reusable = self.loaded.as_ref().is_some_and(|loaded| {
            loaded.path == model_path
                && loaded.use_gpu == options.use_gpu
                && loaded.flash_attention == options.flash_attention
        });
        if !reusable {
            self.loaded = None;
            let mut parameters = WhisperContextParameters::default();
            parameters
                .use_gpu(options.use_gpu)
                .flash_attn(options.flash_attention);
            let context = WhisperContext::new_with_params(model_path, parameters)
                .map_err(|_| EngineError::ModelUnavailable)?;
            self.loaded = Some(LoadedModel {
                path: model_path.to_owned(),
                use_gpu: options.use_gpu,
                flash_attention: options.flash_attention,
                context,
            });
        }
        self.loaded
            .as_ref()
            .map(|loaded| &loaded.context)
            .ok_or(EngineError::ModelUnavailable)
    }
}

impl TranscriptionEngine for WhisperEngine {
    fn transcribe(
        &mut self,
        model_path: &Path,
        options: &WhisperOptions,
        samples: &[f32],
        cancel: Arc<AtomicBool>,
        progress: Box<dyn FnMut(i32) + Send>,
    ) -> Result<Transcript, EngineError> {
        let context = self.context_for(model_path, options)?;
        let mut state = context
            .create_state()
            .map_err(|_| EngineError::ModelUnavailable)?;
        let mut parameters = FullParams::new(match options.sampling {
            SamplingStrategy::Greedy => whisper_rs::SamplingStrategy::Greedy {
                best_of: i32::from(options.best_of),
            },
            SamplingStrategy::BeamSearch => whisper_rs::SamplingStrategy::BeamSearch {
                beam_size: i32::from(options.beam_size),
                patience: -1.0,
            },
        });
        parameters.set_language(Some(options.language.as_str()));
        parameters.set_translate(options.translate);
        parameters.set_n_threads(thread_count(options.threads));
        parameters.set_temperature(options.temperature);
        parameters.set_temperature_inc(options.temperature_increment);
        parameters.set_no_speech_thold(options.no_speech_threshold);
        parameters.set_entropy_thold(options.entropy_threshold);
        parameters.set_logprob_thold(options.logprob_threshold);
        parameters.set_max_len(i32::from(options.max_segment_length));
        parameters.set_split_on_word(options.split_on_word);
        parameters.set_suppress_blank(options.suppress_blank);
        parameters.set_suppress_nst(options.suppress_non_speech);
        parameters.set_no_context(!options.condition_on_previous_text);
        if !options.initial_prompt.is_empty() {
            parameters.set_initial_prompt(&options.initial_prompt);
        }
        parameters.set_print_special(false);
        parameters.set_print_progress(false);
        parameters.set_print_realtime(false);
        parameters.set_print_timestamps(false);
        let progress: Box<dyn FnMut(i32)> = progress;
        parameters.set_progress_callback_safe::<Option<Box<dyn FnMut(i32)>>, Box<dyn FnMut(i32)>>(
            Some(progress),
        );
        // `set_abort_callback_safe` reads its user data back as the closure type it was given,
        // but stores a boxed `dyn FnMut`; passing that boxed form keeps the two the same type.
        let abort_flag = cancel.clone();
        let abort: Box<dyn FnMut() -> bool> = Box::new(move || abort_flag.load(Ordering::SeqCst));
        parameters
            .set_abort_callback_safe::<Option<Box<dyn FnMut() -> bool>>, Box<dyn FnMut() -> bool>>(
                Some(abort),
            );

        let result = state.full(parameters, samples);
        if cancel.load(Ordering::SeqCst) {
            return Err(EngineError::Cancelled);
        }
        result.map_err(|_| EngineError::TranscriptionFailed)?;

        let language = if options.language == AUTO_LANGUAGE {
            whisper_rs::get_lang_str(state.full_lang_id_from_state()).map(str::to_owned)
        } else {
            Some(options.language.clone())
        };
        let segments = state
            .as_iter()
            .map(|segment| Segment {
                start_ms: centiseconds_to_ms(segment.start_timestamp()),
                end_ms: centiseconds_to_ms(segment.end_timestamp()),
                text: segment
                    .to_str_lossy()
                    .map(|text| text.trim().to_owned())
                    .unwrap_or_default(),
            })
            .filter(|segment| !segment.text.is_empty())
            .collect();
        Ok(Transcript { language, segments })
    }

    fn unload(&mut self) {
        self.loaded = None;
    }
}

fn centiseconds_to_ms(value: i64) -> u64 {
    u64::try_from(value).unwrap_or(0).saturating_mul(10)
}

/// whisper.cpp's own default: four threads, or fewer on a smaller machine.
fn thread_count(configured: u8) -> i32 {
    if configured > 0 {
        return i32::from(configured);
    }
    std::thread::available_parallelism()
        .map_or(4, |threads| threads.get().min(4))
        .try_into()
        .unwrap_or(4)
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;

    use super::*;

    fn wav(chunks: &[(&[u8; 4], Vec<u8>)]) -> Vec<u8> {
        let mut body = b"WAVE".to_vec();
        for (id, data) in chunks {
            body.extend_from_slice(*id);
            body.extend_from_slice(&(data.len() as u32).to_le_bytes());
            body.extend_from_slice(data);
            if data.len() % 2 == 1 {
                body.push(0);
            }
        }
        let mut file = b"RIFF".to_vec();
        file.extend_from_slice(&(body.len() as u32).to_le_bytes());
        file.extend_from_slice(&body);
        file
    }

    fn format(channels: u16, rate: u32, bits: u16) -> Vec<u8> {
        let mut format = Vec::new();
        format.extend_from_slice(&1_u16.to_le_bytes());
        format.extend_from_slice(&channels.to_le_bytes());
        format.extend_from_slice(&rate.to_le_bytes());
        format.extend_from_slice(&(rate * 2).to_le_bytes());
        format.extend_from_slice(&2_u16.to_le_bytes());
        format.extend_from_slice(&bits.to_le_bytes());
        format
    }

    fn write(bytes: &[u8]) -> tempfile::NamedTempFile {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        file.write_all(bytes).unwrap();
        file
    }

    #[test]
    fn reads_samples_after_an_odd_sized_padding_chunk() {
        let samples = [0_i16, 16_384, -32_768, 32_767]
            .iter()
            .flat_map(|sample| sample.to_le_bytes())
            .collect::<Vec<_>>();
        let file = write(&wav(&[
            (b"fmt ", format(1, 16_000, 16)),
            (b"FLLR", vec![0; 3]),
            (b"data", samples),
        ]));

        let decoded = read_pcm16_mono(file.path()).unwrap();
        assert_eq!(decoded, [0.0, 0.5, -1.0, 32_767.0 / 32_768.0]);
    }

    #[test]
    fn refuses_audio_that_is_not_16k_mono_16_bit() {
        for refused in [
            format(2, 16_000, 16),
            format(1, 48_000, 16),
            format(1, 16_000, 24),
        ] {
            let file = write(&wav(&[(b"fmt ", refused), (b"data", vec![0; 4])]));
            assert!(read_pcm16_mono(file.path()).is_err());
        }
        let file = write(&wav(&[(b"data", vec![0; 4])]));
        assert!(
            read_pcm16_mono(file.path()).is_err(),
            "data before any format chunk cannot be interpreted"
        );
        assert!(read_pcm16_mono(write(b"not a wav file").path()).is_err());
    }

    /// Speaks a sentence with `say`, makes it a 48 kHz stereo WAV like a two-transmitter
    /// recording, and runs the real decoder and whisper.cpp over it. Needs a downloaded model:
    /// `BACKUP_MIC_WHISPER_MODEL=/path/ggml-tiny.bin cargo test -p backup-mic --lib -- --ignored`.
    #[test]
    #[ignore = "needs BACKUP_MIC_WHISPER_MODEL and macOS speech synthesis"]
    fn transcribes_synthesized_stereo_speech_and_honours_cancellation() {
        let model = PathBuf::from(
            std::env::var_os("BACKUP_MIC_WHISPER_MODEL").expect("BACKUP_MIC_WHISPER_MODEL"),
        );
        let directory = tempfile::tempdir().unwrap();
        let speech = directory.path().join("speech.aiff");
        let stereo = directory.path().join("stereo.wav");
        assert!(
            std::process::Command::new("/usr/bin/say")
                .arg("-o")
                .arg(&speech)
                .arg("The backup finished and every recording was verified.")
                .status()
                .unwrap()
                .success()
        );
        assert!(
            std::process::Command::new("/usr/bin/afconvert")
                .arg(&speech)
                .args(["-o"])
                .arg(&stereo)
                .args(["-f", "WAVE", "-d", "LEI24@48000", "--channels", "2"])
                .status()
                .unwrap()
                .success()
        );

        let samples = AppleDecoder.decode(&stereo).unwrap();
        assert!(
            samples.len() > 16_000,
            "a sentence lasts more than a second"
        );
        let options = WhisperOptions {
            language: "en".to_owned(),
            ..WhisperOptions::default()
        };
        let mut engine = WhisperEngine::default();
        let mut last_progress = 0;
        let progress = std::sync::Arc::new(std::sync::Mutex::new(0));
        let reported = progress.clone();
        let transcript = engine
            .transcribe(
                &model,
                &options,
                &samples,
                Arc::new(AtomicBool::new(false)),
                Box::new(move |percent| *reported.lock().unwrap() = percent),
            )
            .unwrap();
        last_progress = last_progress.max(*progress.lock().unwrap());
        let text = transcript
            .segments
            .iter()
            .map(|segment| segment.text.to_lowercase())
            .collect::<Vec<_>>()
            .join(" ");
        assert!(text.contains("backup"), "{text}");
        assert!(text.contains("verified"), "{text}");
        assert_eq!(transcript.language.as_deref(), Some("en"));
        assert!(transcript.segments[0].end_ms > transcript.segments[0].start_ms);
        assert!(last_progress > 0, "whisper.cpp reports progress");

        let cancelled = engine.transcribe(
            &model,
            &options,
            &samples,
            Arc::new(AtomicBool::new(true)),
            Box::new(|_| {}),
        );
        assert_eq!(cancelled.unwrap_err(), EngineError::Cancelled);
    }

    #[test]
    fn segment_times_convert_from_centiseconds() {
        assert_eq!(centiseconds_to_ms(123), 1_230);
        assert_eq!(centiseconds_to_ms(-1), 0);
    }

    #[test]
    fn automatic_threads_follow_whisper_cpp_and_never_exceed_four() {
        assert_eq!(thread_count(6), 6);
        assert!((1..=4).contains(&thread_count(0)));
    }
}
