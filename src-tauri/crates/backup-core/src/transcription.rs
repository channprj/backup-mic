//! Transcribing backed-up recordings with whisper.cpp: the model catalog and the settings.
//!
//! This module only decides what is allowed — which models exist, which bytes each one must be,
//! and which option values are in range. Downloading, decoding, and running a model are adapter
//! work in the Tauri crate, so nothing here touches the network, the GPU, or the file system.

use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::dictionary::{DictionaryEntry, validate_dictionary};
use crate::error::CoreError;

/// The settings-table key holding the whole transcription configuration as one JSON object.
///
/// One row rather than one key per option: the options only make sense together, and a single
/// write keeps a half-saved combination from ever being read back.
pub const TRANSCRIPTION_SETTING: &str = "transcription";

/// The model a fresh install transcribes with.
pub const DEFAULT_WHISPER_MODEL: &str = "medium";

/// The Hugging Face revision every catalog download is pinned to.
///
/// Pinning the revision is what makes the recorded size and SHA-256 meaningful: `main` could
/// move, and a moved file would then fail verification forever instead of being a new model.
pub const WHISPER_MODEL_REVISION: &str = "5359861c739e955e79d9a303bcbc70fb988958b1";
const WHISPER_MODEL_REPOSITORY: &str = "https://huggingface.co/ggerganov/whisper.cpp/resolve";

/// whisper.cpp runs at most this many decoders, which bounds both beam size and best-of.
pub const MAX_WHISPER_DECODERS: u8 = 8;

/// The language codes whisper.cpp recognises, in its own order.
pub const WHISPER_LANGUAGES: [&str; 100] = [
    "en", "zh", "de", "es", "ru", "ko", "fr", "ja", "pt", "tr", "pl", "ca", "nl", "ar", "sv", "it",
    "id", "hi", "fi", "vi", "he", "uk", "el", "ms", "cs", "ro", "da", "hu", "ta", "no", "th", "ur",
    "hr", "bg", "lt", "la", "mi", "ml", "cy", "sk", "te", "fa", "lv", "bn", "sr", "az", "sl", "kn",
    "et", "mk", "br", "eu", "is", "hy", "ne", "mn", "bs", "kk", "sq", "sw", "gl", "mr", "pa", "si",
    "km", "sn", "yo", "so", "af", "oc", "ka", "be", "tg", "sd", "gu", "am", "yi", "lo", "uz", "fo",
    "ht", "ps", "tk", "nn", "mt", "sa", "lb", "my", "bo", "tl", "mg", "as", "tt", "haw", "ln",
    "ha", "ba", "jw", "su", "yue",
];

/// Where a fresh install looks for Ollama: its default local address.
pub const DEFAULT_OLLAMA_URL: &str = "http://localhost:11434";

/// The language transcripts are translated into until the user picks another.
pub const DEFAULT_TRANSLATION_LANGUAGE: &str = "ko";

/// The language value that asks whisper.cpp to detect the spoken language itself.
pub const AUTO_LANGUAGE: &str = "auto";

/// One downloadable ggml model and the exact bytes it must be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WhisperModel {
    pub id: &'static str,
    pub label: &'static str,
    pub byte_count: u64,
    pub sha256: &'static str,
    pub english_only: bool,
    pub quantized: bool,
}

impl WhisperModel {
    /// The file name whisper.cpp publishes the model under, which is also its local name.
    pub fn file_name(&self) -> String {
        format!("ggml-{}.bin", self.id)
    }

    pub fn download_url(&self) -> String {
        format!(
            "{WHISPER_MODEL_REPOSITORY}/{WHISPER_MODEL_REVISION}/{}",
            self.file_name()
        )
    }
}

const fn model(
    id: &'static str,
    label: &'static str,
    byte_count: u64,
    sha256: &'static str,
    english_only: bool,
    quantized: bool,
) -> WhisperModel {
    WhisperModel {
        id,
        label,
        byte_count,
        sha256,
        english_only,
        quantized,
    }
}

