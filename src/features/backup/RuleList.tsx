import { useState } from "react";
import { CopyIcon, PencilIcon, PlusIcon, RotateCcwIcon, Trash2Icon } from "lucide-react";
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
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Spinner } from "@/components/ui/spinner";
import type { BackupRule, BackupRuleDraft } from "./contracts";

export interface RuleListProps {
  rules: BackupRule[];
  busyRuleId: string | null;
  onAdd: () => void;
  onEdit: (rule: BackupRule) => void;
  onDuplicate: (rule: BackupRule) => void;
  onArchive: (ruleId: string) => Promise<void>;
  onRestoreDji: () => Promise<void>;
}

export function duplicateRuleDraft(rule: BackupRule): BackupRuleDraft {
  return {
    id: null,
    name: `${rule.name} 복사본`,
    archive_directory_name: `${rule.archive_directory_name} 복사본`,
    enabled: true,
    volume_name_glob: rule.volume_name_glob,
    required_path_globs: [...rule.required_path_globs],
    backup_file_globs: [...rule.backup_file_globs],
    session_directory_globs: [...rule.session_directory_globs],
    filename_prefix: rule.filename_prefix,
    filename_suffix: rule.filename_suffix,
  };
}

export function RuleList({
  rules,
  busyRuleId,
  onAdd,
  onEdit,
  onDuplicate,
  onArchive,
  onRestoreDji,
}: RuleListProps) {
  const [archiveCandidate, setArchiveCandidate] = useState<BackupRule | null>(null);
  const ordered = [...rules].sort((left, right) =>
    Number(right.is_dji_preset) - Number(left.is_dji_preset) || left.name.localeCompare(right.name),
  );
  const dji = ordered.find((rule) => rule.is_dji_preset);

  function requestArchive(rule: BackupRule) {
    if (rule.archive_directory_locked || rule.is_dji_preset) {
      setArchiveCandidate(rule);
    } else {
      void onArchive(rule.id);
    }
  }

  return (
    <div className="rule-list-wrap">
      <div className="rule-list-toolbar">
        <p>{rules.filter((rule) => !rule.archived).length}개 사용 가능</p>
        <div>
          {dji ? (
            <Button
              type="button"
              variant="outline"
              size="sm"
              disabled={busyRuleId !== null}
              onClick={() => void onRestoreDji()}
            >
              {busyRuleId === dji.id ? <Spinner data-icon="inline-start" /> : <RotateCcwIcon data-icon="inline-start" />}
              DJI 기본값 복원
            </Button>
          ) : null}
          <Button type="button" size="sm" disabled={busyRuleId !== null} onClick={onAdd}>
            <PlusIcon data-icon="inline-start" />
            녹음기 규칙 추가
          </Button>
        </div>
      </div>
      <ul className="rule-list">
        {ordered.map((rule) => (
          <li key={rule.id} className={rule.archived ? "is-archived" : undefined}>
            <div className="rule-list-copy">
              <div>
                <strong>{rule.name}</strong>
                {rule.is_dji_preset ? <Badge variant="secondary">기본 규칙</Badge> : null}
                {!rule.enabled ? <Badge variant="outline">사용 안 함</Badge> : null}
                {rule.archived ? <Badge variant="outline">보관됨</Badge> : null}
              </div>
              <span>{rule.volume_name_glob} · {rule.backup_file_globs.join(", ")}</span>
              <code>{rule.archive_directory_name}/YYYY/MM</code>
            </div>
            <div className="rule-list-actions">
              <Button
                type="button"
                variant="ghost"
                size="icon"
                aria-label={`${rule.name} 편집`}
                disabled={busyRuleId !== null || rule.archived}
                onClick={() => onEdit(rule)}
              >
                <PencilIcon aria-hidden="true" />
              </Button>
              <Button
                type="button"
                variant="ghost"
                size="icon"
                aria-label={`${rule.name} 복제`}
                disabled={busyRuleId !== null}
                onClick={() => onDuplicate(rule)}
              >
                <CopyIcon aria-hidden="true" />
              </Button>
              {!rule.archived ? (
                <Button
                  type="button"
                  variant="ghost"
                  size="icon"
                  aria-label={`${rule.name} 보관`}
                  disabled={busyRuleId !== null}
                  onClick={() => requestArchive(rule)}
                >
                  {busyRuleId === rule.id ? <Spinner /> : <Trash2Icon aria-hidden="true" />}
                </Button>
              ) : null}
            </div>
          </li>
        ))}
      </ul>

      <AlertDialog
        open={archiveCandidate !== null}
        onOpenChange={(open) => !open && setArchiveCandidate(null)}
      >
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>{archiveCandidate?.name} 규칙을 보관할까요?</AlertDialogTitle>
            <AlertDialogDescription>
              규칙은 더 이상 새 녹음기에 적용되지 않지만 기존 백업 기록은 유지됩니다.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>취소</AlertDialogCancel>
            <AlertDialogAction
              onClick={() => {
                if (archiveCandidate) void onArchive(archiveCandidate.id);
                setArchiveCandidate(null);
              }}
            >
              규칙 보관
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  );
}
