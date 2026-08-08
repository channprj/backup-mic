import { ShieldCheckIcon, Trash2Icon } from "lucide-react";
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
import { Alert, AlertDescription } from "@/components/ui/alert";
import { Spinner } from "@/components/ui/spinner";
import type { DeletionProposalSummary } from "./contracts";
import { formatBytes } from "./format";

interface DeletionDialogProps {
  proposal: DeletionProposalSummary | null;
  busy: boolean;
  error: string | null;
  onOpenChange: (open: boolean) => void;
  onConfirm: (proposalId: string) => Promise<void>;
}

export function DeletionDialog({
  proposal,
  busy,
  error,
  onOpenChange,
  onConfirm,
}: DeletionDialogProps) {
  return (
    <AlertDialog open={proposal !== null} onOpenChange={onOpenChange}>
      <AlertDialogContent className="deletion-dialog">
        <AlertDialogHeader>
          <div className="dialog-kicker">
            <ShieldCheckIcon aria-hidden="true" />
            두 번째 검증 준비됨
          </div>
          <AlertDialogTitle>{proposal?.transmitter} 원본을 삭제할까요?</AlertDialogTitle>
          <AlertDialogDescription>
            삭제 직전에 송신기와 백업 파일 전체를 다시 검사합니다. 하나라도 달라지면 아무것도
            삭제하지 않습니다.
          </AlertDialogDescription>
        </AlertDialogHeader>
        {proposal ? (
          <dl className="proposal-summary">
            <div>
              <dt>검증 대상</dt>
              <dd>{proposal.file_count}개</dd>
            </div>
            <div>
              <dt>원본 용량</dt>
              <dd>{formatBytes(proposal.byte_count)}</dd>
            </div>
            <div>
              <dt>백업 위치</dt>
              <dd>{proposal.destination_summary}</dd>
            </div>
          </dl>
        ) : null}
        {error ? (
          <Alert variant="destructive">
            <AlertDescription>{error}</AlertDescription>
          </Alert>
        ) : null}
        <AlertDialogFooter>
          <AlertDialogCancel disabled={busy}>원본 유지</AlertDialogCancel>
          <AlertDialogAction
            className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
            disabled={busy || !proposal}
            onClick={(event) => {
              event.preventDefault();
              if (proposal) void onConfirm(proposal.proposal_id);
            }}
          >
            {busy ? <Spinner data-icon="inline-start" /> : <Trash2Icon data-icon="inline-start" />}
            검증 후 원본 삭제
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}
