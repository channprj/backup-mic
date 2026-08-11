import { useEffect, useState } from "react";
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
import { Switch } from "@/components/ui/switch";
import { backupClient, type BackupActions } from "./client";
import type { AppSnapshot, BackupRule, BackupRuleDraft } from "./contracts";
import { errorCopy, retirementOutcomeLabel } from "./format";
import { RuleEditor } from "./RuleEditor";
import { RuleList, duplicateRuleDraft } from "./RuleList";
import { useBackupSnapshot } from "./useBackupSnapshot";

type SettingKey = keyof AppSnapshot["settings"];

interface ActionError {
  messageCode: string;
  title: string;
  detail: string;
}

function commandError(error: unknown): ActionError {
  if (typeof error === "object" && error !== null && "message_code" in error) {
    const messageCode = String(error.message_code);
    return { messageCode, ...errorCopy(messageCode) };
  }
  const messageCode = "setting_save_failed";
  return { messageCode, ...errorCopy(messageCode) };
}

export function SettingsApp() {
  const { snapshot, loading, errorCode, refresh } = useBackupSnapshot();

  if (loading) {
    return (
      <main className="settings-shell settings-loading" aria-label="Backup Mic 설정" aria-busy>
        <Skeleton className="settings-title-skeleton" />
        <Skeleton className="settings-group-skeleton" />
        <Skeleton className="settings-group-skeleton" />
      </main>
    );
  }

  if (!snapshot) {
    const copy = errorCopy(errorCode ?? "snapshot_unavailable");
    return (
      <main className="settings-shell settings-unavailable" aria-label="Backup Mic 설정">
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
  const [viewSnapshot, setViewSnapshot] = useState(snapshot);
  const [pending, setPending] = useState<Set<SettingKey>>(new Set());
  const [destinationPending, setDestinationPending] = useState(false);
  const [destinationStatus, setDestinationStatus] = useState<string | null>(null);
  const [actionError, setActionError] = useState<ActionError | null>(null);
  const [confirmAutomaticTrash, setConfirmAutomaticTrash] = useState(false);
  const [editorRule, setEditorRule] = useState<BackupRule | null>(null);
  const [editorDraft, setEditorDraft] = useState<BackupRuleDraft | undefined>();
  const [editing, setEditing] = useState(false);
  const [busyRuleId, setBusyRuleId] = useState<string | null>(null);

  useEffect(() => {
    setViewSnapshot((current) => (snapshot.revision > current.revision ? snapshot : current));
  }, [snapshot]);

  function markPending(key: SettingKey, value: boolean) {
    setPending((current) => {
      const next = new Set(current);
      if (value) next.add(key);
      else next.delete(key);
      return next;
    });
  }

  async function persistSetting(
    key: SettingKey,
    enabled: boolean,
    operation: () => Promise<AppSnapshot>,
  ) {
    if (pending.has(key)) return;
    const previous = viewSnapshot.settings[key];
    setActionError(null);
    markPending(key, true);
    setViewSnapshot((current) => ({
      ...current,
      settings: { ...current.settings, [key]: enabled },
    }));
    try {
      const persisted = await operation();
      setViewSnapshot((current) => {
        if (persisted.revision < current.revision) return current;
        return {
          ...persisted,
          artifact_format:
            key === "m4a_conversion" ? persisted.artifact_format : current.artifact_format,
          retirement_mode:
            key === "automatic_trash" ? persisted.retirement_mode : current.retirement_mode,
          settings: {
            ...current.settings,
            [key]: persisted.settings[key],
          },
        };
      });
    } catch (error) {
      setViewSnapshot((current) => ({
        ...current,
        settings: { ...current.settings, [key]: previous },
      }));
      setActionError({ ...commandError(error), title: "설정을 저장하지 못했습니다" });
    } finally {
      markPending(key, false);
    }
  }

  async function chooseDestination() {
    if (destinationPending) return;
    const previousDisplay = viewSnapshot.destination_display;
    setDestinationPending(true);
    setDestinationStatus(null);
    setActionError(null);
    try {
      const persisted = await actions.chooseDestination();
      setViewSnapshot((current) => ({
        ...persisted,
        artifact_format: current.artifact_format,
        retirement_mode: current.retirement_mode,
        settings: current.settings,
      }));
      if (
        persisted.destination_display &&
        persisted.destination_display !== previousDisplay
      ) {
        setDestinationStatus("백업 폴더가 변경되었습니다");
      }
    } catch (error) {
      setActionError({
        ...commandError(error),
        title: "백업 폴더를 변경하지 못했습니다",
      });
    } finally {
      setDestinationPending(false);
    }
  }

  function closeRuleEditor() {
    setEditing(false);
    setEditorRule(null);
    setEditorDraft(undefined);
  }

  async function saveRule(draft: BackupRuleDraft) {
    if (busyRuleId !== null) return;
    setBusyRuleId(draft.id ?? "new-rule");
    setActionError(null);
    try {
      setViewSnapshot(await actions.saveBackupRule(draft));
      closeRuleEditor();
    } catch (error) {
      setActionError({ ...commandError(error), title: "규칙을 저장하지 못했습니다" });
    } finally {
      setBusyRuleId(null);
    }
  }

  async function archiveRule(ruleId: string) {
    if (busyRuleId !== null) return;
    setBusyRuleId(ruleId);
    setActionError(null);
    try {
      setViewSnapshot(await actions.archiveBackupRule(ruleId));
    } catch (error) {
      setActionError({ ...commandError(error), title: "규칙을 보관하지 못했습니다" });
    } finally {
      setBusyRuleId(null);
    }
  }

  async function restoreDjiRule() {
    if (busyRuleId !== null) return;
    const djiRuleId =
      viewSnapshot.backup_rules.find((rule) => rule.is_dji_preset)?.id ?? "dji-preset";
    setBusyRuleId(djiRuleId);
    setActionError(null);
    try {
      setViewSnapshot(await actions.restoreDjiRule());
      closeRuleEditor();
    } catch (error) {
      setActionError({ ...commandError(error), title: "DJI 기본 규칙을 복원하지 못했습니다" });
    } finally {
      setBusyRuleId(null);
    }
  }

  const refusal = viewSnapshot.sources
    .map(({ retirement_outcome }) => retirementOutcomeLabel(retirement_outcome))
    .find((outcome) => outcome === "이동 중단됨" || outcome === "일부만 이동됨");

  return (
    <main className="settings-shell" aria-label="Backup Mic 설정">
      <header className="settings-header">
        <div>
          <p>Backup Mic</p>
          <h1>설정</h1>
        </div>
        <span>v0.260811.2</span>
      </header>

      {actionError ? (
        <Alert variant="destructive">
          <AlertTitle>{actionError.title}</AlertTitle>
          <AlertDescription>
            <p>{actionError.detail}</p>
            <p className="support-code">오류 코드: {actionError.messageCode}</p>
            {viewSnapshot.current_log_available ? (
              <Button variant="outline" size="sm" onClick={() => void actions.openLogs()}>
                <FileTextIcon data-icon="inline-start" />
                로그 열기
              </Button>
            ) : null}
          </AlertDescription>
        </Alert>
      ) : null}

      <section className="settings-section" aria-labelledby="recorder-rules-title">
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
              connectedVolumeNames={[
                ...new Set(
                  viewSnapshot.sources
                    .filter(({ mounted }) => mounted)
                    .map(({ volume_name }) => volume_name),
                ),
              ]}
              busy={busyRuleId !== null}
              onCancel={closeRuleEditor}
              onSave={saveRule}
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
              onArchive={archiveRule}
              onRestoreDji={restoreDjiRule}
            />
          )}
        </div>
      </section>

      <section className="settings-section" aria-labelledby="backup-settings-title">
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
            <p className="settings-status-note is-success" role="status" aria-live="polite">
              {destinationStatus}
            </p>
          ) : null}
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
          {viewSnapshot.setting_applies_next_run ? (
            <p className="settings-status-note is-info">다음 백업부터 적용됩니다</p>
          ) : null}
        </div>
      </section>

      <section className="settings-section" aria-labelledby="source-safety-title">
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
          {refusal ? <p className="settings-status-note">최근 원본 처리: {refusal}</p> : null}
        </div>
      </section>

      <section className="settings-section" aria-labelledby="general-settings-title">
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
              void persistSetting("autostart", enabled, () => actions.setAutostart(enabled))
            }
          />
          <div className="settings-shortcuts">
            <Button variant="outline" onClick={() => void actions.openDestination()}>
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

      <AlertDialog open={confirmAutomaticTrash} onOpenChange={setConfirmAutomaticTrash}>
        <AlertDialogContent className="settings-confirmation-dialog">
          <AlertDialogHeader>
            <AlertDialogTitle>자동 휴지통 이동을 켤까요?</AlertDialogTitle>
            <AlertDialogDescription>
              각 녹음기의 모든 녹음과 M4A 백업을 다시 검증한 뒤 세션 폴더 전체를 macOS
              휴지통으로 이동합니다. 원본을 즉시 영구적으로 지우지 않습니다.
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

function SettingRow({
  id,
  label,
  description,
  checked,
  pending,
  onChange,
}: {
  id: string;
  label: string;
  description: string;
  checked: boolean;
  pending: boolean;
  onChange: (enabled: boolean) => void;
}) {
  return (
    <div className="settings-row">
      <label htmlFor={id}>
        <strong>{label}</strong>
        <span>{description}</span>
      </label>
      <div className="settings-control">
        {pending ? <Spinner aria-label={`${label} 저장 중`} /> : null}
        <Switch
          id={id}
          aria-label={label}
          checked={checked}
          disabled={pending}
          onCheckedChange={onChange}
        />
      </div>
    </div>
  );
}
