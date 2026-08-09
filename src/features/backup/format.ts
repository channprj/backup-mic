import type { AppSnapshot, Transmitter } from "./contracts";

export function formatBytes(bytes: number) {
  if (bytes < 1024) return `${bytes} B`;
  const units = ["KB", "MB", "GB"];
  let value = bytes / 1024;
  let unit = units[0];
  for (let index = 1; index < units.length && value >= 1024; index += 1) {
    value /= 1024;
    unit = units[index];
  }
  return `${new Intl.NumberFormat("ko-KR", {
    maximumFractionDigits: value >= 10 ? 0 : 1,
  }).format(value)} ${unit}`;
}

export function formatTime(value: string | null) {
  if (!value) return null;
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return null;
  return new Intl.DateTimeFormat("ko-KR", {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  }).format(date);
}

export function formatCompactTime(value: string | null) {
  if (!value) return null;
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return null;
  const hours = String(date.getHours()).padStart(2, "0");
  const minutes = String(date.getMinutes()).padStart(2, "0");
  return `${date.getMonth() + 1}/${date.getDate()} ${hours}:${minutes}`;
}

export function stageLabel(snapshot: AppSnapshot) {
  if (snapshot.phase === "scanning") return "안정적인 녹음 찾는 중";
  if (snapshot.phase === "checking_capacity") return "저장 공간 확인 중";
  if (snapshot.current_stage === "copy") return "전체 WAV 복사 중";
  if (snapshot.current_stage === "source_verification") return "WAV 검증 중";
  if (snapshot.current_stage === "conversion") return "128kbps M4A 변환 중";
  if (snapshot.current_stage === "artifact_verification") return "전체 M4A 검증 중";
  if (snapshot.current_stage === "source_revalidation") return "원본 다시 검증 중";
  if (snapshot.current_stage === "trash") return "휴지통으로 이동 중";
  if (snapshot.phase === "detecting") return "송신기 확인 중";
  return "백업 준비 중";
}

const activityMessages: Record<string, string> = {
  device_detected: "송신기를 감지했습니다",
  device_removed: "송신기 연결이 해제됐습니다",
  devices_paired: "두 송신기를 안전하게 연결했습니다",
  destination_changed: "백업 폴더를 변경했습니다",
  copy_started: "녹음 백업을 시작했습니다",
  backup_complete: "복사와 검증을 마쳤습니다",
  verification_complete: "복사와 검증을 마쳤습니다",
  nothing_new: "새 녹음이 없습니다",
  partial_failure: "일부 녹음을 백업하지 못했습니다",
  deletion_complete: "검증된 원본을 휴지통으로 이동했습니다",
  deletion_refused: "휴지통 이동을 중단했습니다",
  partial_deletion: "일부 원본만 휴지통으로 이동했습니다",
  trash_complete: "검증된 원본을 휴지통으로 이동했습니다",
  trash_refused: "원본을 이동하지 않고 중단했습니다",
  partial_trash: "일부 원본만 휴지통으로 이동했습니다",
  automatic_trash_complete: "검증된 원본을 자동으로 휴지통으로 이동했습니다",
  automatic_trash_partial: "자동 휴지통 이동이 일부만 완료됐습니다",
  legacy_session_moved_to_trash: "빈 이전 세션 폴더를 휴지통으로 이동했습니다",
  capacity_check_failed: "백업 공간이 부족합니다",
};

export function activityLabel(code: string, transmitter: Transmitter | null) {
  const message = activityMessages[code] ?? "상태가 변경됐습니다";
  return transmitter ? `${transmitter} · ${message}` : message;
}

export function retirementOutcomeLabel(outcome: AppSnapshot["transmitters"][number]["retirement_outcome"]) {
  const labels = {
    inactive: null,
    preparing: "이동 준비 중",
    awaiting_confirmation: "확인 대기 중",
    revalidating: "다시 검증 중",
    moving_to_trash: "휴지통 이동 중",
    moved_to_trash: "휴지통 이동 완료",
    refused: "이동 중단됨",
    partially_moved_to_trash: "일부만 이동됨",
  } as const;
  return labels[outcome];
}

