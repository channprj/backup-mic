import { StrictMode, useMemo, useState } from "react";
import { createRoot } from "react-dom/client";
import completeFixture from "../contracts/fixtures/backup-complete.json";
import copyingFixture from "../contracts/fixtures/backup-copying.json";
import errorFixture from "../contracts/fixtures/error-destination-full.json";
import "./index.css";
import { BackupPopover } from "./features/backup/BackupPopover";
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
  const actions = useMemo<BackupActions>(
    () => ({
      backupNow: async () => setSnapshot(copying),
      chooseDestination: async () => snapshot,
      pairDevices: async () => {
        setSnapshot(complete);
        return complete;
      },
      prepareDeletion: async (transmitter) => ({
        proposal_id: "550e8400-e29b-41d4-a716-446655440000",
        transmitter,
        file_count: 9,
        byte_count: 99_000_000,
        destination_summary: "DJI-Mic-Mini-2S 백업 폴더",
        expires_at: "2026-08-09T02:25:00Z",
      }),
      confirmDeletion: async () => snapshot,
      setAutostart: async (enabled) => {
        const next = { ...snapshot, autostart_enabled: enabled };
        setSnapshot(next);
        return next;
      },
      openDestination: async () => undefined,
      quitApp: async () => undefined,
    }),
    [snapshot],
  );

  return <BackupPopover snapshot={snapshot} actions={actions} />;
}

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <Preview />
  </StrictMode>,
);
