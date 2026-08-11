import { useEffect, useRef, useState } from "react";
import {
  CheckIcon,
  HardDriveIcon,
  RadioTowerIcon,
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
import {
  Card,
  CardContent,
  CardDescription,
  CardFooter,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Spinner } from "@/components/ui/spinner";
import { Switch } from "@/components/ui/switch";
import type { BackupActions } from "./client";
import type { AppSnapshot } from "./contracts";
import { errorCopy } from "./format";

type SetupSettingKey = keyof AppSnapshot["settings"];

export type SetupActions = Pick<
  BackupActions,
  | "chooseDestination"
  | "completeInitialSetup"
  | "setAutomaticBackup"
  | "setM4aConversion"
  | "setAutomaticTrash"
  | "setAutostart"
>;

interface SetupFlowProps {
  snapshot: AppSnapshot;
  actions: SetupActions;
  busy: boolean;
}

interface SetupError {
  title: string;
  detail: string;
}

function setupError(error: unknown, title: string): SetupError {
  const messageCode =
    typeof error === "object" && error !== null && "message_code" in error
      ? String(error.message_code)
      : "operation_failed";
  return { title, detail: errorCopy(messageCode).detail };
}

export function SetupFlow({ snapshot, actions, busy }: SetupFlowProps) {
  const [viewSnapshot, setViewSnapshot] = useState(snapshot);
  const [pending, setPending] = useState<string | null>(null);
  const [actionError, setActionError] = useState<SetupError | null>(null);
  const [confirmAutomaticTrash, setConfirmAutomaticTrash] = useState(false);
  const pendingRef = useRef<string | null>(null);

  useEffect(() => {
    setViewSnapshot((current) => (snapshot.revision > current.revision ? snapshot : current));
  }, [snapshot]);

  async function run(
    key: string,
    operation: () => Promise<AppSnapshot>,
    errorTitle: string,
  ): Promise<AppSnapshot | null> {
    if (pendingRef.current || busy) return null;
    pendingRef.current = key;
    setPending(key);
    setActionError(null);
    try {
      const persisted = await operation();
      setViewSnapshot(persisted);
      return persisted;
    } catch (error) {
      setActionError(setupError(error, errorTitle));
      return null;
    } finally {
      pendingRef.current = null;
      setPending(null);
    }
  }

  async function chooseDestination() {
    await run("destination", actions.chooseDestination, "백업 폴더를 설정하지 못했습니다");
  }

  async function persistSetting(
    key: SetupSettingKey,
    enabled: boolean,
    operation: () => Promise<AppSnapshot>,
  ) {
    if (pendingRef.current || busy) return;
    const previous = viewSnapshot;
    setViewSnapshot((current) => ({
      ...current,
      settings: { ...current.settings, [key]: enabled },
    }));
    const persisted = await run(key, operation, "설정을 저장하지 못했습니다");
    if (!persisted) setViewSnapshot(previous);
  }

  if (viewSnapshot.setup_state === "ready") {
    return (
      <Card>
        <CardHeader>
          <ShieldCheckIcon className="setup-icon" aria-hidden="true" />
          <CardTitle>
            <h2>백업 준비가 끝났습니다</h2>
          </CardTitle>
          <CardDescription>
            DJI Mic Mini 2S 기본 규칙이 등록되어 있습니다. 다른 녹음기는 Settings에서
            추가할 수 있습니다.
          </CardDescription>
        </CardHeader>
      </Card>
    );
  }

  if (viewSnapshot.setup_state === "needs_destination") {
    return (
      <Card>
        <CardHeader>
          <HardDriveIcon className="setup-icon" aria-hidden="true" />
          <CardTitle>
            <h2>백업 폴더를 확인해 주세요</h2>
          </CardTitle>
          <CardDescription>
            녹음은 선택한 폴더 안에서 녹음기별 규칙과 날짜에 따라 정리됩니다.
          </CardDescription>
        </CardHeader>
        <CardContent>
          <p className="setup-note">
            DJI Mic Mini 2S 기본 규칙은 이미 준비되어 있으며 폴더 경로는 앱 밖으로 전송되지
            않습니다.
          </p>
          {actionError ? <SetupErrorAlert error={actionError} /> : null}
        </CardContent>
        <CardFooter>
          <Button disabled={busy || pending !== null} onClick={() => void chooseDestination()}>
            {pending === "destination" ? (
              <Spinner data-icon="inline-start" />
            ) : (
              <HardDriveIcon data-icon="inline-start" />
            )}
            백업 폴더 선택
          </Button>
        </CardFooter>
      </Card>
    );
  }

  return (
    <>
      <Card className="setup-review-card">
        <CardHeader>
          <ShieldCheckIcon className="setup-icon" aria-hidden="true" />
          <CardTitle>
            <h2>기본 설정을 확인해 주세요</h2>
          </CardTitle>
          <CardDescription>
            아래 값은 지금 바꿀 수 있으며, 확인을 마칠 때까지 백업은 시작되지 않습니다.
          </CardDescription>
        </CardHeader>
        <CardContent className="setup-review-content">
          <div className="setup-destination">
            <div>
              <strong>백업 폴더</strong>
              <span title={viewSnapshot.destination_display ?? undefined}>
                {viewSnapshot.destination_display ?? "설정되지 않음"}
              </span>
            </div>
            <Button
              variant="outline"
              size="sm"
              disabled={busy || pending !== null}
              onClick={() => void chooseDestination()}
            >
              {pending === "destination" ? <Spinner data-icon="inline-start" /> : null}
              변경…
            </Button>
          </div>

          <div className="setup-setting-list">
            <SetupSettingRow
              id="setup-automatic-backup"
              label="자동으로 백업"
              checked={viewSnapshot.settings.automatic_backup}
              disabled={busy || pending !== null}
              onChange={(enabled) =>
                void persistSetting("automatic_backup", enabled, () =>
                  actions.setAutomaticBackup(enabled),
                )
              }
            />
            <SetupSettingRow
              id="setup-m4a-conversion"
              label="WAV 백업 후 M4A로 변환"
              checked={viewSnapshot.settings.m4a_conversion}
              disabled={busy || pending !== null}
              onChange={(enabled) =>
                void persistSetting("m4a_conversion", enabled, () =>
                  actions.setM4aConversion(enabled),
                )
              }
            />
            <SetupSettingRow
              id="setup-automatic-trash"
              label="백업 후 휴지통으로 이동"
              checked={viewSnapshot.settings.automatic_trash}
              disabled={busy || pending !== null}
              onChange={(enabled) => {
                if (enabled) setConfirmAutomaticTrash(true);
                else {
                  void persistSetting("automatic_trash", false, () =>
                    actions.setAutomaticTrash(false, false),
                  );
                }
              }}
            />
            <SetupSettingRow
              id="setup-autostart"
              label="로그인할 때 시작"
              checked={viewSnapshot.settings.autostart}
              disabled={busy || pending !== null}
              onChange={(enabled) =>
                void persistSetting("autostart", enabled, () => actions.setAutostart(enabled))
              }
            />
          </div>

          <div className="setup-rule-summary">
            <RadioTowerIcon aria-hidden="true" />
            <div>
              <strong>DJI Mic Mini 2S</strong>
              <span>기본 제공 · DJI-MIC-1 / DJI-MIC-2 · T01 · T02</span>
            </div>
          </div>
          {actionError ? <SetupErrorAlert error={actionError} /> : null}
        </CardContent>
        <CardFooter>
          <Button
            disabled={busy || pending !== null || !viewSnapshot.destination_display}
            onClick={() =>
              void run("complete", actions.completeInitialSetup, "초기 설정을 완료하지 못했습니다")
            }
          >
            {pending === "complete" ? (
              <Spinner data-icon="inline-start" />
            ) : (
              <CheckIcon data-icon="inline-start" />
            )}
            설정 확인 완료
          </Button>
        </CardFooter>
      </Card>

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
    </>
  );
}

function SetupSettingRow({
  id,
  label,
  checked,
  disabled,
  onChange,
}: {
  id: string;
  label: string;
  checked: boolean;
  disabled: boolean;
  onChange: (enabled: boolean) => void;
}) {
  return (
    <label className="setup-setting-row" htmlFor={id}>
      <span>{label}</span>
      <Switch
        id={id}
        aria-label={label}
        checked={checked}
        disabled={disabled}
        onCheckedChange={onChange}
      />
    </label>
  );
}

function SetupErrorAlert({ error }: { error: SetupError }) {
  return (
    <Alert variant="destructive" className="setup-error">
      <AlertTitle>{error.title}</AlertTitle>
      <AlertDescription>{error.detail}</AlertDescription>
    </Alert>
  );
}