/// Every model the settings window offers, smallest first.
///
/// Sizes and digests come from the Git LFS pointers at [`WHISPER_MODEL_REVISION`].
pub const WHISPER_MODELS: [WhisperModel; 15] = [
    model(
        "tiny",
        "Tiny",
        77_691_713,
        "be07e048e1e599ad46341c8d2a135645097a538221678b7acdd1b1919c6e1b21",
        false,
        false,
    ),
    model(
        "tiny.en",
        "Tiny (English)",
        77_704_715,
        "921e4cf8686fdd993dcd081a5da5b6c365bfde1162e72b08d75ac75289920b1f",
        true,
        false,
    ),
    model(
        "base",
        "Base",
        147_951_465,
        "60ed5bc3dd14eea856493d334349b405782ddcaf0028d4b5df4088345fba2efe",
        false,
        false,
    ),
    model(
        "base.en",
        "Base (English)",
        147_964_211,
        "a03779c86df3323075f5e796cb2ce5029f00ec8869eee3fdfb897afe36c6d002",
        true,
        false,
    ),
    model(
        "small",
        "Small",
        487_601_967,
        "1be3a9b2063867b937e64e2ec7483364a79917e157fa98c5d94b5c1fffea987b",
        false,
        false,
    ),
    model(
        "small.en",
        "Small (English)",
        487_614_201,
        "c6138d6d58ecc8322097e0f987c32f1be8bb0a18532a3f88f734d1bbf9c41e5d",
        true,
        false,
    ),
    model(
        "medium-q5_0",
        "Medium (Q5_0)",
        539_212_467,
        "19fea4b380c3a618ec4723c3eef2eb785ffba0d0538cf43f8f235e7b3b34220f",
        false,
        true,
    ),
    model(
        "large-v3-turbo-q5_0",
        "Large v3 Turbo (Q5_0)",
        574_041_195,
        "394221709cd5ad1f40c46e6031ca61bce88931e6e088c188294c6d5a55ffa7e2",
        false,
        true,
    ),
    model(
        "large-v3-turbo-q8_0",
        "Large v3 Turbo (Q8_0)",
        874_188_075,
        "317eb69c11673c9de1e1f0d459b253999804ec71ac4c23c17ecf5fbe24e259a1",
        false,
        true,
    ),
    model(
        "large-v3-q5_0",
        "Large v3 (Q5_0)",
        1_081_140_203,
        "d75795ecff3f83b5faa89d1900604ad8c780abd5739fae406de19f23ecd98ad1",
        false,
        true,
    ),
    model(
        "medium",
        "Medium",
        1_533_763_059,
        "6c14d5adee5f86394037b4e4e8b59f1673b6cee10e3cf0b11bbdbee79c156208",
        false,
        false,
    ),
    model(
        "medium.en",
        "Medium (English)",
        1_533_774_781,
        "cc37e93478338ec7700281a7ac30a10128929eb8f427dda2e865faa8f6da4356",
        true,
        false,
    ),
    model(
        "large-v3-turbo",
        "Large v3 Turbo",
        1_624_555_275,
        "1fc70f774d38eb169993ac391eea357ef47c88757ef72ee5943879b7e8e2bc69",
        false,
        false,
    ),
    model(
        "large-v2",
        "Large v2",
        3_094_623_691,
        "9a423fe4d40c82774b6af34115b8b935f34152246eb19e80e376071d3f999487",
        false,
        false,
    ),
    model(
        "large-v3",
        "Large v3",
        3_095_033_483,
        "64d182b440b98d5203c4f9bd541544d84c605196c4f7b845dfa11fb23594d1e2",
        false,
        false,
    ),
];

/// Looks up a catalog model by its id.
pub fn whisper_model(id: &str) -> Option<&'static WhisperModel> {
    WHISPER_MODELS.iter().find(|model| model.id == id)
}

/// Which model file transcription loads.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ModelChoice {
    /// A catalog model, loaded from the models directory under its published file name.
    Catalog { id: String },
    /// A ggml file the user picked themselves; its bytes are not checked against the catalog.
    Custom { path: PathBuf },
}

impl Default for ModelChoice {
    fn default() -> Self {
        Self::Catalog {
            id: DEFAULT_WHISPER_MODEL.to_owned(),
        }
    }
}

