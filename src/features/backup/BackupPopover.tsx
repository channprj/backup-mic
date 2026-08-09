import { useEffect, useRef, useState } from "react";
import {
  CheckCircle2Icon,
  CircleAlertIcon,
  FileTextIcon,
  FolderOpenIcon,
  PowerIcon,
  RefreshCwIcon,
  SettingsIcon,
  ShieldCheckIcon,
  Trash2Icon,
  UsbIcon,
} from "lucide-react";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardFooter,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Progress } from "@/components/ui/progress";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Separator } from "@/components/ui/separator";
import { Spinner } from "@/components/ui/spinner";
import type { BackupActions } from "./client";
import type {
  AppSnapshot,
  PairingAssignment,
  TrashProposalSummary,
  Transmitter,
} from "./contracts";
import { TrashDialog } from "./TrashDialog";
import {
  activityLabel,
  errorCopy,
  formatBytes,
  formatCompactTime,
  formatTime,
  retirementOutcomeLabel,
  stageLabel,
} from "./format";
import { SetupFlow } from "./SetupFlow";

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

function commandMessage(error: unknown) {
  if (typeof error === "object" && error !== null && "message_code" in error) {
    return errorCopy(String(error.message_code)).detail;
  }
  return "작업을 완료하지 못했습니다. 원본은 그대로 유지됩니다.";
}

