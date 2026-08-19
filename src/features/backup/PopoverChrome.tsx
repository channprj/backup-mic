import {
  FileTextIcon,
  FolderOpenIcon,
  PowerIcon,
  RefreshCwIcon,
  SettingsIcon,
  XIcon,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Spinner } from "@/components/ui/spinner";
import type { AppSnapshot } from "./contracts";
import { activityLabel, formatBytes, formatTime } from "./format";

export function PopoverHeader({
  mountedCount,
  setupComplete,
  active,
  busy,
  pending,
  onRefresh,
  onShowSettings,
}: {
  mountedCount: number;
  setupComplete: boolean;
  active: boolean;
  busy: boolean;
  pending: string | null;
  onRefresh: () => void;
  onShowSettings: () => void;
}) {
  return (
    <header className="app-header">
      <div className="product-identity">
        <div className="product-mark" aria-hidden="true">
          <span />
          <span />
        </div>
        <div>
          <h1>Backup Mic</h1>
          <p>로컬 · SHA-256 검증</p>
        </div>
      </div>
      {setupComplete ? (
        <div className="header-actions">
          <div
            className="connection-summary"
            role="status"
            aria-label={`${mountedCount}개 녹음기 연결됨`}
          >
            <span className={mountedCount > 0 ? "status-dot is-connected" : "status-dot"} />
            {mountedCount}개
          </div>
          <Button
            className="header-refresh"
            variant="ghost"
            size="icon"
            aria-label="녹음기 다시 확인 및 백업"
            title="녹음기 다시 확인 및 백업 (⌘R)"
            disabled={active || busy}
            onClick={onRefresh}
          >
            {pending === "backup" ? <Spinner /> : <RefreshCwIcon />}
          </Button>
          <Button variant="ghost" size="sm" disabled={busy} onClick={onShowSettings}>
            <SettingsIcon data-icon="inline-start" />
            Settings…
          </Button>
        </div>
      ) : null}
    </header>
  );
}

export function ActivityFeed({ activity }: { activity: AppSnapshot["recent_activity"] }) {
  return (
    <section className="activity-section" aria-labelledby="activity-title">
      <div className="section-heading">
        <h2 id="activity-title">최근 활동</h2>
        <span>{activity.length}</span>
      </div>
      <ScrollArea className="activity-scroll">
        {activity.length ? (
          <ol className="activity-list">
            {activity.map((entry, index) => (
              <li key={`${entry.occurred_at}-${entry.code}-${index}`}>
                <span className={`activity-node is-${entry.severity}`} aria-hidden="true" />
                <div>
                  <p>{activityLabel(entry.code, entry.source_label)}</p>
                  <span>
                    {formatTime(entry.occurred_at)}
                    {entry.count_value !== null ? ` · ${entry.count_value}개` : ""}
                    {entry.byte_value !== null ? ` · ${formatBytes(entry.byte_value)}` : ""}
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
  );
}

export function PopoverFooter({
  setupComplete,
  active,
  busy,
  pending,
  cancelRequested,
  logAvailable,
  onRefresh,
  onCancel,
  onOpenDestination,
  onOpenLogs,
  onQuit,
}: {
  setupComplete: boolean;
  active: boolean;
  busy: boolean;
  pending: string | null;
  cancelRequested: boolean;
  logAvailable: boolean;
  onRefresh: () => void;
  onCancel: () => void;
  onOpenDestination: () => void;
  onOpenLogs: () => void;
  onQuit: () => void;
}) {
  return (
    <footer className="app-footer">
      <div className="utility-actions">
        {setupComplete ? (
          <>
            {active ? (
              <Button
                variant="outline"
                size="sm"
                aria-label="백업 취소"
                disabled={cancelRequested || busy}
                onClick={onCancel}
              >
                {cancelRequested ? (
                  <Spinner data-icon="inline-start" />
                ) : (
                  <XIcon data-icon="inline-start" />
                )}
                {cancelRequested ? "취소 중…" : "백업 취소"}
              </Button>
            ) : (
              <Button
                variant="ghost"
                size="sm"
                aria-label="지금 백업"
                disabled={busy}
                onClick={onRefresh}
              >
                {pending === "backup" ? (
                  <Spinner data-icon="inline-start" />
                ) : (
                  <RefreshCwIcon data-icon="inline-start" />
                )}
                지금 백업
              </Button>
            )}
            <Button variant="ghost" size="sm" disabled={busy} onClick={onOpenDestination}>
              <FolderOpenIcon data-icon="inline-start" />
              백업 폴더
            </Button>
            <Button
              variant="ghost"
              size="sm"
              aria-label="로그 열기"
              disabled={!logAvailable || busy}
              onClick={onOpenLogs}
            >
              <FileTextIcon data-icon="inline-start" />
              로그
            </Button>
          </>
        ) : null}
        <Button
          variant="ghost"
          size="icon"
          aria-label="앱 종료"
          disabled={busy}
          onClick={onQuit}
        >
          <PowerIcon data-icon="inline-start" />
        </Button>
      </div>
    </footer>
  );
}
