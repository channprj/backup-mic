import { z } from "zod";

const opaqueIdSchema = z.string().uuid();
const safeLabelSchema = z.string().min(1).max(128);
const ruleTextSchema = z.string().trim().min(1).max(64);
const optionalRuleTextSchema = z
  .string()
  .max(64)
  .refine((value) => !/[\\/\0]/u.test(value), "Must not contain path separators");
const globSchema = z.string().min(1).max(256);
const optionalPatternsSchema = z.array(globSchema).max(32);
const requiredPatternsSchema = optionalPatternsSchema.min(1);

export const backupPhaseSchema = z.enum([
  "idle",
  "detecting",
  "scanning",
  "checking_capacity",
  "copying",
  "verifying",
  "completed_deletion_pending",
  "nothing_new",
  "partial_failure",
  "error",
]);
export const retirementOutcomeSchema = z.enum([
  "inactive",
  "preparing",
  "awaiting_confirmation",
  "revalidating",
  "moving_to_trash",
  "moved_to_trash",
  "refused",
  "partially_moved_to_trash",
]);
export const currentStageSchema = z.enum([
  "copy",
  "source_verification",
  "conversion",
  "artifact_verification",
  "source_revalidation",
  "trash",
]);
export const backupSettingsSchema = z
  .object({
    automatic_backup: z.boolean(),
    m4a_conversion: z.boolean(),
    automatic_trash: z.boolean(),
    autostart: z.boolean(),
  })
  .strict();
export const progressSchema = z
  .object({
    percent: z.number().int().min(0).max(100),
    copied_bytes: z.number().int().nonnegative(),
    bytes_requiring_copy: z.number().int().nonnegative(),
    verified_files: z.number().int().nonnegative(),
    total_files: z.number().int().nonnegative(),
  })
  .strict();

export const backupRuleDraftSchema = z
  .object({
    id: opaqueIdSchema.nullable(),
    name: ruleTextSchema,
    archive_directory_name: ruleTextSchema,
    enabled: z.boolean(),
    volume_name_glob: globSchema,
    required_path_globs: optionalPatternsSchema,
    backup_file_globs: requiredPatternsSchema,
    session_directory_globs: optionalPatternsSchema,
    filename_prefix: optionalRuleTextSchema,
    filename_suffix: optionalRuleTextSchema,
  })
  .strict();

export const backupRuleSchema = backupRuleDraftSchema
  .extend({
    id: opaqueIdSchema,
    archive_directory_locked: z.boolean(),
    is_dji_preset: z.boolean(),
    archived: z.boolean(),
  })
  .strict();

export const sourceSnapshotSchema = z
  .object({
    source_id: opaqueIdSchema,
    rule_name: ruleTextSchema,
    volume_name: safeLabelSchema,
    legacy_slot: z.string().max(64).nullable(),
    mounted: z.boolean(),
    phase: backupPhaseSchema,
    progress: progressSchema,
    retirement_outcome: retirementOutcomeSchema,
    deletion_ready: z.boolean(),
  })
  .strict();

export const ruleTestResultSchema = z
  .object({
    matched_volumes: z.array(safeLabelSchema).max(128),
    matched_file_count: z.number().int().nonnegative(),
    conflict_rule_names: z.array(ruleTextSchema).max(128),
  })
  .strict();

export const activitySchema = z
  .object({
    occurred_at: z.string(),
    code: z.string(),
    source_id: opaqueIdSchema.nullable(),
    source_label: safeLabelSchema.nullable(),
    count_value: z.number().int().nonnegative().nullable(),
    byte_value: z.number().int().nonnegative().nullable(),
    severity: z.enum(["info", "success", "warning", "error"]),
  })
  .strict();
export const publicErrorSchema = z
  .object({
    code: z.string(),
    message_code: z.string(),
    retryable: z.boolean(),
    source_id: opaqueIdSchema.nullable(),
    source_label: safeLabelSchema.nullable(),
  })
  .strict();

export const appSnapshotSchema = z
  .object({
    revision: z.number().int().nonnegative(),
    phase: backupPhaseSchema,
    message_code: z.string(),
    overall_progress: progressSchema,
    sources: z.array(sourceSnapshotSchema).max(128),
    backup_rules: z.array(backupRuleSchema).max(128),
    current_stage: currentStageSchema.nullable(),
    failure_stage: currentStageSchema.nullable(),
    setting_applies_next_run: z.boolean(),
    current_item_ordinal: z.number().int().positive().nullable(),
    last_success_at: z.string().nullable(),
    artifact_format: z.enum(["wav", "m4a"]),
    retirement_mode: z.enum(["manual", "automatic"]),
    current_log_available: z.boolean(),
    destination_display: z.string().min(1).max(2048).nullable(),
    settings: backupSettingsSchema,
    notification_status: z.enum(["unknown", "granted", "denied"]),
    setup_state: z.enum(["needs_destination", "needs_settings_review", "ready"]),
    recent_activity: z.array(activitySchema).max(8),
    error: publicErrorSchema.nullable(),
  })
  .strict();

export const trashProposalSummarySchema = z
  .object({
    proposal_id: opaqueIdSchema,
    source_id: opaqueIdSchema,
    source_label: safeLabelSchema,
    session_count: z.number().int().nonnegative(),
    file_count: z.number().int().nonnegative(),
    byte_count: z.number().int().nonnegative(),
    destination_summary: z.string().max(128),
    expires_at: z.string(),
  })
  .strict();

export type AppSnapshot = z.infer<typeof appSnapshotSchema>;
export type BackupRule = z.infer<typeof backupRuleSchema>;
export type BackupRuleDraft = z.infer<typeof backupRuleDraftSchema>;
export type RuleTestResult = z.infer<typeof ruleTestResultSchema>;
export type SourceSnapshot = z.infer<typeof sourceSnapshotSchema>;
export type TrashProposalSummary = z.infer<typeof trashProposalSummarySchema>;