export function errorCopy(messageCode: string) {
  const messages: Record<string, { title: string; detail: string }> = {
    insufficient_capacity: {
      title: "저장 공간이 부족합니다",
      detail: "백업 폴더에 10GB 여유 공간을 확보한 뒤 다시 시도해 주세요.",
    },
    device_removed: {
      title: "송신기 연결이 끊겼습니다",
      detail: "원본은 변경되지 않았습니다. 케이스를 다시 연결해 주세요.",
    },
    source_changed: {
      title: "녹음이 변경됐습니다",
      detail: "녹음이 안정된 뒤 다시 백업합니다. 원본은 그대로 남아 있습니다.",
    },
    hash_mismatch: {
      title: "백업 검증이 일치하지 않습니다",
      detail: "검증되지 않은 원본은 그대로 있습니다. 다시 시도해 주세요.",
    },
    copy_failed: {
      title: "파일을 백업 폴더로 복사하지 못했습니다",
      detail: "원본은 그대로 있습니다. 저장 장치 연결과 백업 폴더 권한을 확인해 주세요.",
    },
    sync_failed: {
      title: "복사한 파일을 안전하게 기록하지 못했습니다",
      detail: "원본은 그대로 있습니다. 백업 디스크를 확인한 뒤 다시 시도해 주세요.",
    },
    ledger_operation_failed: {
      title: "백업 기록을 저장하지 못했습니다",
      detail: "원본은 그대로 있습니다. 로그를 확인한 뒤 다시 시도해 주세요.",
    },
    ledger_corrupt: {
      title: "백업 기록을 읽을 수 없습니다",
      detail: "안전을 위해 원본 처리를 중단했습니다. 로그를 확인해 주세요.",
    },
    audit_log_unavailable: {
      title: "오류 로그를 기록하지 못했습니다",
      detail: "원본은 그대로 있습니다. 백업 폴더와 사용자 로그 폴더 권한을 확인해 주세요.",
    },
    audit_event_invalid: {
      title: "안전한 오류 기록을 만들지 못했습니다",
      detail: "민감한 정보는 표시하지 않았습니다. 앱을 다시 실행해 주세요.",
    },
    audio_conversion_failed: {
      title: "M4A로 변환하지 못했습니다",
      detail: "백업된 WAV와 외장 디스크 원본은 그대로 있습니다. 로그를 확인해 주세요.",
    },
    artifact_validation_failed: {
      title: "변환된 M4A를 검증하지 못했습니다",
      detail: "검증되지 않은 결과는 원본 삭제 근거로 사용하지 않습니다. 다시 시도해 주세요.",
    },
    session_contains_unverified_file: {
      title: "세션에 검증되지 않은 파일이 있습니다",
      detail: "원본은 이동하지 않았습니다. 새 파일까지 백업한 뒤 다시 검증합니다.",
    },
    deletion_preflight_refused: {
      title: "원본을 다시 검증하지 못했습니다",
      detail: "세션이 백업 당시와 달라 원본을 이동하지 않았습니다. 백업을 다시 실행해 주세요.",
    },
    proposal_expired: {
      title: "휴지통 이동 확인 시간이 지났습니다",
      detail: "원본은 그대로 있습니다. 이동 대상을 다시 준비해 주세요.",
    },
    proposal_invalidated: {
      title: "휴지통 이동 대상이 변경됐습니다",
      detail: "원본은 그대로 있습니다. 백업과 원본 검증을 다시 실행해 주세요.",
    },
    setting_save_failed: {
      title: "설정을 저장하지 못했습니다",
      detail: "변경 전 값으로 되돌렸습니다. 로그를 확인한 뒤 다시 시도해 주세요.",
    },
    settings_persist_failed: {
      title: "설정을 저장하지 못했습니다",
      detail: "변경 전 값으로 되돌렸습니다. 로그를 확인한 뒤 다시 시도해 주세요.",
    },
    partial_deletion: {
      title: "일부 원본만 휴지통으로 이동했습니다",
      detail: "송신기를 분리하지 말고 로그에서 이동 결과를 확인해 주세요.",
    },
    partial_trash: {
      title: "일부 원본만 휴지통으로 이동했습니다",
      detail: "송신기를 분리하지 말고 로그에서 이동 결과를 확인해 주세요.",
    },
    trash_move_failed: {
      title: "휴지통으로 이동하지 못했습니다",
      detail: "이동되지 않은 원본은 그대로 있습니다. 연결 상태를 확인해 주세요.",
    },
    ledger_reindex_required: {
      title: "백업 기록을 다시 확인해야 합니다",
      detail: "안전을 위해 휴지통 이동이 잠겼습니다. 백업을 다시 실행해 주세요.",
    },
    snapshot_unavailable: {
      title: "현재 상태를 불러오지 못했습니다",
      detail: "메뉴 막대를 다시 열거나 앱을 재시작해 주세요.",
    },
  };
  return (
    messages[messageCode] ?? {
      title: "작업을 완료하지 못했습니다",
      detail: "원본은 그대로 유지됩니다. 잠시 후 다시 시도해 주세요.",
    }
  );
}
