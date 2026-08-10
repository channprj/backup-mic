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
import type { TrashProposalSummary } from "./contracts";
import { formatBytes } from "./format";

interface TrashDialogProps {
  proposal: TrashProposalSummary | null;
  busy: boolean;
  error: string | null;
  onOpenChange: (open: boolean) => void;
  onConfirm: (proposalId: string) => Promise<void>;
}

export function TrashDialog({
  proposal,
  busy,
  error,
  onOpenChange,
  onConfirm,
}: TrashDialogProps) {
  return (
    <AlertDialog open={proposal !== null} onOpenChange={onOpenChange}>
      <AlertDialogContent className="trash-dialog">
        <AlertDialogHeader>
          <div className="dialog-kicker">
            <ShieldCheckIcon aria-hidden="true" />
            이동 직전 다시 검증합니다
          </div>
          <AlertDialogTitle>
            {proposal?.source_label} 원본을 휴지통으로 이동할까요?
          </AlertDialogTitle>
          <AlertDialogDescription>
            녹음기와 백업 결과 전체가 그대로인지 다시 검사합니다. 하나라도 달라지면 이동하지
            않으며, 휴지통이 비워지기 전에는 복구할 수 있습니다.
          </AlertDialogDescription>
        </AlertDialogHeader>
        {proposal ? (
          <dl className="proposal-summary" aria-label="휴지통 이동 대상">
            <div>
              <dt>세션</dt>
              <dd>{proposal.session_count}개 세션</dd>
            </div>
            <div>
              <dt>녹음</dt>
              <dd>{proposal.file_count}개 파일</dd>
            </div>
            <div>
              <dt>원본 용량</dt>
              <dd>{formatBytes(proposal.byte_count)}</dd>
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
            검증 후 휴지통으로 이동
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}
