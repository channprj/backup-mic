import { RefreshCwIcon } from "lucide-react";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { backupClient } from "./client";
import { BackupPopover } from "./BackupPopover";
import { errorCopy } from "./format";
import { useBackupSnapshot } from "./useBackupSnapshot";

export function BackupApp() {
  const { snapshot, loading, errorCode, refresh } = useBackupSnapshot();

  if (loading) {
    return (
      <main className="app-shell loading-shell" aria-label="Backup Mic" aria-busy="true">
        <h1 className="sr-only">Backup Mic</h1>
        <div className="loading-brand">
          <Skeleton className="brand-skeleton" />
          <div>
            <Skeleton className="title-skeleton" />
            <Skeleton className="copy-skeleton" />
          </div>
        </div>
        <Skeleton className="status-skeleton" />
        <Skeleton className="channel-skeleton" />
        <Skeleton className="channel-skeleton" />
      </main>
    );
  }

  if (!snapshot) {
    const copy = errorCopy(errorCode ?? "snapshot_unavailable");
    return (
      <main className="app-shell unavailable-shell" aria-label="Backup Mic">
        <div className="unavailable-brand">
          <h1>Backup Mic</h1>
          <p>로컬 녹음 백업</p>
        </div>
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

  return <BackupPopover snapshot={snapshot} actions={backupClient} />;
}
