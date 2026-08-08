import { describe, expect, test } from "vitest";
import backupComplete from "../../../../contracts/fixtures/backup-complete.json";
import backupCopying from "../../../../contracts/fixtures/backup-copying.json";
import deletionProposal from "../../../../contracts/fixtures/deletion-proposal.json";
import destinationFull from "../../../../contracts/fixtures/error-destination-full.json";
import partialDeletion from "../../../../contracts/fixtures/partial-deletion.json";
import { appSnapshotSchema, deletionProposalSummarySchema } from "../contracts";

const baseSnapshot = {
  revision: 1,
  phase: "idle",
  message_code: "idle",
  overall_progress: {
    percent: 0,
    copied_bytes: 0,
    bytes_requiring_copy: 0,
    verified_files: 0,
    total_files: 0,
  },
  transmitters: (["TX01", "TX02"] as const).map((transmitter) => ({
    transmitter,
    mounted: false,
    phase: "idle" as const,
    progress: {
      percent: 0,
      copied_bytes: 0,
      bytes_requiring_copy: 0,
      verified_files: 0,
      total_files: 0,
    },
    deletion_phase: "inactive" as const,
    deletion_ready: false,
  })),
  current_stage: null,
  current_item_ordinal: null,
  last_success_at: null,
  autostart_enabled: false,
  notification_status: "unknown",
  setup_state: "needs_destination",
  pairing_candidates: [],
  recent_activity: [],
  error: null,
} as const;

describe("app snapshot contract", () => {
  test("accepts every committed Rust snapshot fixture", () => {
    for (const fixture of [backupComplete, backupCopying, destinationFull, partialDeletion]) {
      expect(appSnapshotSchema.safeParse(fixture).success).toBe(true);
    }
    expect(deletionProposalSummarySchema.safeParse(deletionProposal).success).toBe(true);
  });

  test("rejects more activity entries than the popover contract allows", () => {
    const activity = {
      occurred_at: "2026-08-09T00:00:00Z",
      code: "device_detected",
      transmitter: "TX01",
      count_value: null,
      byte_value: null,
      severity: "info",
    } as const;
    const result = appSnapshotSchema.safeParse({
      ...baseSnapshot,
      recent_activity: Array.from({ length: 9 }, () => activity),
    });
    expect(result.success).toBe(false);
  });

  test("rejects unknown fields that could smuggle authority", () => {
    const result = appSnapshotSchema.safeParse({
      ...baseSnapshot,
      source_path: "/Volumes/example/recording.wav",
    });
    expect(result.success).toBe(false);
  });
});