/// One transcript file type written next to each other for the same recording.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TranscriptFormat {
    Txt,
    Srt,
    Vtt,
    Json,
}

impl TranscriptFormat {
    pub const fn extension(self) -> &'static str {
        match self {
            Self::Txt => "txt",
            Self::Srt => "srt",
            Self::Vtt => "vtt",
            Self::Json => "json",
        }
    }
}

/// The transcript files to write for each recording.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TranscriptFormats {
    pub txt: bool,
    pub srt: bool,
    pub vtt: bool,
    pub json: bool,
}

impl Default for TranscriptFormats {
    fn default() -> Self {
        Self {
            txt: true,
            srt: true,
            vtt: false,
            json: false,
        }
    }
}

impl TranscriptFormats {
    /// The selected formats, in a fixed order so output and tests stay deterministic.
    pub fn selected(self) -> Vec<TranscriptFormat> {
        [
            (self.txt, TranscriptFormat::Txt),
            (self.srt, TranscriptFormat::Srt),
            (self.vtt, TranscriptFormat::Vtt),
            (self.json, TranscriptFormat::Json),
        ]
        .into_iter()
        .filter_map(|(selected, format)| selected.then_some(format))
        .collect()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SamplingStrategy {
    /// Keeps the most likely of `best_of` candidates at each step.
    Greedy,
    /// Searches `beam_size` hypotheses; slower, usually more accurate.
    BeamSearch,
}

/// The decoder options passed to whisper.cpp.
///
/// Defaults match the whisper.cpp command-line tool, except that a fresh install detects the
/// language instead of assuming English.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WhisperOptions {
    /// [`AUTO_LANGUAGE`] or one of [`WHISPER_LANGUAGES`].
    pub language: String,
    pub sampling: SamplingStrategy,
    pub beam_size: u8,
    pub best_of: u8,
    pub temperature: f32,
    /// How much to raise the temperature when a segment fails the thresholds; 0 disables it.
    pub temperature_increment: f32,
    /// CPU threads; 0 lets whisper.cpp choose.
    pub threads: u8,
    /// Run the model on the Metal GPU.
    pub use_gpu: bool,
    pub flash_attention: bool,
    pub no_speech_threshold: f32,
    pub entropy_threshold: f32,
    pub logprob_threshold: f32,
    /// The longest segment in characters; 0 leaves segment length to the model.
    pub max_segment_length: u16,
    /// Break long segments only between words.
    pub split_on_word: bool,
    pub suppress_blank: bool,
    pub suppress_non_speech: bool,
    /// Feed each window the text decoded before it. More consistent wording, but a mistake can
    /// repeat itself across a long recording.
    pub condition_on_previous_text: bool,
}

impl Default for WhisperOptions {
    fn default() -> Self {
        Self {
            language: AUTO_LANGUAGE.to_owned(),
            sampling: SamplingStrategy::BeamSearch,
            beam_size: 5,
            best_of: 5,
            temperature: 0.0,
            temperature_increment: 0.2,
            threads: 0,
            use_gpu: true,
            flash_attention: true,
            no_speech_threshold: 0.6,
            entropy_threshold: 2.4,
            logprob_threshold: -1.0,
            max_segment_length: 0,
            split_on_word: false,
            suppress_blank: true,
            suppress_non_speech: false,
            condition_on_previous_text: false,
        }
    }
}

impl WhisperOptions {
    /// Refuses any value whisper.cpp would clamp, ignore, or crash on.
    ///
    /// Out-of-range input is refused rather than clamped, for the same reason the backup limits
    /// are: the settings window must never show a value that is not the one in effect.
    pub fn validate(&self) -> Result<(), CoreError> {
        let language_known =
            self.language == AUTO_LANGUAGE || WHISPER_LANGUAGES.contains(&self.language.as_str());
        let decoders = 1..=MAX_WHISPER_DECODERS;
        let valid = language_known
            && decoders.contains(&self.beam_size)
            && decoders.contains(&self.best_of)
            && in_range(self.temperature, 0.0, 1.0)
            && in_range(self.temperature_increment, 0.0, 1.0)
            && self.threads <= 32
            && in_range(self.no_speech_threshold, 0.0, 1.0)
            && in_range(self.entropy_threshold, 0.0, 10.0)
            && in_range(self.logprob_threshold, -10.0, 0.0)
            && self.max_segment_length <= 1_000;
        if valid {
            Ok(())
        } else {
            Err(CoreError::InvalidRequest)
        }
    }
}

fn in_range(value: f32, low: f32, high: f32) -> bool {
    value.is_finite() && (low..=high).contains(&value)
}

/// Translating finished transcripts with a local Ollama model.
///
/// Whisper itself only translates into English, so any other target needs a second model. An
/// empty model name means translation is not set up yet, and the queue then skips it rather than
/// failing every recording.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TranslationSettings {
    pub enabled: bool,
    /// One of [`WHISPER_LANGUAGES`].
    pub target_language: String,
    /// The Ollama server, `http://localhost:11434` by default.
    pub ollama_url: String,
    /// An Ollama model name such as `qwen2.5:7b`; empty until the user enters one.
    pub ollama_model: String,
}

