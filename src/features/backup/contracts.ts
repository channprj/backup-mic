import { z } from "zod";

export const transmitterSchema = z.enum(["TX01", "TX02"]);
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
export const deletionPhaseSchema = z.enum([
  "inactive",
  "preparing",
  "awaiting_confirmation",
  "revalidating",
  "deleting",
  "deleted",
  "refused",
  "partially_deleted",
]);
export const progressSchema = z
  .object({
    percent: z.number().int().min(0).max(100),
    copied_bytes: z.number().int().nonnegative(),
    bytes_requiring_copy: z.number().int().nonnegative(),
    verified_files: z.number().int().nonnegative(),
    total_files: z.number().int().nonnegative(),
  })
  .strict();
export const activitySchema = z
  .object({
    occurred_at: z.string(),
    code: z.string(),
    transmitter: transmitterSchema.nullable(),
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
    transmitter: transmitterSchema.nullable(),
  })
  .strict();
export const pairingCandidateSchema = z
  .object({
    candidate_id: z.string().uuid(),
    display_name: z.string(),
    capacity_bytes: z.number().int().nonnegative(),
  })
  .strict();
export const transmitterSnapshotSchema = z
  .object({
    transmitter: transmitterSchema,
    mounted: z.boolean(),
    phase: backupPhaseSchema,
    progress: progressSchema,
    deletion_phase: deletionPhaseSchema,
    deletion_ready: z.boolean(),
  })
  .strict();
export const appSnapshotSchema = z
  .object({
    revision: z.number().int().nonnegative(),
    phase: backupPhaseSchema,
    message_code: z.string(),
    overall_progress: progressSchema,
    transmitters: z.array(transmitterSnapshotSchema).length(2),
    current_stage: z.enum(["copy", "sha256_verification"]).nullable(),
    current_item_ordinal: z.number().int().positive().nullable(),
    last_success_at: z.string().nullable(),
    autostart_enabled: z.boolean(),
    notification_status: z.enum(["unknown", "granted", "denied"]),
    setup_state: z.enum(["needs_destination", "needs_pairing", "ready"]),
    pairing_candidates: z.array(pairingCandidateSchema).max(2),
    recent_activity: z.array(activitySchema).max(8),
    error: publicErrorSchema.nullable(),
  })
  .strict()
  .superRefine((snapshot, context) => {
    const labels = new Set(snapshot.transmitters.map(({ transmitter }) => transmitter));
    if (labels.size !== 2) {
      context.addIssue({
        code: "custom",
        path: ["transmitters"],
        message: "TX01 and TX02 must each appear exactly once",
      });
    }
  });

export const deletionProposalSummarySchema = z
  .object({
    proposal_id: z.string().uuid(),
    transmitter: transmitterSchema,
    file_count: z.number().int().nonnegative(),
    byte_count: z.number().int().nonnegative(),
    destination_summary: z.string(),
    expires_at: z.string(),
  })
  .strict();

export type AppSnapshot = z.infer<typeof appSnapshotSchema>;
export type DeletionProposalSummary = z.infer<typeof deletionProposalSummarySchema>;
export type Transmitter = z.infer<typeof transmitterSchema>;

export const pairingAssignmentSchema = z
  .object({
    candidate_id: z.string().uuid(),
    transmitter: transmitterSchema,
  })
  .strict();
export const pairingAssignmentsSchema = z
  .array(pairingAssignmentSchema)
  .length(2)
  .superRefine((assignments, context) => {
    if (new Set(assignments.map(({ candidate_id }) => candidate_id)).size !== assignments.length) {
      context.addIssue({ code: "custom", message: "Candidates must be unique" });
    }
    if (new Set(assignments.map(({ transmitter }) => transmitter)).size !== assignments.length) {
      context.addIssue({ code: "custom", message: "Transmitters must be unique" });
    }
  });

export type PairingAssignment = z.infer<typeof pairingAssignmentSchema>;
