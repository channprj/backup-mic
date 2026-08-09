import { StrictMode, useMemo, useState } from "react";
import { createRoot } from "react-dom/client";
import completeFixture from "../contracts/fixtures/backup-complete.json";
import copyingFixture from "../contracts/fixtures/backup-copying.json";
import errorFixture from "../contracts/fixtures/error-destination-full.json";
import "./index.css";
import { BackupPopover } from "./features/backup/BackupPopover";
import { SettingsView } from "./features/backup/SettingsApp";
import type { BackupActions } from "./features/backup/client";
import { appSnapshotSchema, type AppSnapshot } from "./features/backup/contracts";

const complete = appSnapshotSchema.parse(completeFixture);
const copying = appSnapshotSchema.parse(copyingFixture);
const failure = appSnapshotSchema.parse(errorFixture);
const pairing = appSnapshotSchema.parse({
  ...completeFixture,
  phase: "idle",
  setup_state: "needs_pairing",
  pairing_candidates: [
    {
      candidate_id: "11111111-1111-4111-8111-111111111111",
      display_name: "DJI 저장 장치 A",
      capacity_bytes: 15_636_365_312,
    },
    {
      candidate_id: "22222222-2222-4222-8222-222222222222",
      display_name: "DJI 저장 장치 B",
      capacity_bytes: 15_636_365_312,
    },
  ],
});

const previews: Record<string, AppSnapshot> = { complete, copying, error: failure, pairing };

function Preview() {
  const initial = new URLSearchParams(window.location.search).get("state") ?? "complete";
  const [snapshot, setSnapshot] = useState(previews[initial] ?? complete);
  const settingsWindow = new URLSearchParams(window.location.search).get("window") === "settings";
  const actions = useMemo<BackupActions>(
    () => {
      const withSetting = (
        key: keyof AppSnapshot["settings"],
        enabled: boolean,
      ): AppSnapshot => {
        const next: AppSnapshot = {
          ...snapshot,
          revision: snapshot.revision + 1,
          artifact_format:
            key === "m4a_conversion" ? (enabled ? "m4a" : "wav") : snapshot.artifact_format,
          retirement_mode:
            key === "automatic_trash"
              ? enabled
                ? "automatic"
                : "manual"
              : snapshot.retirement_mode,
          settings: { ...snapshot.settings, [key]: enabled },
        };
        setSnapshot(next);
        return next;
      };
      return {
      backupNow: async () => setSnapshot(copying),
      chooseDestination: async () => snapshot,
      pairDevices: async () => {
        setSnapshot(complete);
        return complete;
      },
      prepareTrash: async (transmitter) => ({
        proposal_id: "550e8400-e29b-41d4-a716-446655440000",
        transmitter,
        session_count: 1,
        file_count: 9,
        byte_count: 99_000_000,
        destination_summary: "DJI-Mic-Mini-2S 백업 폴더",
        expires_at: new Date(Date.now() + 5 * 60 * 1_000).toISOString(),
      }),
      confirmTrash: async () => snapshot,
      setAutostart: async (enabled) => withSetting("autostart", enabled),
      showSettings: async () => undefined,
      setAutomaticBackup: async (enabled) => withSetting("automatic_backup", enabled),
      setM4aConversion: async (enabled) => withSetting("m4a_conversion", enabled),
      setAutomaticTrash: async (enabled) => withSetting("automatic_trash", enabled),
      openDestination: async () => undefined,
      openLogs: async () => undefined,
      quitApp: async () => undefined,
    };
    },
    [snapshot],
  );

  return settingsWindow ? (
    <SettingsView snapshot={snapshot} actions={actions} />
  ) : (
    <BackupPopover snapshot={snapshot} actions={actions} />
  );
}

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <Preview />
  </StrictMode>,
);
