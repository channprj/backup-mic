import { describe, expect, test } from "vitest";
import backupComplete from "../../../../contracts/fixtures/backup-complete.json";
import backupCopying from "../../../../contracts/fixtures/backup-copying.json";
import trashProposal from "../../../../contracts/fixtures/trash-proposal.json";
import destinationFull from "../../../../contracts/fixtures/error-destination-full.json";
import partialTrash from "../../../../contracts/fixtures/partial-trash.json";
import {
  appSnapshotSchema,
  backupRuleDraftSchema,
  ruleTestResultSchema,
  trashProposalSummarySchema,
} from "../contracts";

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
  sources: [],
  backup_rules: backupComplete.backup_rules,
  current_stage: null,
  failure_stage: null,
  setting_applies_next_run: false,
  current_item_ordinal: null,
  last_success_at: null,
  artifact_format: "m4a",
  retirement_mode: "manual",
  current_log_available: false,
  destination_display: null,
  settings: {
    automatic_backup: true,
    m4a_conversion: true,
    automatic_trash: false,
    autostart: false,
    free_space_reserve_gib: 10,
    rescan_interval_seconds: 15,
  },
  notification_status: "unknown",
  setup_state: "needs_destination",
  recent_activity: [],
  error: null,
} as const;

describe("app snapshot contract", () => {
  test("accepts every committed Rust snapshot fixture", () => {
    for (const fixture of [
      backupComplete,
      backupCopying,
      destinationFull,
      partialTrash,
    ]) {
      expect(appSnapshotSchema.safeParse(fixture).success).toBe(true);
    }
    expect(trashProposalSummarySchema.safeParse(trashProposal).success).toBe(
      true,
    );
  });

  test("accepts exactly the six public backup stages", () => {
    for (const current_stage of [
      "copy",
      "source_verification",
      "conversion",
      "artifact_verification",
      "source_revalidation",
      "trash",
    ]) {
      expect(
        appSnapshotSchema.safeParse({ ...baseSnapshot, current_stage }).success,
      ).toBe(true);
    }
    expect(
      appSnapshotSchema.safeParse({
        ...baseSnapshot,
        current_stage: "sha256_verification",
      }).success,
    ).toBe(false);
  });

  test("requires display-safe artifact, retirement, settings, and log fields", () => {
    const parsed = appSnapshotSchema.parse(baseSnapshot);
    expect(parsed.artifact_format).toBe("m4a");
    expect(parsed.retirement_mode).toBe("manual");
    expect(parsed.current_log_available).toBe(false);
    expect(parsed.destination_display).toBeNull();
    expect(parsed.failure_stage).toBeNull();
    expect(parsed.setting_applies_next_run).toBe(false);
    expect(parsed.settings.automatic_trash).toBe(false);
    expect(parsed.backup_rules[0].is_dji_preset).toBe(true);
    expect(parsed.backup_rules[0].date_folder_layout).toBe("year_month_day");
  });

  test("bounds the numeric settings to the range Rust accepts", () => {
    const parsed = appSnapshotSchema.parse(baseSnapshot);
    expect(parsed.settings.free_space_reserve_gib).toBe(10);
    expect(parsed.settings.rescan_interval_seconds).toBe(15);

    // Rust refuses anything outside these ranges before it writes, so a snapshot carrying an
    // out-of-range value means the two sides disagree and must not be rendered.
    for (const settings of [
      { free_space_reserve_gib: 0 },
      { free_space_reserve_gib: 513 },
      { free_space_reserve_gib: 10.5 },
      { rescan_interval_seconds: 4 },
      { rescan_interval_seconds: 3601 },
    ]) {
      expect(
        appSnapshotSchema.safeParse({
          ...baseSnapshot,
          settings: { ...baseSnapshot.settings, ...settings },
        }).success,
      ).toBe(false);
    }

    // Both ends of each range are valid, and so is a value between the offered presets.
    for (const settings of [
      { free_space_reserve_gib: 1 },
      { free_space_reserve_gib: 512 },
      { rescan_interval_seconds: 5 },
      { rescan_interval_seconds: 47 },
      { rescan_interval_seconds: 3600 },
    ]) {
      expect(
        appSnapshotSchema.safeParse({
          ...baseSnapshot,
          settings: { ...baseSnapshot.settings, ...settings },
        }).success,
      ).toBe(true);
    }
  });

  test("requires an explicit nullable error for every recorder source", () => {
    const source = backupComplete.sources[0];
    expect(appSnapshotSchema.safeParse(backupComplete).success).toBe(true);
    const { error: _error, ...withoutError } = source;
    expect(
      appSnapshotSchema.safeParse({
        ...backupComplete,
        sources: [withoutError],
      }).success,
    ).toBe(false);
  });

  test("accepts only the three date folder layouts and defaults legacy drafts", () => {
    const draft = {
      id: null,
      name: "Zoom",
      archive_directory_name: "Zoom",
      enabled: true,
      volume_name_glob: "ZOOM_*",
      required_path_globs: [],
      backup_file_globs: ["*.WAV"],
      session_directory_globs: [],
      filename_prefix: "",
      filename_suffix: "",
    };

    expect(backupRuleDraftSchema.parse(draft).date_folder_layout).toBe(
      "year_month_day",
    );
    for (const date_folder_layout of [
      "year_month_day",
      "year_month",
      "compact_date",
    ]) {
      expect(
        backupRuleDraftSchema.safeParse({ ...draft, date_folder_layout })
          .success,
      ).toBe(true);
    }
    expect(
      backupRuleDraftSchema.safeParse({
        ...draft,
        date_folder_layout: "custom",
      }).success,
    ).toBe(false);
  });

  test("accepts the pending settings-review state and bounds the display-only destination", () => {
    expect(
      appSnapshotSchema.safeParse({
        ...baseSnapshot,
        setup_state: "needs_settings_review",
        destination_display: "/Volumes/Archive/Backup Mic",
      }).success,
    ).toBe(true);
    expect(
      appSnapshotSchema.safeParse({
        ...baseSnapshot,
        destination_display: "x".repeat(2_049),
      }).success,
    ).toBe(false);
  });

  test("rejects more activity entries than the popover contract allows", () => {
    const activity = {
      occurred_at: "2026-08-09T00:00:00Z",
      code: "device_detected",
      source_id: "11111111-1111-4111-8111-111111111111",
      source_label: "MIC_TX",
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

  test("bounds and validates editable rule drafts", () => {
    const draft = {
      id: null,
      name: "Zoom H1n",
      archive_directory_name: "Zoom H1n",
      enabled: true,
      volume_name_glob: "ZOOM_*",
      required_path_globs: ["RECORD/**"],
      backup_file_globs: ["RECORD/**/*.WAV"],
      session_directory_globs: ["RECORD/*"],
      filename_prefix: "zoom-",
      filename_suffix: "-field",
      date_folder_layout: "year_month",
    };
    expect(backupRuleDraftSchema.parse(draft)).toEqual(draft);
    expect(
      backupRuleDraftSchema.safeParse({ ...draft, filename_suffix: "/private" })
        .success,
    ).toBe(false);
    expect(
      backupRuleDraftSchema.safeParse({ ...draft, backup_file_globs: [] })
        .success,
    ).toBe(false);
    expect(
      backupRuleDraftSchema.safeParse({ ...draft, arbitrary_pattern: "**/*" })
        .success,
    ).toBe(false);
  });

  test("keeps rule test results display-only", () => {
    expect(
      ruleTestResultSchema.safeParse({
        matched_volumes: ["ZOOM_TEST"],
        matched_file_count: 3,
        conflict_rule_names: [],
      }).success,
    ).toBe(true);
    expect(
      ruleTestResultSchema.safeParse({
        matched_volumes: ["ZOOM_TEST"],
        matched_file_count: 3,
        conflict_rule_names: [],
        volume_uuid: "private",
      }).success,
    ).toBe(false);
  });

  test("keeps fixtures free from sensitive diagnostics and deletion authority", () => {
    const serialized = JSON.stringify([
      backupComplete,
      backupCopying,
      destinationFull,
      partialTrash,
    ]);
    for (const forbidden of [
      "/Volumes/",
      "source_path",
      "destination_path",
      "sha256",
      "device_uuid",
      "proposal_context",
      "tool_output",
    ]) {
      expect(serialized).not.toContain(forbidden);
    }
  });
});