export function BackupPopover({ snapshot, actions }: BackupPopoverProps) {
  const [pending, setPending] = useState<string | null>(null);
  const [proposal, setProposal] = useState<TrashProposalSummary | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const pendingRef = useRef<string | null>(null);
  const active = activePhases.has(snapshot.phase) || snapshot.current_stage === "trash";
  const failed = snapshot.phase === "error" || snapshot.phase === "partial_failure";
  const mountedCount = snapshot.transmitters.filter(({ mounted }) => mounted).length;

  useEffect(() => {
    if (!proposal || pending === "confirm-trash") return;
    const transmitter = snapshot.transmitters.find(
      ({ transmitter }) => transmitter === proposal.transmitter,
    );
    if (!transmitter?.deletion_ready) {
      setProposal(null);
    }
  }, [pending, proposal, snapshot]);

  useEffect(() => {
    if (!proposal) return;
    const remaining = Date.parse(proposal.expires_at) - Date.now();
    if (!Number.isFinite(remaining) || remaining <= 0) {
      setProposal(null);
      return;
    }
    const timeout = window.setTimeout(() => setProposal(null), remaining);
    return () => window.clearTimeout(timeout);
  }, [proposal]);

  async function run(key: string, operation: () => Promise<unknown>) {
    if (pendingRef.current) return;
    pendingRef.current = key;
    setPending(key);
    setActionError(null);
    try {
      await operation();
    } catch (error) {
      setActionError(commandMessage(error));
      throw error;
    } finally {
      pendingRef.current = null;
      setPending(null);
    }
  }

  async function pair(assignments: PairingAssignment[]) {
    try {
      await run("pair", () => actions.pairDevices(assignments));
    } catch {
      // The inline error already describes the retry path.
    }
  }

  async function prepare(transmitter: Transmitter) {
    try {
      await run(`prepare-${transmitter}`, async () => {
        setProposal(await actions.prepareTrash(transmitter));
      });
    } catch {
      // The inline error already describes the retry path.
    }
  }

  async function confirm(proposalId: string) {
    try {
      await run("confirm-trash", () => actions.confirmTrash(proposalId));
      setProposal(null);
    } catch {
      // Keep the confirmation open; Rust has already refused unsafe Trash movement.
    }
  }

  return (
    <main className="app-shell" aria-label="DJI Mic Backup">
      <header className="app-header">
        <div className="product-identity">
          <div className="product-mark" aria-hidden="true">
            <span />
            <span />
          </div>
          <div>
            <h1>DJI Mic Backup</h1>
            <p>로컬 · SHA-256 검증</p>
          </div>
        </div>
        <div className="header-actions">
          <div
            className="connection-summary"
            role="status"
            aria-label={`${mountedCount}개 송신기 연결됨`}
          >
            <span className={mountedCount > 0 ? "status-dot is-connected" : "status-dot"} />
            {mountedCount}/2
          </div>
          <Button
            variant="ghost"
            size="sm"
            disabled={pending !== null}
            onClick={() => void run("settings", actions.showSettings).catch(() => undefined)}
          >
            <SettingsIcon data-icon="inline-start" />
            Settings…
          </Button>
        </div>
      </header>

      <Separator />

      {snapshot.setup_state !== "ready" ? (
        <section className="setup-section" aria-label="초기 설정">
          <SetupFlow
            snapshot={snapshot}
            busy={pending !== null}
            onChooseDestination={async () => {
              try {
                await run("destination", actions.chooseDestination);
              } catch {
                // The inline error already describes the retry path.
              }
            }}
            onPair={pair}
          />
        </section>
      ) : (
        <>
          <section className="status-section" aria-live="polite">
            {active ? <ActiveStatus snapshot={snapshot} /> : null}
            {!active && !failed ? (
              <SettledStatus snapshot={snapshot} onPrepare={prepare} pending={pending} />
            ) : null}
            {failed ? <FailureStatus snapshot={snapshot} /> : null}
            <StageSequence snapshot={snapshot} />
            {actionError ? (
              <Alert variant="destructive">
                <CircleAlertIcon aria-hidden="true" />
                <AlertTitle>작업을 완료하지 못했습니다</AlertTitle>
                <AlertDescription>{actionError}</AlertDescription>
              </Alert>
            ) : null}
          </section>

          <section className="channel-section" aria-label="송신기 상태">
            {snapshot.transmitters.map((transmitter) => (
              <ChannelRow key={transmitter.transmitter} snapshot={transmitter} active={active} />
            ))}
          </section>

          <section className="activity-section" aria-labelledby="activity-title">
            <div className="section-heading">
              <h2 id="activity-title">최근 활동</h2>
              <span>{snapshot.recent_activity.length}</span>
            </div>
            <ScrollArea className="activity-scroll">
              {snapshot.recent_activity.length ? (
                <ol className="activity-list">
                  {snapshot.recent_activity.map((activity, index) => (
                    <li key={`${activity.occurred_at}-${activity.code}-${index}`}>
                      <span className={`activity-node is-${activity.severity}`} aria-hidden="true" />
                      <div>
                        <p>{activityLabel(activity.code, activity.transmitter)}</p>
                        <span>
                          {formatTime(activity.occurred_at)}
                          {activity.count_value !== null ? ` · ${activity.count_value}개` : ""}
                          {activity.byte_value !== null ? ` · ${formatBytes(activity.byte_value)}` : ""}
                        </span>
                      </div>
                    </li>
                  ))}
                </ol>
              ) : (
                <p className="activity-empty">첫 백업 활동이 여기에 기록됩니다.</p>
              )}
            </ScrollArea>
          </section>
        </>
      )}

      <footer className="app-footer">
        <div className="utility-actions">
          <Button
            variant="ghost"
            size="sm"
            disabled={active || pending !== null}
            onClick={() => void run("backup", actions.backupNow).catch(() => undefined)}
          >
            {pending === "backup" ? (
              <Spinner data-icon="inline-start" />
            ) : (
              <RefreshCwIcon data-icon="inline-start" />
            )}
            지금 백업
          </Button>
          <Button
            variant="ghost"
            size="sm"
            disabled={pending !== null}
            onClick={() => void run("open", actions.openDestination).catch(() => undefined)}
          >
            <FolderOpenIcon data-icon="inline-start" />
            백업 폴더
          </Button>
          <Button
            variant="ghost"
            size="sm"
            aria-label="로그 열기"
            disabled={!snapshot.current_log_available || pending !== null}
            onClick={() => void run("logs", actions.openLogs).catch(() => undefined)}
          >
            <FileTextIcon data-icon="inline-start" />
            로그
          </Button>
          <Button
            variant="ghost"
            size="icon"
            aria-label="앱 종료"
            disabled={pending !== null}
            onClick={() => void run("quit", actions.quitApp).catch(() => undefined)}
          >
            <PowerIcon data-icon="inline-start" />
          </Button>
        </div>
      </footer>

      <TrashDialog
        proposal={proposal}
        busy={pending === "confirm-trash"}
        error={proposal ? actionError : null}
        onOpenChange={(open) => {
          if (!open && pending !== "confirm-trash") {
            setProposal(null);
            setActionError(null);
          }
        }}
        onConfirm={confirm}
      />
    </main>
  );
}

function ActiveStatus({ snapshot }: { snapshot: AppSnapshot }) {
  const progress = snapshot.overall_progress;
  return (
    <Card className="status-card is-active">
      <CardHeader>
        <div className="status-title-row">
          <div>
            <CardTitle>백업 중</CardTitle>
            <CardDescription>원본은 그대로 유지됩니다</CardDescription>
          </div>
          <strong className="hero-percent">{progress.percent}%</strong>
        </div>
      </CardHeader>
      <CardContent>
        <Progress
          value={progress.percent}
          aria-label="전체 백업 진행률"
          aria-valuenow={progress.percent}
        />
        <div className="progress-meta">
          <span>{stageLabel(snapshot)}</span>
          <span>
            {progress.verified_files}/{progress.total_files}개 · {formatBytes(progress.copied_bytes)}
          </span>
        </div>
      </CardContent>
      <CardFooter>
        <UsbIcon aria-hidden="true" />
        완료될 때까지 케이스를 연결해 두세요
      </CardFooter>
    </Card>
  );
}

const backupStages = [
  ["copy", "복사"],
  ["source_verification", "원본 검증"],
  ["conversion", "M4A 변환"],
  ["artifact_verification", "M4A 검증"],
  ["trash", "휴지통 이동"],
] as const;

