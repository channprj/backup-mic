import { useEffect, useState } from "react";
import { errorCopy } from "../backup/format";
import type { ActionError } from "../backup/useSettingsPersistence";
import type { TranscriptionActions } from "./client";
import type { TranscriptionOptions, TranscriptionSnapshot } from "./contracts";

/** The parts of the view a pending write can cover; each disables its own controls. */
export type PendingKey =
  | "enabled"
  | "options"
  | "model"
  | "output_folder"
  | "models_folder"
  | "download"
  | "queue";

type EditableFields = Pick<
  TranscriptionSnapshot,
  "enabled" | "include_existing_recordings" | "formats" | "whisper"
>;

function commandError(error: unknown, title: string): ActionError {
  const messageCode =
    typeof error === "object" && error !== null && "message_code" in error
      ? String(error.message_code)
      : "setting_save_failed";
  return { messageCode, ...errorCopy(messageCode), title };
}

/**
 * Owns the transcribe tab's view of Rust state and every write it can make.
 *
 * The same contract as the backup settings: a control moves immediately, settles on the
 * snapshot Rust returns, and goes back to its previous value when the write fails. The view only
 * moves forward by revision, so a slow reply cannot overwrite a newer event.
 */
export function useTranscriptionPersistence(
  snapshot: TranscriptionSnapshot,
  actions: TranscriptionActions,
) {
  const [view, setView] = useState(snapshot);
  const [pending, setPending] = useState<Set<PendingKey>>(new Set());
  const [actionError, setActionError] = useState<ActionError | null>(null);

  useEffect(() => {
    setView((current) =>
      snapshot.revision > current.revision ? snapshot : current,
    );
  }, [snapshot]);

  function markPending(key: PendingKey, value: boolean) {
    setPending((current) => {
      const next = new Set(current);
      if (value) next.add(key);
      else next.delete(key);
      return next;
    });
  }

  async function run(
    key: PendingKey,
    title: string,
    operation: () => Promise<TranscriptionSnapshot | void>,
    patch?: Partial<EditableFields>,
  ) {
    if (pending.has(key)) return;
    const previous: Partial<EditableFields> = {};
    for (const field of Object.keys(patch ?? {}) as (keyof EditableFields)[]) {
      Object.assign(previous, { [field]: view[field] });
    }
    setActionError(null);
    markPending(key, true);
    if (patch) setView((current) => ({ ...current, ...patch }));
    try {
      const persisted = await operation();
      if (persisted) {
        setView((current) =>
          persisted.revision < current.revision ? current : persisted,
        );
      }
    } catch (error) {
      if (patch) setView((current) => ({ ...current, ...previous }));
      setActionError(commandError(error, title));
    } finally {
      markPending(key, false);
    }
  }

  function optionsWith(
    change: Partial<TranscriptionOptions>,
  ): TranscriptionOptions {
    return {
      include_existing_recordings: view.include_existing_recordings,
      formats: view.formats,
      whisper: view.whisper,
      ...change,
    };
  }

  return {
    view,
    pending,
    actionError,
    setEnabled: (enabled: boolean) =>
      run(
        "enabled",
        "전사 설정을 저장하지 못했습니다",
        () => actions.setTranscriptionEnabled(enabled),
        { enabled },
      ),
    saveOptions: (change: Partial<TranscriptionOptions>) => {
      const options = optionsWith(change);
      return run(
        "options",
        "전사 옵션을 저장하지 못했습니다",
        () => actions.saveTranscriptionOptions(options),
        options,
      );
    },
    selectModel: (modelId: string) =>
      run("model", "모델을 선택하지 못했습니다", () =>
        actions.selectWhisperModel(modelId),
      ),
    chooseCustomModel: () =>
      run("model", "모델 파일을 선택하지 못했습니다", () =>
        actions.chooseCustomWhisperModel(),
      ),
    chooseFolder: (folder: "output" | "models") =>
      run(`${folder}_folder`, "폴더를 변경하지 못했습니다", () =>
        actions.chooseTranscriptionFolder(folder),
      ),
    resetFolder: (folder: "output" | "models") =>
      run(`${folder}_folder`, "폴더를 되돌리지 못했습니다", () =>
        actions.resetTranscriptionFolder(folder),
      ),
    openFolder: (folder: "output" | "models") =>
      run(`${folder}_folder`, "폴더를 열지 못했습니다", () =>
        actions.openTranscriptionFolder(folder),
      ),
    download: (modelId: string) =>
      run("download", "모델을 다운로드하지 못했습니다", () =>
        actions.downloadWhisperModel(modelId),
      ),
    cancelDownload: () =>
      run("download", "다운로드를 중단하지 못했습니다", () =>
        actions.cancelWhisperModelDownload(),
      ),
    transcribeNow: () =>
      run("queue", "전사를 시작하지 못했습니다", () => actions.transcribeNow()),
    cancelTranscription: () =>
      run("queue", "전사를 중단하지 못했습니다", () =>
        actions.cancelTranscription(),
      ),
  };
}
