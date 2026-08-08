import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { z } from "zod";
import {
  appSnapshotSchema,
  deletionProposalSummarySchema,
  pairingAssignmentsSchema,
  transmitterSchema,
  type AppSnapshot,
  type DeletionProposalSummary,
  type PairingAssignment,
  type Transmitter,
} from "./contracts";

const proposalIdSchema = z.string().uuid();
const snapshotEvent = "app-snapshot-changed";

async function invokeSnapshot(command: string, args?: Record<string, unknown>) {
  return appSnapshotSchema.parse(await invoke(command, args));
}

export function getAppSnapshot(): Promise<AppSnapshot> {
  return invokeSnapshot("get_app_snapshot");
}

export async function backupNow(): Promise<void> {
  await invoke("backup_now");
}

export function chooseDestination(): Promise<AppSnapshot> {
  return invokeSnapshot("choose_destination");
}

export async function pairDevices(assignments: PairingAssignment[]): Promise<AppSnapshot> {
  const safeAssignments = pairingAssignmentsSchema.parse(assignments);
  return await invokeSnapshot("pair_devices", { assignments: safeAssignments });
}

export async function prepareDeletion(
  transmitter: Transmitter,
): Promise<DeletionProposalSummary> {
  const safeTransmitter = transmitterSchema.parse(transmitter);
  return deletionProposalSummarySchema.parse(
    await invoke("prepare_deletion", { transmitter: safeTransmitter }),
  );
}

export function confirmDeletion(proposalId: string): Promise<AppSnapshot> {
  return invokeSnapshot("confirm_deletion", {
    proposalId: proposalIdSchema.parse(proposalId),
  });
}

export function setAutostart(enabled: boolean): Promise<AppSnapshot> {
  return invokeSnapshot("set_autostart", { enabled: z.boolean().parse(enabled) });
}

export async function openDestination(): Promise<void> {
  await invoke("open_destination");
}

export async function quitApp(): Promise<void> {
  await invoke("quit_app");
}

export function listenForSnapshots(
  onSnapshot: (snapshot: AppSnapshot) => void,
): Promise<UnlistenFn> {
  return listen(snapshotEvent, (event) => {
    onSnapshot(appSnapshotSchema.parse(event.payload));
  });
}

export const backupClient = {
  backupNow,
  chooseDestination,
  pairDevices,
  prepareDeletion,
  confirmDeletion,
  setAutostart,
  openDestination,
  quitApp,
};

export type BackupActions = typeof backupClient;
