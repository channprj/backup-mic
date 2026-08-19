import { useState } from "react";
import {
  FileTextIcon,
  FolderOpenIcon,
  HardDriveIcon,
  RadioTowerIcon,
  RefreshCwIcon,
  ShieldCheckIcon,
} from "lucide-react";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { Spinner } from "@/components/ui/spinner";
import { backupClient, type BackupActions } from "./client";
import type { AppSnapshot, BackupRule, BackupRuleDraft } from "./contracts";
import { errorCopy, retirementOutcomeLabel } from "./format";
import { RuleEditor } from "./RuleEditor";
import { RuleList, duplicateRuleDraft } from "./RuleList";
import {
  SettingChoiceRow,
  SettingRow,
  freeSpaceReserveChoices,
  rescanIntervalChoices,
} from "./SettingsControls";
import { useBackupSnapshot } from "./useBackupSnapshot";
import { useSettingsPersistence } from "./useSettingsPersistence";

export function SettingsApp() {
  const { snapshot, loading, errorCode, refresh } = useBackupSnapshot();

  if (loading) {
    return (
      <main
        className="settings-shell settings-loading"
        aria-label="Backup Mic 설정"
        aria-busy
      >
        <Skeleton className="settings-title-skeleton" />
        <Skeleton className="settings-group-skeleton" />
        <Skeleton className="settings-group-skeleton" />
      </main>
    );
  }

  if (!snapshot) {
    const copy = errorCopy(errorCode ?? "snapshot_unavailable");
    return (
      <main
        className="settings-shell settings-unavailable"
        aria-label="Backup Mic 설정"
      >
        <Alert variant="destructive">
          <AlertTitle>{copy.title}</AlertTitle>
          <AlertDescription>{copy.detail}</AlertDescription>
        </Alert>
        <Button onClick={() => void refresh()}>
          <RefreshCwIcon data-icon="inline-start" />
          다시 불러오기
        </Button>
      </main>
    );
  }

  return <SettingsView snapshot={snapshot} actions={backupClient} />;
}

