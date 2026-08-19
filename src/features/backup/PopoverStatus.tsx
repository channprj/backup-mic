import {
  CheckCircle2Icon,
  CircleAlertIcon,
  FileTextIcon,
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
import { Spinner } from "@/components/ui/spinner";
import type { AppSnapshot, SourceSnapshot } from "./contracts";
import {
  activeTitle,
  errorCopy,
  formatBytes,
  formatCompactTime,
  isWaitingForDevice,
  retirementOutcomeLabel,
  stageLabel,
} from "./format";
import type { ActionError } from "./usePopoverActions";

export function ActiveStatus({ snapshot }: { snapshot: AppSnapshot }) {
  const progress = snapshot.overall_progress;
  const waitingForDevice = isWaitingForDevice(snapshot);
  const findingFiles = ["detecting", "scanning", "checking_capacity"].includes(snapshot.phase);
  return (
    <Card className="status-card is-active">
      <CardHeader>
        <div className="status-title-row">
          <div>
            <CardTitle>{activeTitle(snapshot)}</CardTitle>
          </div>
          <strong className="hero-percent">
            {waitingForDevice ? "대기 중" : findingFiles ? "확인 중" : `${progress.percent}%`}
          </strong>
        </div>
      </CardHeader>
      <CardContent>
        <Progress
          value={findingFiles ? undefined : progress.percent}
          aria-label={
            waitingForDevice
              ? "녹음기 연결 대기 상태"
              : findingFiles
                ? "백업 파일 검색 상태"
                : "전체 백업 진행률"
          }
          aria-valuenow={findingFiles ? undefined : progress.percent}
        />
        <div className="progress-meta">
          {!waitingForDevice ? <span>{stageLabel(snapshot)}</span> : null}
          {!findingFiles ? (
            <span>
              {progress.verified_files}/{progress.total_files}개 · {formatBytes(progress.copied_bytes)}
            </span>
          ) : null}
        </div>
      </CardContent>
      <CardFooter>
        <UsbIcon aria-hidden="true" />
        {waitingForDevice
          ? "녹음기를 연결하면 백업을 자동으로 시작합니다."
          : "완료될 때까지 케이스를 연결해 두세요"}
      </CardFooter>
    </Card>
  );
}

const backupStages = [
  ["copy", "전체 WAV 복사"],
  ["source_verification", "WAV 검증"],
  ["conversion", "128kbps M4A 변환"],
  ["artifact_verification", "전체 M4A 검증"],
  ["source_revalidation", "원본 재검증"],
  ["trash", "휴지통 이동"],
] as const;

export function StageSequence({ snapshot }: { snapshot: AppSnapshot }) {
  const visibleStage = snapshot.current_stage ?? snapshot.failure_stage;
  const activeIndex = backupStages.findIndex(([stage]) => stage === visibleStage);
  const backupComplete = ["completed_deletion_pending", "nothing_new"].includes(snapshot.phase);
  return (
    <>
      <ol className="stage-sequence" aria-label="백업 단계">
        {backupStages.map(([stage, label], index) => {
          const complete = backupComplete
            ? stage !== "trash" || snapshot.retirement_mode === "automatic"
            : activeIndex > index;
          const failed = snapshot.failure_stage === stage;
          return (
            <li
              key={stage}
              className={
                failed
                  ? "is-failed"
                  : activeIndex === index
                    ? "is-current"
                    : complete
                      ? "is-complete"
                      : undefined
              }
              aria-current={activeIndex === index ? "step" : undefined}
            >
              <span aria-hidden="true" />
              {label}
            </li>
          );
        })}
      </ol>
      {!snapshot.settings.m4a_conversion ? (
        <p className="source-retention-note">외장 디스크 원본을 유지합니다</p>
      ) : null}
    </>
  );
}

export function SettledStatus({
  snapshot,
  mountedCount,
  onPrepare,
  pending,
}: {
  snapshot: AppSnapshot;
  mountedCount: number;
  onPrepare: (sourceId: string) => Promise<void>;
  pending: string | null;
}) {
  const complete = snapshot.phase === "completed_deletion_pending";
  const nothingNew = snapshot.phase === "nothing_new";
  const mountedIdle = snapshot.phase === "idle" && mountedCount > 0;
  const title = complete
    ? "백업 검증 완료"
    : nothingNew
      ? "새 녹음 없음"
      : mountedIdle
        ? "녹음기 연결됨"
        : "연결 대기 중";
  const description = complete
    ? "모든 파일의 크기와 SHA-256이 일치합니다"
    : nothingNew
      ? "기존 백업도 다시 검증했습니다"
      : mountedIdle
        ? "다시 확인 및 백업을 눌러 새 녹음을 확인하세요"
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
          {snapshot.sources
            .filter(({ deletion_ready }) => deletion_ready)
            .map(({ source_id, volume_name }) => (
              <Button
                key={source_id}
                variant="outline"
                size="sm"
                aria-label={`${volume_name} 휴지통으로 이동`}
                disabled={pending !== null}
                onClick={() => void onPrepare(source_id)}
              >
                {pending === `prepare-${source_id}` ? (
                  <Spinner data-icon="inline-start" />
                ) : (
                  <Trash2Icon data-icon="inline-start" />
                )}
                {volume_name} 이동
              </Button>
            ))}
        </CardFooter>
      ) : null}
    </Card>
  );
}

export function FailureStatus({
  snapshot,
  busy,
  onOpenLogs,
}: {
  snapshot: AppSnapshot;
  busy: boolean;
  onOpenLogs: () => void;
}) {
  const messageCode = snapshot.error?.message_code ?? snapshot.message_code;
  return (
    <FailureAlert
      failure={{ messageCode, ...errorCopy(messageCode) }}
      logAvailable={snapshot.current_log_available}
      busy={busy}
      onOpenLogs={onOpenLogs}
    />
  );
}

export function FailureAlert({
  failure,
  logAvailable,
  busy,
  onOpenLogs,
}: {
  failure: ActionError;
  logAvailable: boolean;
  busy: boolean;
  onOpenLogs: () => void;
}) {
  return (
    <Alert variant="destructive">
      <CircleAlertIcon aria-hidden="true" />
      <AlertTitle>{failure.title}</AlertTitle>
      <AlertDescription>
        <p>{failure.detail}</p>
        <p className="support-code">오류 코드: {failure.messageCode}</p>
        {logAvailable ? (
          <Button
            variant="outline"
            size="sm"
            aria-label="오류 로그 열기"
            disabled={busy}
            onClick={onOpenLogs}
          >
            <FileTextIcon data-icon="inline-start" />
            로그 열기
          </Button>
        ) : null}
      </AlertDescription>
    </Alert>
  );
}

export function ChannelRow({
  snapshot,
  active,
}: {
  snapshot: AppSnapshot["sources"][number];
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
  const sourceError = snapshot.error
    ? errorCopy(snapshot.error.message_code)
    : null;
  return (
    <div className="channel-row">
      <div className="channel-label">
        <Badge variant={snapshot.mounted ? "secondary" : "outline"}>{snapshot.volume_name}</Badge>
        <span>{snapshot.rule_name} · {state}</span>
      </div>
      <Progress
        value={snapshot.progress.percent}
        aria-label={`${snapshot.volume_name} 백업 진행률`}
        aria-valuenow={snapshot.progress.percent}
      />
      <span className="channel-count">
        {snapshot.progress.verified_files}/{snapshot.progress.total_files}
      </span>
      {sourceError ? (
        <span className="channel-error" role="alert">
          {sourceError.title} · 오류 코드: {snapshot.error?.message_code}
        </span>
      ) : null}
    </div>
  );
}