function StageSequence({ snapshot }: { snapshot: AppSnapshot }) {
  const activeIndex = backupStages.findIndex(([stage]) => stage === snapshot.current_stage);
  const backupComplete = ["completed_deletion_pending", "nothing_new"].includes(snapshot.phase);
  return (
    <ol className="stage-sequence" aria-label="백업 단계">
      {backupStages.map(([stage, label], index) => {
        const complete = backupComplete
          ? stage !== "trash" || snapshot.retirement_mode === "automatic"
          : activeIndex > index;
        return (
          <li
            key={stage}
            className={
              activeIndex === index ? "is-current" : complete ? "is-complete" : undefined
            }
            aria-current={activeIndex === index ? "step" : undefined}
          >
            <span aria-hidden="true" />
            {label}
          </li>
        );
      })}
    </ol>
  );
}

function SettledStatus({
  snapshot,
  onPrepare,
  pending,
}: {
  snapshot: AppSnapshot;
  onPrepare: (transmitter: Transmitter) => Promise<void>;
  pending: string | null;
}) {
  const complete = snapshot.phase === "completed_deletion_pending";
  const nothingNew = snapshot.phase === "nothing_new";
  const title = complete ? "백업 검증 완료" : nothingNew ? "새 녹음 없음" : "연결 대기 중";
  const description = complete
    ? "모든 파일의 크기와 SHA-256이 일치합니다"
    : nothingNew
      ? "기존 백업도 다시 검증했습니다"
      : "케이스를 연결하면 자동으로 백업합니다";
  const deletable = (complete || nothingNew) && !snapshot.error;
  return (
    <Card className="status-card is-settled">
      <CardHeader>
        <div className="status-title-row">
          <div>
            <CardTitle>{title}</CardTitle>
            <CardDescription>{description}</CardDescription>
          </div>
          {complete || nothingNew ? (
            <CheckCircle2Icon className="verified-icon" aria-label="검증 완료" />
          ) : (
            <UsbIcon className="idle-icon" aria-hidden="true" />
          )}
        </div>
      </CardHeader>
      <CardContent>
        <div className="verified-summary">
          <div>
            <span>검증된 녹음</span>
            <strong>{snapshot.overall_progress.verified_files}개</strong>
          </div>
          <div>
            <span>백업 용량</span>
            <strong>{formatBytes(snapshot.overall_progress.bytes_requiring_copy)}</strong>
          </div>
          <div>
            <span>최근 완료</span>
            <strong>{formatCompactTime(snapshot.last_success_at) ?? "—"}</strong>
          </div>
        </div>
      </CardContent>
      {deletable ? (
        <CardFooter className="trash-actions">
          {snapshot.transmitters
            .filter(({ deletion_ready }) => deletion_ready)
            .map(({ transmitter }) => (
              <Button
                key={transmitter}
                variant="outline"
                size="sm"
                aria-label={`${transmitter} 휴지통으로 이동`}
                disabled={pending !== null}
                onClick={() => void onPrepare(transmitter)}
              >
                {pending === `prepare-${transmitter}` ? (
                  <Spinner data-icon="inline-start" />
                ) : (
                  <Trash2Icon data-icon="inline-start" />
                )}
                {transmitter} 이동
              </Button>
            ))}
        </CardFooter>
      ) : null}
    </Card>
  );
}

function FailureStatus({ snapshot }: { snapshot: AppSnapshot }) {
  const copy = errorCopy(snapshot.error?.message_code ?? snapshot.message_code);
  return (
    <Alert variant="destructive">
      <CircleAlertIcon aria-hidden="true" />
      <AlertTitle>{copy.title}</AlertTitle>
      <AlertDescription>{copy.detail}</AlertDescription>
    </Alert>
  );
}

function ChannelRow({
  snapshot,
  active,
}: {
  snapshot: AppSnapshot["transmitters"][number];
  active: boolean;
}) {
  const retirement = retirementOutcomeLabel(snapshot.retirement_outcome);
  const state = !snapshot.mounted
    ? "연결 안 됨"
    : retirement
      ? retirement
    : snapshot.phase === "error" || snapshot.phase === "partial_failure"
      ? "확인 필요"
      : snapshot.progress.percent === 100
        ? "검증 완료"
        : active
          ? `${snapshot.progress.percent}%`
          : "연결됨";
  return (
    <div className="channel-row">
      <div className="channel-label">
        <Badge variant={snapshot.mounted ? "secondary" : "outline"}>{snapshot.transmitter}</Badge>
        <span>{state}</span>
      </div>
      <Progress
        value={snapshot.progress.percent}
        aria-label={`${snapshot.transmitter} 백업 진행률`}
        aria-valuenow={snapshot.progress.percent}
      />
      <span className="channel-count">
        {snapshot.progress.verified_files}/{snapshot.progress.total_files}
      </span>
    </div>
  );
}