impl Default for TranslationSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            target_language: DEFAULT_TRANSLATION_LANGUAGE.to_owned(),
            ollama_url: DEFAULT_OLLAMA_URL.to_owned(),
            ollama_model: String::new(),
        }
    }
}

impl TranslationSettings {
    /// Refuses a target whisper.cpp does not know, a URL that is not plain `http(s)` or that
    /// carries credentials, and a model name Ollama could not have.
    pub fn validate(&self) -> Result<(), CoreError> {
        let url_valid = ["http://", "https://"].iter().any(|scheme| {
            self.ollama_url
                .strip_prefix(scheme)
                .is_some_and(|rest| !rest.is_empty())
        }) && self.ollama_url.chars().count() <= 512
            && !self.ollama_url.chars().any(|character| {
                character.is_whitespace()
                    || character.is_control()
                    || matches!(character, '@' | '"' | '\\')
            });
        let model_valid = self.ollama_model.chars().count() <= 128
            && self.ollama_model.chars().all(|character| {
                character.is_ascii_alphanumeric()
                    || matches!(character, '.' | '_' | ':' | '/' | '-')
            });
        if WHISPER_LANGUAGES.contains(&self.target_language.as_str()) && url_valid && model_valid {
            Ok(())
        } else {
            Err(CoreError::InvalidRequest)
        }
    }

    /// Whether finished transcripts should be translated now.
    pub fn is_configured(&self) -> bool {
        self.enabled && !self.ollama_model.is_empty()
    }
}

/// Everything the transcription feature persists.
///
/// The three paths are absent until the user picks one, so the defaults can follow the app's
/// own folders instead of being frozen into the ledger on first launch.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TranscriptionSettings {
    pub enabled: bool,
    /// When transcription was last switched on. Recordings verified before it are left alone
    /// unless [`Self::include_existing_recordings`] is set, so turning the feature on does not
    /// silently queue an entire archive.
    pub enabled_since: Option<String>,
    pub include_existing_recordings: bool,
    pub output_directory: Option<PathBuf>,
    pub models_directory: Option<PathBuf>,
    pub model: ModelChoice,
    pub formats: TranscriptFormats,
    pub whisper: WhisperOptions,
    /// Whether the dictionary below is applied; entries are kept either way.
    pub dictionary_enabled: bool,
    pub dictionary: Vec<DictionaryEntry>,
    pub translation: TranslationSettings,
}

impl Default for TranscriptionSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            enabled_since: None,
            include_existing_recordings: false,
            output_directory: None,
            models_directory: None,
            model: ModelChoice::default(),
            formats: TranscriptFormats::default(),
            whisper: WhisperOptions::default(),
            dictionary_enabled: true,
            dictionary: Vec::new(),
            translation: TranslationSettings::default(),
        }
    }
}

impl TranscriptionSettings {
    pub fn validate(&self) -> Result<(), CoreError> {
        let paths_valid = self
            .output_directory
            .iter()
            .chain(self.models_directory.iter())
            .all(|path| is_safe_absolute_path(path));
        let model_valid = match &self.model {
            ModelChoice::Catalog { id } => whisper_model(id).is_some(),
            ModelChoice::Custom { path } => is_safe_absolute_path(path),
        };
        if paths_valid && model_valid && !self.formats.selected().is_empty() {
            self.whisper.validate()?;
            self.translation.validate()?;
            validate_dictionary(&self.dictionary)
        } else {
            Err(CoreError::InvalidRequest)
        }
    }