export function SettingsView({
  snapshot,
  actions,
}: {
  snapshot: AppSnapshot;
  actions: BackupActions;
}) {
  const [confirmAutomaticTrash, setConfirmAutomaticTrash] = useState(false);
  const [editorRule, setEditorRule] = useState<BackupRule | null>(null);
  const [editorDraft, setEditorDraft] = useState<BackupRuleDraft | undefined>();
  const [editing, setEditing] = useState(false);
  const {
    viewSnapshot,
    pending,
    destinationPending,
    destinationStatus,
    actionError,
    busyRuleId,
    persistSetting,
    chooseDestination,
    saveRule,
    archiveRule,
    restoreDjiRule,
  } = useSettingsPersistence(snapshot, actions);

  function closeRuleEditor() {
    setEditing(false);
    setEditorRule(null);
    setEditorDraft(undefined);
  }

  const refusal = viewSnapshot.sources
    .map(({ retirement_outcome }) => retirementOutcomeLabel(retirement_outcome))
    .find(
      (outcome) => outcome === "이동 중단됨" || outcome === "일부만 이동됨",
    );

  return (
    <main className="settings-shell" aria-label="Backup Mic 설정">
      <header className="settings-header">
        <div>
          <p>Backup Mic</p>
          <h1>설정</h1>
        </div>
        <span>v{__APP_VERSION__}</span>
      </header>

      {actionError ? (
        <Alert variant="destructive">
          <AlertTitle>{actionError.title}</AlertTitle>
          <AlertDescription>
            <p>{actionError.detail}</p>
            <p className="support-code">오류 코드: {actionError.messageCode}</p>
            {viewSnapshot.current_log_available ? (
              <Button
                variant="outline"
                size="sm"
                onClick={() => void actions.openLogs()}
              >
                <FileTextIcon data-icon="inline-start" />
                로그 열기
              </Button>
            ) : null}
          </AlertDescription>
        </Alert>
      ) : null}

      <section
        className="settings-section"
        aria-labelledby="recorder-rules-title"
      >
        <div className="settings-section-title">
          <RadioTowerIcon aria-hidden="true" />
          <div>
            <h2 id="recorder-rules-title">녹음기 규칙</h2>
            <p>녹음기별 파일 선택과 파일명 프리픽스·서픽스를 관리합니다.</p>
          </div>
        </div>
        <div className="settings-group rule-settings-group">
          {editing ? (
            <RuleEditor
              rule={editorRule}
              initialDraft={editorDraft}
              artifactFormat={viewSnapshot.artifact_format}
              connectedVolumeNames={[
                ...new Set(
                  viewSnapshot.sources
                    .filter(({ mounted }) => mounted)
                    .map(({ volume_name }) => volume_name),
                ),
              ]}
              busy={busyRuleId !== null}
              onCancel={closeRuleEditor}
              onSave={async (draft) => {
                if (await saveRule(draft)) closeRuleEditor();
              }}
              onTest={actions.testBackupRule}
            />
          ) : (
            <RuleList
              rules={viewSnapshot.backup_rules}
              busyRuleId={busyRuleId}
              onAdd={() => {
                setEditorRule(null);
                setEditorDraft(undefined);
                setEditing(true);
              }}
              onEdit={(rule) => {
                setEditorRule(rule);
                setEditorDraft(undefined);
                setEditing(true);
              }}
              onDuplicate={(rule) => {
                setEditorRule(null);
                setEditorDraft(duplicateRuleDraft(rule));
                setEditing(true);
              }}
              onArchive={async (ruleId) => {
                await archiveRule(ruleId);
              }}
              onRestoreDji={async () => {
                if (await restoreDjiRule()) closeRuleEditor();
              }}
            />
          )}
        </div>
      </section>

      <section
        className="settings-section"
        aria-labelledby="backup-settings-title"
      >
        <div className="settings-section-title">
          <HardDriveIcon aria-hidden="true" />
          <div>
            <h2 id="backup-settings-title">백업</h2>
            <p>백업 위치와 기본 출력 형식을 관리합니다.</p>
          </div>
        </div>
        <div className="settings-group">
          <div className="settings-row settings-destination-row">
            <div>
              <strong>백업 폴더</strong>
              <span
                className="settings-destination-path"
                title={viewSnapshot.destination_display ?? undefined}
              >
                {viewSnapshot.destination_display ?? "설정되지 않음"}
              </span>
            </div>
            <Button
              variant="outline"
              disabled={destinationPending}
              onClick={() => void chooseDestination()}
            >
              {destinationPending ? <Spinner data-icon="inline-start" /> : null}
              변경…
            </Button>
          </div>
          {destinationStatus ? (
            <p
              className="settings-status-note is-success"
              role="status"
              aria-live="polite"
            >
              {destinationStatus}
            </p>
          ) : null}
          <SettingChoiceRow
            id="free-space-reserve"
            label="백업 폴더에 남길 여유 공간"
            description="변환 중에는 WAV와 M4A가 함께 존재하므로, 이만큼 남지 않으면 백업을 시작하지 않습니다."
            value={viewSnapshot.settings.free_space_reserve_gib}
            choices={freeSpaceReserveChoices.map((gibibytes) => ({
              value: gibibytes,
              label: `${gibibytes} GiB`,
            }))}
            pending={pending.has("free_space_reserve_gib")}
            onChange={(gibibytes) =>
              void persistSetting("free_space_reserve_gib", gibibytes, () =>
                actions.setFreeSpaceReserve(gibibytes),
              )
            }
          />
          <SettingRow
            id="automatic-backup"
            label="자동으로 백업"
            description="연결된 녹음기에 새 녹음이 생기면 자동으로 확인합니다."
            checked={viewSnapshot.settings.automatic_backup}
            pending={pending.has("automatic_backup")}
            onChange={(enabled) =>
              void persistSetting("automatic_backup", enabled, () =>
                actions.setAutomaticBackup(enabled),
              )
            }
          />
          <SettingRow
            id="m4a-conversion"
            label="WAV 백업 후 M4A로 변환"
            description="백업 폴더의 WAV를 AAC-LC 128kbps M4A로 변환합니다."
            checked={viewSnapshot.settings.m4a_conversion}
            pending={pending.has("m4a_conversion")}
            onChange={(enabled) =>
              void persistSetting("m4a_conversion", enabled, () =>
                actions.setM4aConversion(enabled),
              )
            }
          />
          <SettingChoiceRow
            id="rescan-interval"
            label="연결된 녹음기 확인 주기"
            description="녹음기가 계속 연결되어 있을 때 새 녹음을 확인하는 간격입니다."
            value={viewSnapshot.settings.rescan_interval_seconds}
            choices={rescanIntervalChoices}
            pending={pending.has("rescan_interval_seconds")}
            onChange={(seconds) =>
              void persistSetting("rescan_interval_seconds", seconds, () =>
                actions.setRescanInterval(seconds),
              )
            }
          />
          {viewSnapshot.setting_applies_next_run ? (
            <p className="settings-status-note is-info">
              다음 백업부터 적용됩니다
            </p>
          ) : null}
        </div>
      </section>

      <section
        className="settings-section"
        aria-labelledby="source-safety-title"
      >
        <div className="settings-section-title">
          <ShieldCheckIcon aria-hidden="true" />
          <div>
            <h2 id="source-safety-title">원본 안전</h2>
            <p>각 녹음기는 백업과 재검증을 독립적으로 마친 뒤 처리합니다.</p>
          </div>
        </div>
        <div className="settings-group">
          <SettingRow
            id="automatic-trash"
            label="백업 후 휴지통으로 이동"
            description="원본은 macOS 휴지통으로만 이동하며 비워지기 전에는 복구할 수 있습니다."
            checked={viewSnapshot.settings.automatic_trash}
            pending={pending.has("automatic_trash")}
            onChange={(enabled) => {
              if (enabled) setConfirmAutomaticTrash(true);
              else {
                void persistSetting("automatic_trash", false, () =>
                  actions.setAutomaticTrash(false, false),
                );
              }
            }}
          />
          {refusal ? (
            <p className="settings-status-note">최근 원본 처리: {refusal}</p>
          ) : null}
        </div>
      </section>

      <section
        className="settings-section"
        aria-labelledby="general-settings-title"
      >
        <div className="settings-section-title">
          <FolderOpenIcon aria-hidden="true" />
          <div>
            <h2 id="general-settings-title">일반</h2>
            <p>로그인 실행과 백업 기록 바로가기를 관리합니다.</p>
          </div>
        </div>
        <div className="settings-group">
          <SettingRow
            id="autostart"
            label="로그인할 때 시작"
            description="메뉴 막대에서 새 녹음을 계속 확인합니다."
            checked={viewSnapshot.settings.autostart}
            pending={pending.has("autostart")}
            onChange={(enabled) =>
              void persistSetting("autostart", enabled, () =>
                actions.setAutostart(enabled),
              )
            }
          />
          <div className="settings-shortcuts">
            <Button
              variant="outline"
              onClick={() => void actions.openDestination()}
            >
              <FolderOpenIcon data-icon="inline-start" />
              백업 폴더 열기
            </Button>
            <Button
              variant="outline"
              aria-label="로그 열기"
              disabled={!viewSnapshot.current_log_available}
              onClick={() => void actions.openLogs()}
            >
              <FileTextIcon data-icon="inline-start" />
              로그 열기
            </Button>
          </div>
        </div>
      </section>

      <AlertDialog
        open={confirmAutomaticTrash}
        onOpenChange={setConfirmAutomaticTrash}
      >
        <AlertDialogContent className="settings-confirmation-dialog">
          <AlertDialogHeader>
            <AlertDialogTitle>자동 휴지통 이동을 켤까요?</AlertDialogTitle>
            <AlertDialogDescription>
              각 녹음기의 모든 녹음과 M4A 백업을 다시 검증한 뒤 세션 폴더 전체를
              macOS 휴지통으로 이동합니다. 원본을 즉시 영구적으로 지우지
              않습니다.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>취소</AlertDialogCancel>
            <AlertDialogAction
              onClick={() => {
                void persistSetting("automatic_trash", true, () =>
                  actions.setAutomaticTrash(true, true),
                );
              }}
            >
              확인하고 켜기
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </main>
  );
}
