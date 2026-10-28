import { StrictMode, useMemo, useState } from "react";
import { createRoot } from "react-dom/client";
import completeFixture from "../contracts/fixtures/backup-complete.json";
import copyingFixture from "../contracts/fixtures/backup-copying.json";
import errorFixture from "../contracts/fixtures/error-destination-full.json";
import transcriptionFixture from "../contracts/fixtures/transcription-snapshot.json";
import "./index.css";
import { BackupPopover } from "./features/backup/BackupPopover";
import { SettingsView } from "./features/backup/SettingsApp";
import type { BackupActions } from "./features/backup/client";
import { appSnapshotSchema, type AppSnapshot } from "./features/backup/contracts";
import { TranscriptionView } from "./features/transcription/TranscriptionPanel";
import type { TranscriptionActions } from "./features/transcription/client";
import {
  transcriptionSnapshotSchema,
  type TranscriptionSnapshot,
} from "./features/transcription/contracts";

const complete = appSnapshotSchema.parse(completeFixture);
const copying = appSnapshotSchema.parse(copyingFixture);
const failure = appSnapshotSchema.parse(errorFixture);
const setup = appSnapshotSchema.parse({
  ...completeFixture,
  phase: "idle",
  setup_state: "needs_destination",
});

const previews: Record<string, AppSnapshot> = { complete, copying, error: failure, setup };

function Preview() {
  const initial = new URLSearchParams(window.location.search).get("state") ?? "complete";
  const [snapshot, setSnapshot] = useState(previews[initial] ?? complete);
  const settingsWindow =
    initial === "rules" || new URLSearchParams(window.location.search).get("window") === "settings";
  const actions = useMemo<BackupActions>(
    () => {
      const withSetting = <Key extends keyof AppSnapshot["settings"]>(
        key: Key,
        value: AppSnapshot["settings"][Key],
      ): AppSnapshot => {
        const next: AppSnapshot = {
          ...snapshot,
          revision: snapshot.revision + 1,
          artifact_format:
            key === "m4a_conversion" ? (value ? "m4a" : "wav") : snapshot.artifact_format,
          retirement_mode:
            key === "automatic_trash"
              ? value
                ? "automatic"
                : "manual"
              : snapshot.retirement_mode,
          settings: { ...snapshot.settings, [key]: value },
        };
        setSnapshot(next);
        return next;
      };
      return {
      backupNow: async () => setSnapshot(copying),
      cancelBackup: async () => {
        setSnapshot((current) => ({
          ...current,
          revision: current.revision + 1,
          phase: "idle",
          message_code: "operation_cancelled",
          current_stage: null,
          failure_stage: null,
          error: null,
        }));
      },
      chooseDestination: async () => {
        const next: AppSnapshot = {
          ...snapshot,
          revision: snapshot.revision + 1,
          destination_display: "~/Documents/Backup Mic",
          setup_state:
            snapshot.setup_state === "needs_destination"
              ? "needs_settings_review"
              : snapshot.setup_state,
        };
        setSnapshot(next);
        return next;
      },
      completeInitialSetup: async () => {
        const next: AppSnapshot = {
          ...snapshot,
          revision: snapshot.revision + 1,
          setup_state: "ready",
        };
        setSnapshot(next);
        return next;
      },
      saveBackupRule: async () => snapshot,
      archiveBackupRule: async () => snapshot,
      restoreDjiRule: async () => snapshot,
      testBackupRule: async () => ({
        matched_volumes: [],
        matched_file_count: 0,
        conflict_rule_names: [],
      }),
      prepareTrash: async (sourceId) => ({
        proposal_id: "550e8400-e29b-41d4-a716-446655440000",
        source_id: sourceId,
        source_label:
          snapshot.sources.find(({ source_id }) => source_id === sourceId)?.volume_name ??
          "External Recorder",
        session_count: 1,
        file_count: 9,
        byte_count: 99_000_000,
        destination_summary: "Backup Mic 백업 폴더",
        expires_at: new Date(Date.now() + 5 * 60 * 1_000).toISOString(),
      }),
      confirmTrash: async () => snapshot,
      setAutostart: async (enabled) => withSetting("autostart", enabled),
      showSettings: async () => undefined,
      setAutomaticBackup: async (enabled) => withSetting("automatic_backup", enabled),
      setM4aConversion: async (enabled) => withSetting("m4a_conversion", enabled),
      setAutomaticTrash: async (enabled) => withSetting("automatic_trash", enabled),
      setFreeSpaceReserve: async (gibibytes) =>
        withSetting("free_space_reserve_gib", gibibytes),
      setRescanInterval: async (seconds) => withSetting("rescan_interval_seconds", seconds),
      openDestination: async () => undefined,
      openLogs: async () => undefined,
      quitApp: async () => undefined,
    };
    },
    [snapshot],
  );

  return settingsWindow ? (
    <SettingsView
      snapshot={snapshot}
      actions={actions}
      transcription={<TranscriptionPreview />}
      initialTab={
        new URLSearchParams(window.location.search).get("tab") === "transcription"
          ? "transcription"
          : "backup"
      }
    />
  ) : (
    <BackupPopover snapshot={snapshot} actions={actions} />
  );
}

/** The transcribe tab over the committed fixture, with every write applied locally. */
function TranscriptionPreview() {
  const [snapshot, setSnapshot] = useState<TranscriptionSnapshot>(() =>
    transcriptionSnapshotSchema.parse(transcriptionFixture),
  );
  const actions = useMemo<TranscriptionActions>(() => {
    const apply = async (change: Partial<TranscriptionSnapshot>) => {
      const next = { ...snapshot, ...change, revision: snapshot.revision + 1 };
      setSnapshot(next);
      return next;
    };
    return {
      getTranscriptionSnapshot: async () => snapshot,
      setTranscriptionEnabled: (enabled) => apply({ enabled }),
      saveTranscriptionOptions: (options) => apply(options),
      saveTranscriptionDictionary: ({ enabled, entries }) =>
        apply({ dictionary_enabled: enabled, dictionary: entries }),
      saveTranslationSettings: (translation) => apply({ translation }),
      selectWhisperModel: (id) =>
        apply({
          selected_model: {
            ...snapshot.selected_model,
            source: "catalog",
            id,
            label: snapshot.models.find((model) => model.id === id)?.label ?? id,
            ready: snapshot.models.some((model) => model.id === id && model.installed),
          },
        }),
      chooseCustomWhisperModel: async () => snapshot,
      chooseTranscriptionFolder: async () => snapshot,
      resetTranscriptionFolder: async () => snapshot,
      openTranscriptionFolder: async () => undefined,
      downloadWhisperModel: (id) =>
        apply({
          download: {
            model_id: id,
            received_bytes: 0,
            total_bytes: snapshot.models.find((model) => model.id === id)?.byte_count ?? 1,
          },
        }),
      cancelWhisperModelDownload: () => apply({ download: null }),
      transcribeNow: async () => snapshot,
      cancelTranscription: () =>
        apply({ job: { ...snapshot.job, phase: "idle", current_item: null, percent: 0 } }),
    };
  }, [snapshot]);
  return <TranscriptionView snapshot={snapshot} actions={actions} />;
}

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <Preview />
  </StrictMode>,
);
