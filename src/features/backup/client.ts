import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { z } from "zod";
import {
  appSnapshotSchema,
  trashProposalSummarySchema,
  pairingAssignmentsSchema,
  transmitterSchema,
  type AppSnapshot,
  type TrashProposalSummary,
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

export async function prepareTrash(transmitter: Transmitter): Promise<TrashProposalSummary> {
  const safeTransmitter = transmitterSchema.parse(transmitter);
  return trashProposalSummarySchema.parse(
    await invoke("prepare_trash", { transmitter: safeTransmitter }),
  );
}

export function confirmTrash(proposalId: string): Promise<AppSnapshot> {
  return invokeSnapshot("confirm_trash", {
    proposalId: proposalIdSchema.parse(proposalId),
  });
}

export function setAutostart(enabled: boolean): Promise<AppSnapshot> {
  return invokeSnapshot("set_autostart", { enabled: z.boolean().parse(enabled) });
}

export async function showSettings(): Promise<void> {
  await invoke("show_settings");
}

export function setAutomaticBackup(enabled: boolean): Promise<AppSnapshot> {
  return invokeSnapshot("set_automatic_backup", { enabled: z.boolean().parse(enabled) });
}

export function setM4aConversion(enabled: boolean): Promise<AppSnapshot> {
  return invokeSnapshot("set_m4a_conversion", { enabled: z.boolean().parse(enabled) });
}

export function setAutomaticTrash(
  enabled: boolean,
  acknowledged: boolean,
): Promise<AppSnapshot> {
  return invokeSnapshot("set_automatic_trash", {
    enabled: z.boolean().parse(enabled),
    acknowledged: z.boolean().parse(acknowledged),
  });
}

export async function openDestination(): Promise<void> {
  await invoke("open_destination");
}

export async function openLogs(): Promise<void> {
  await invoke("open_logs");
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
  prepareTrash,
  confirmTrash,
  setAutostart,
  showSettings,
  setAutomaticBackup,
  setM4aConversion,
  setAutomaticTrash,
  openDestination,
  openLogs,
  quitApp,
};

export type BackupActions = typeof backupClient;