    /// Decodes a stored row, treating an unset row as the defaults.
    ///
    /// A row that does not parse or validate was edited outside the app, which gets the same
    /// fail-closed answer as any other corrupt ledger row.
    pub fn decode(value: Option<&str>) -> Result<Self, CoreError> {
        let Some(value) = value else {
            return Ok(Self::default());
        };
        let settings = serde_json::from_str::<Self>(value).map_err(|_| CoreError::LedgerCorrupt)?;
        settings.validate().map_err(|_| CoreError::LedgerCorrupt)?;
        Ok(settings)
    }

    /// Encodes settings after refusing any that would not decode again.
    pub fn encode(&self) -> Result<String, CoreError> {
        self.validate()?;
        serde_json::to_string(self).map_err(|_| CoreError::InvalidRequest)
    }
}

/// An absolute path with no `.` or `..` components, so it names exactly one place.
pub fn is_safe_absolute_path(path: &Path) -> bool {
    path.is_absolute()
        && path.parent().is_some()
        && path
            .components()
            .all(|component| matches!(component, Component::RootDir | Component::Normal(_)))
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn the_default_model_is_whisper_medium_and_in_the_catalog() {
        let settings = TranscriptionSettings::default();
        assert_eq!(
            settings.model,
            ModelChoice::Catalog {
                id: "medium".to_owned()
            }
        );
        assert!(whisper_model(DEFAULT_WHISPER_MODEL).is_some());
        assert!(!settings.enabled, "transcription is opt-in");
        assert!(settings.validate().is_ok());
    }

    #[test]
    fn catalog_entries_are_unique_pinned_and_well_formed() {
        let mut ids = HashSet::new();
        for model in WHISPER_MODELS {
            assert!(ids.insert(model.id), "{} is listed twice", model.id);
            assert_eq!(model.sha256.len(), 64);
            assert!(model.sha256.bytes().all(|byte| byte.is_ascii_hexdigit()));
            assert!(model.byte_count > 0);
            assert!(model.download_url().starts_with("https://huggingface.co/"));
            assert!(model.download_url().contains(WHISPER_MODEL_REVISION));
            assert!(model.download_url().ends_with(&model.file_name()));
        }
        let sizes = WHISPER_MODELS.map(|model| model.byte_count);
        assert!(
            sizes.windows(2).all(|pair| pair[0] <= pair[1]),
            "the catalog is listed smallest first"
        );
    }

    #[test]
    fn stored_settings_round_trip_and_unset_rows_read_as_defaults() {
        assert_eq!(
            TranscriptionSettings::decode(None).unwrap(),
            TranscriptionSettings::default()
        );
        let settings = TranscriptionSettings {
            enabled: true,
            enabled_since: Some("2026-10-28T00:00:00Z".to_owned()),
            output_directory: Some(PathBuf::from("/Users/example/Transcripts")),
            model: ModelChoice::Custom {
                path: PathBuf::from("/Users/example/models/ggml-custom.bin"),
            },
            whisper: WhisperOptions {
                language: "ko".to_owned(),
                ..WhisperOptions::default()
            },
            dictionary: vec![DictionaryEntry {
                id: "dictionary-1".to_owned(),
                term: "백업 마이크".to_owned(),
                aliases: vec!["백업마이크".to_owned()],
                enabled: true,
            }],
            ..TranscriptionSettings::default()
        };
        let encoded = settings.encode().unwrap();
        assert_eq!(
            TranscriptionSettings::decode(Some(&encoded)).unwrap(),
            settings
        );
    }

    /// Unknown keys are ignored so a row written by a newer build still reads after a downgrade;
    /// the command input is the strict boundary, not the stored row.
    #[test]
    fn a_row_missing_or_adding_fields_reads_with_defaults_for_the_rest() {
        let decoded = TranscriptionSettings::decode(Some(
            r#"{"enabled":true,"later":1,"whisper":{"language":"ko"}}"#,
        ))
        .unwrap();
        assert!(decoded.enabled);
        assert_eq!(decoded.whisper.language, "ko");
        assert_eq!(decoded.whisper.beam_size, 5);
        assert_eq!(decoded.model, ModelChoice::default());
    }

    #[test]
    fn corrupt_or_out_of_range_rows_are_ledger_corruption() {
        for stored in [
            "not json",
            r#"{"model":{"kind":"catalog","id":"giant"}}"#,
            r#"{"model":{"kind":"custom","path":"relative/model.bin"}}"#,
            r#"{"output_directory":"/Users/example/../elsewhere"}"#,
            r#"{"formats":{"txt":false,"srt":false,"vtt":false,"json":false}}"#,
            r#"{"whisper":{"beam_size":9}}"#,
        ] {
            assert!(
                matches!(
                    TranscriptionSettings::decode(Some(stored)),
                    Err(CoreError::LedgerCorrupt)
                ),
                "{stored} must be refused"
            );
        }
    }

    #[test]
    fn whisper_options_refuse_values_whisper_cpp_would_clamp_or_crash_on() {
        let refused = [
            WhisperOptions {
                language: "klingon".to_owned(),
                ..WhisperOptions::default()
            },
            WhisperOptions {
                beam_size: 0,
                ..WhisperOptions::default()
            },
            WhisperOptions {
                best_of: MAX_WHISPER_DECODERS + 1,
                ..WhisperOptions::default()
            },
            WhisperOptions {
                temperature: f32::NAN,
                ..WhisperOptions::default()
            },
            WhisperOptions {
                temperature_increment: 1.5,
                ..WhisperOptions::default()
            },
            WhisperOptions {
                threads: 33,
                ..WhisperOptions::default()
            },
            WhisperOptions {
                logprob_threshold: 0.5,
                ..WhisperOptions::default()
            },
            WhisperOptions {
                max_segment_length: 1_001,
                ..WhisperOptions::default()
            },
        ];
        for options in refused {
            assert!(
                matches!(options.validate(), Err(CoreError::InvalidRequest)),
                "{options:?} must be refused"
            );
        }
        let accepted = WhisperOptions {
            language: "yue".to_owned(),
            beam_size: MAX_WHISPER_DECODERS,
            best_of: 1,
            ..WhisperOptions::default()
        };
        assert!(accepted.validate().is_ok());
    }

    #[test]
    fn translation_needs_a_model_and_refuses_credentials_or_odd_urls() {
        let defaults = TranslationSettings::default();
        assert!(defaults.validate().is_ok());
        assert!(
            !defaults.is_configured(),
            "off and without a model by default"
        );
        let configured = TranslationSettings {
            enabled: true,
            ollama_model: "qwen2.5:7b".to_owned(),
            ollama_url: "http://192.168.0.10:11434".to_owned(),
            ..TranslationSettings::default()
        };
        assert!(configured.validate().is_ok());
        assert!(configured.is_configured());
        for refused in [
            TranslationSettings {
                target_language: "auto".to_owned(),
                ..configured.clone()
            },
            TranslationSettings {
                ollama_url: "ftp://localhost".to_owned(),
                ..configured.clone()
            },
            TranslationSettings {
                ollama_url: "http://user:secret@localhost".to_owned(),
                ..configured.clone()
            },
            TranslationSettings {
                ollama_url: "http://local host".to_owned(),
                ..configured.clone()
            },
            TranslationSettings {
                ollama_url: "http://".to_owned(),
                ..configured.clone()
            },
            TranslationSettings {
                ollama_model: "model; rm".to_owned(),
                ..configured.clone()
            },
        ] {
            assert!(refused.validate().is_err(), "{refused:?}");
        }
    }

    #[test]
    fn formats_list_in_a_fixed_order() {
        let formats = TranscriptFormats {
            txt: false,
            srt: true,
            vtt: true,
            json: true,
        };
        assert_eq!(
            formats.selected(),
            [
                TranscriptFormat::Srt,
                TranscriptFormat::Vtt,
                TranscriptFormat::Json
            ]
        );
    }
}
