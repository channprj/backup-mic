import { z } from "zod";

// whisper.cpp runs at most eight decoders, bounding both beam size and best-of.
export const maxWhisperDecoders = 8;

const decoderCountSchema = z.number().int().min(1).max(maxWhisperDecoders);

export const transcriptionFolderSchema = z.enum(["output", "models"]);

export const transcriptFormatsSchema = z
  .object({
    txt: z.boolean(),
    srt: z.boolean(),
    vtt: z.boolean(),
    json: z.boolean(),
  })
  .strict()
  .refine(
    (formats) => Object.values(formats).some(Boolean),
    "At least one transcript format is required",
  );

// Mirrors `WhisperOptions::validate` in Rust. Rust refuses anything outside these bounds, so a
// snapshot that fails them means the two sides disagree.
export const whisperOptionsSchema = z
  .object({
    language: z.string().min(2).max(8),
    sampling: z.enum(["greedy", "beam_search"]),
    beam_size: decoderCountSchema,
    best_of: decoderCountSchema,
    temperature: z.number().min(0).max(1),
    temperature_increment: z.number().min(0).max(1),
    threads: z.number().int().min(0).max(32),
    use_gpu: z.boolean(),
    flash_attention: z.boolean(),
    no_speech_threshold: z.number().min(0).max(1),
    entropy_threshold: z.number().min(0).max(10),
    logprob_threshold: z.number().min(-10).max(0),
    max_segment_length: z.number().int().min(0).max(1000),
    split_on_word: z.boolean(),
    suppress_blank: z.boolean(),
    suppress_non_speech: z.boolean(),
    condition_on_previous_text: z.boolean(),
  })
  .strict();

// Mirrors `backup_core::dictionary`: at most 1,000 entries, 32 aliases each, 128 characters a
// value, counted in characters rather than UTF-16 units.
export const maxDictionaryEntries = 1000;
export const maxDictionaryAliases = 32;
export const maxDictionaryTextChars = 128;

const dictionaryTextSchema = z
  .string()
  .min(1)
  .refine(
    (value) =>
      value === value.trim() &&
      [...value].length <= maxDictionaryTextChars &&
      !/\p{Cc}/u.test(value),
    "Dictionary values must be trimmed, short, and free of control characters",
  );

export const dictionaryEntrySchema = z
  .object({
    id: dictionaryTextSchema,
    term: dictionaryTextSchema,
    aliases: z.array(dictionaryTextSchema).max(maxDictionaryAliases),
    enabled: z.boolean(),
  })
  .strict();

export const transcriptionDictionarySchema = z
  .object({
    enabled: z.boolean(),
    entries: z.array(dictionaryEntrySchema).max(maxDictionaryEntries),
  })
  .strict();

// Mirrors `TranslationSettings::validate` in Rust: plain http(s) without credentials, and an
// Ollama model name that is empty until the user enters one.
export const ollamaUrlPattern = /^https?:\/\/[^\s@"\\]+$/u;
export const ollamaModelPattern = /^[A-Za-z0-9._:/-]*$/u;

export const translationSettingsSchema = z
  .object({
    enabled: z.boolean(),
    target_language: z.string().min(2).max(8),
    ollama_url: z.string().max(512).regex(ollamaUrlPattern),
    ollama_model: z.string().max(128).regex(ollamaModelPattern),
  })
  .strict();

export const transcriptionOptionsSchema = z
  .object({
    include_existing_recordings: z.boolean(),
    formats: transcriptFormatsSchema,
    whisper: whisperOptionsSchema,
  })
  .strict();

const whisperModelSchema = z
  .object({
    id: z.string().min(1).max(64),
    label: z.string().min(1).max(128),
    byte_count: z.number().int().positive(),
    english_only: z.boolean(),
    quantized: z.boolean(),
    installed: z.boolean(),
  })
  .strict();

const publicErrorSchema = z
  .object({
    code: z.string(),
    message_code: z.string(),
    retryable: z.boolean(),
    source_id: z.string().nullable(),
    source_label: z.string().nullable(),
  })
  .strict();

export const transcriptionPhaseSchema = z.enum([
  "idle",
  "waiting_for_model",
  "waiting_for_backup",
  "transcribing",
]);

export const transcriptionSnapshotSchema = z
  .object({
    revision: z.number().int().nonnegative(),
    enabled: z.boolean(),
    include_existing_recordings: z.boolean(),
    output_directory_display: z.string().min(1).max(2048),
    output_directory_is_default: z.boolean(),
    models_directory_display: z.string().min(1).max(2048),
    models_directory_is_default: z.boolean(),
    selected_model: z
      .object({
        source: z.enum(["catalog", "custom"]),
        id: z.string().min(1).max(64).nullable(),
        label: z.string().max(128),
        file_display: z.string().min(1).max(2048),
        ready: z.boolean(),
      })
      .strict(),
    models: z.array(whisperModelSchema).max(64),
    download: z
      .object({
        model_id: z.string().min(1).max(64),
        received_bytes: z.number().int().nonnegative(),
        total_bytes: z.number().int().positive(),
      })
      .strict()
      .nullable(),
    formats: transcriptFormatsSchema,
    whisper: whisperOptionsSchema,
    dictionary_enabled: z.boolean(),
    dictionary: z.array(dictionaryEntrySchema).max(maxDictionaryEntries),
    translation: translationSettingsSchema,
    job: z
      .object({
        phase: transcriptionPhaseSchema,
        current_item: z.string().max(128).nullable(),
        percent: z.number().int().min(0).max(100),
        queued_count: z.number().int().nonnegative(),
        completed_count: z.number().int().nonnegative(),
        failed_count: z.number().int().nonnegative(),
        last_completed_at: z.string().nullable(),
      })
      .strict(),
    error: publicErrorSchema.nullable(),
  })
  .strict();

export type TranscriptionFolder = z.infer<typeof transcriptionFolderSchema>;
export type TranscriptFormats = z.infer<typeof transcriptFormatsSchema>;
export type WhisperOptions = z.infer<typeof whisperOptionsSchema>;
export type TranscriptionOptions = z.infer<typeof transcriptionOptionsSchema>;
export type DictionaryEntry = z.infer<typeof dictionaryEntrySchema>;
export type TranslationSettings = z.infer<typeof translationSettingsSchema>;
export type TranscriptionDictionary = z.infer<
  typeof transcriptionDictionarySchema
>;
export type TranscriptionSnapshot = z.infer<typeof transcriptionSnapshotSchema>;
export type WhisperModel = TranscriptionSnapshot["models"][number];
