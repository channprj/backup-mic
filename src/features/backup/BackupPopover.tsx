import { Separator } from "@/components/ui/separator";
import type { BackupActions } from "./client";
import type { AppSnapshot } from "./contracts";
import { isWaitingForDevice } from "./format";
import { ActivityFeed, PopoverFooter, PopoverHeader } from "./PopoverChrome";
import {
  ActiveStatus,
  ChannelRow,
  FailureAlert,
  FailureStatus,
  SettledStatus,
  StageSequence,
} from "./PopoverStatus";
import { SetupFlow } from "./SetupFlow";
import { TrashDialog } from "./TrashDialog";
import { usePopoverActions } from "./usePopoverActions";

/** Phases where a run is under way, so the popover shows progress rather than a summary. */
const activePhases = new Set([
  "detecting",
  "scanning",
  "checking_capacity",
  "copying",
  "verifying",
]);

interface BackupPopoverProps {
  snapshot: AppSnapshot;
  actions: BackupActions;
}

export function BackupPopover({ snapshot, actions }: BackupPopoverProps) {
  const setupComplete = snapshot.setup_state === "ready";
  const active = activePhases.has(snapshot.phase) || snapshot.current_stage !== null;
  const waitingForDevice = isWaitingForDevice(snapshot);
  const failed = snapshot.phase === "error" || snapshot.phase === "partial_failure";
  const mountedCount = snapshot.sources.filter(({ mounted }) => mounted).length;
  const {
    pending,
    busy,
    cancelRequested,
    proposal,
    actionError,
    runQuietly,
    refreshAndBackup,
    cancelCurrentBackup,
    prepare,
    confirm,
    dismissProposal,
  } = usePopoverActions({ snapshot, actions, active, setupComplete });

  const openLogs = () => runQuietly("logs", actions.openLogs);

  return (
    <main className="app-shell" aria-label="Backup Mic">
      <PopoverHeader
        mountedCount={mountedCount}
        setupComplete={setupComplete}
        active={active}
        busy={busy}
        pending={pending}
        onRefresh={() => void refreshAndBackup()}
        onShowSettings={() => runQuietly("settings", actions.showSettings)}
      />

      <Separator />

      {setupComplete ? (
        <>
          <section className="status-section" aria-live="polite">
            {active ? <ActiveStatus snapshot={snapshot} /> : null}
            {!active && !failed ? (
              <SettledStatus
                snapshot={snapshot}
                mountedCount={mountedCount}
                onPrepare={prepare}
                pending={pending}
              />
            ) : null}
            {failed ? (
              <FailureStatus snapshot={snapshot} busy={busy} onOpenLogs={openLogs} />
            ) : null}
            {!waitingForDevice ? <StageSequence snapshot={snapshot} /> : null}
            {actionError ? (
              <FailureAlert
                failure={actionError}
                logAvailable={snapshot.current_log_available}
                busy={busy}
                onOpenLogs={openLogs}
              />
            ) : null}
          </section>

          <section className="channel-section" aria-label="녹음기 상태">
            {snapshot.sources.map((source) => (
              <ChannelRow key={source.source_id} snapshot={source} active={active} />
            ))}
          </section>

          <ActivityFeed activity={snapshot.recent_activity} />
        </>
      ) : (
        <section className="setup-section" aria-label="초기 설정">
          <SetupFlow snapshot={snapshot} actions={actions} busy={busy} />
        </section>
      )}

      <PopoverFooter
        setupComplete={setupComplete}
        active={active}
        busy={busy}
        pending={pending}
        cancelRequested={cancelRequested}
        logAvailable={snapshot.current_log_available}
        onRefresh={() => void refreshAndBackup()}
        onCancel={() => void cancelCurrentBackup()}
        onOpenDestination={() => runQuietly("open", actions.openDestination)}
        onOpenLogs={openLogs}
        onQuit={() => runQuietly("quit", actions.quitApp)}
      />

      <TrashDialog
        proposal={proposal}
        busy={pending === "confirm-trash"}
        error={proposal ? actionError?.detail ?? null : null}
        onOpenChange={(open) => {
          if (!open) dismissProposal();
        }}
        onConfirm={confirm}
      />
    </main>
  );
}
