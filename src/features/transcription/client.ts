import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { z } from "zod";
import {
  transcriptionDictionarySchema,
  transcriptionFolderSchema,
  transcriptionOptionsSchema,
  transcriptionSnapshotSchema,
  translationSettingsSchema,
  type TranscriptionDictionary,
  type TranscriptionFolder,
  type TranscriptionOptions,
  type TranscriptionSnapshot,
  type TranslationSettings,
} from "./contracts";

const snapshotEvent = "transcription-snapshot-changed";
const modelIdSchema = z.string().min(1).max(64);

async function invokeSnapshot(command: string, args?: Record<string, unknown>) {
  return transcriptionSnapshotSchema.parse(await invoke(command, args));
}

export function getTranscriptionSnapshot(): Promise<TranscriptionSnapshot> {
  return invokeSnapshot("get_transcription_snapshot");
}

export function setTranscriptionEnabled(
  enabled: boolean,
): Promise<TranscriptionSnapshot> {
  return invokeSnapshot("set_transcription_enabled", {
    enabled: z.boolean().parse(enabled),
  });
}

export function saveTranscriptionOptions(
  options: TranscriptionOptions,
): Promise<TranscriptionSnapshot> {
  return invokeSnapshot("save_transcription_options", {
    options: transcriptionOptionsSchema.parse(options),
  });
}

export function saveTranscriptionDictionary(
  dictionary: TranscriptionDictionary,
): Promise<TranscriptionSnapshot> {
  return invokeSnapshot("save_transcription_dictionary", {
    dictionary: transcriptionDictionarySchema.parse(dictionary),
  });
}

export function saveTranslationSettings(
  translation: TranslationSettings,
): Promise<TranscriptionSnapshot> {
  return invokeSnapshot("save_translation_settings", {
    translation: translationSettingsSchema.parse(translation),
  });
}

export function selectWhisperModel(
  modelId: string,
): Promise<TranscriptionSnapshot> {
  return invokeSnapshot("select_whisper_model", {
    modelId: modelIdSchema.parse(modelId),
  });
}

export function chooseCustomWhisperModel(): Promise<TranscriptionSnapshot> {
  return invokeSnapshot("choose_custom_whisper_model");
}

export function chooseTranscriptionFolder(
  folder: TranscriptionFolder,
): Promise<TranscriptionSnapshot> {
  return invokeSnapshot("choose_transcription_folder", {
    folder: transcriptionFolderSchema.parse(folder),
  });
}

export function resetTranscriptionFolder(
  folder: TranscriptionFolder,
): Promise<TranscriptionSnapshot> {
  return invokeSnapshot("reset_transcription_folder", {
    folder: transcriptionFolderSchema.parse(folder),
  });
}

export async function openTranscriptionFolder(
  folder: TranscriptionFolder,
): Promise<void> {
  await invoke("open_transcription_folder", {
    folder: transcriptionFolderSchema.parse(folder),
  });
}

export function downloadWhisperModel(
  modelId: string,
): Promise<TranscriptionSnapshot> {
  return invokeSnapshot("download_whisper_model", {
    modelId: modelIdSchema.parse(modelId),
  });
}

export function cancelWhisperModelDownload(): Promise<TranscriptionSnapshot> {
  return invokeSnapshot("cancel_whisper_model_download");
}

export function transcribeNow(): Promise<TranscriptionSnapshot> {
  return invokeSnapshot("transcribe_now");
}

export function cancelTranscription(): Promise<TranscriptionSnapshot> {
  return invokeSnapshot("cancel_transcription");
}

export function listenForTranscriptionSnapshots(
  onSnapshot: (snapshot: TranscriptionSnapshot) => void,
): Promise<UnlistenFn> {
  return listen(snapshotEvent, (event) => {
    onSnapshot(transcriptionSnapshotSchema.parse(event.payload));
  });
}

export const transcriptionClient = {
  getTranscriptionSnapshot,
  setTranscriptionEnabled,
  saveTranscriptionOptions,
  saveTranscriptionDictionary,
  saveTranslationSettings,
  selectWhisperModel,
  chooseCustomWhisperModel,
  chooseTranscriptionFolder,
  resetTranscriptionFolder,
  openTranscriptionFolder,
  downloadWhisperModel,
  cancelWhisperModelDownload,
  transcribeNow,
  cancelTranscription,
};

export type TranscriptionActions = typeof transcriptionClient;
