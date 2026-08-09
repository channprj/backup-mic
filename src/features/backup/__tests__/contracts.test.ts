import { describe, expect, test } from "vitest";
import backupComplete from "../../../../contracts/fixtures/backup-complete.json";
import backupCopying from "../../../../contracts/fixtures/backup-copying.json";
import trashProposal from "../../../../contracts/fixtures/trash-proposal.json";
import destinationFull from "../../../../contracts/fixtures/error-destination-full.json";
import partialTrash from "../../../../contracts/fixtures/partial-trash.json";
import { appSnapshotSchema, trashProposalSummarySchema } from "../contracts";

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
    retirement_outcome: "inactive" as const,
    deletion_ready: false,
  })),
  current_stage: null,
  current_item_ordinal: null,
  last_success_at: null,
  artifact_format: "m4a",
  retirement_mode: "manual",
  current_log_available: false,
  settings: {
    automatic_backup: true,
    m4a_conversion: true,
    automatic_trash: false,
    autostart: false,
  },
  notification_status: "unknown",
  setup_state: "needs_destination",
  pairing_candidates: [],
  recent_activity: [],
  error: null,
} as const;

describe("app snapshot contract", () => {
  test("accepts every committed Rust snapshot fixture", () => {
    for (const fixture of [backupComplete, backupCopying, destinationFull, partialTrash]) {
      expect(appSnapshotSchema.safeParse(fixture).success).toBe(true);
    }
    expect(trashProposalSummarySchema.safeParse(trashProposal).success).toBe(true);
  });

  test("accepts exactly the five public backup stages", () => {
    for (const current_stage of [
      "copy",
      "source_verification",
      "conversion",
      "artifact_verification",
      "trash",
    ]) {
      expect(appSnapshotSchema.safeParse({ ...baseSnapshot, current_stage }).success).toBe(true);
    }
    expect(
      appSnapshotSchema.safeParse({ ...baseSnapshot, current_stage: "sha256_verification" }).success,
    ).toBe(false);
  });

  test("requires display-safe artifact, retirement, settings, and log fields", () => {
    const parsed = appSnapshotSchema.parse(baseSnapshot);
    expect(parsed.artifact_format).toBe("m4a");
    expect(parsed.retirement_mode).toBe("manual");
    expect(parsed.current_log_available).toBe(false);
    expect(parsed.settings.automatic_trash).toBe(false);
    expect(parsed.transmitters[0].retirement_outcome).toBe("inactive");
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
