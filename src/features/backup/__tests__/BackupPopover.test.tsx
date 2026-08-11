import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import completeFixture from "../../../../contracts/fixtures/backup-complete.json";
import copyingFixture from "../../../../contracts/fixtures/backup-copying.json";
import errorFixture from "../../../../contracts/fixtures/error-destination-full.json";
import { BackupPopover } from "../BackupPopover";
import { appSnapshotSchema, type TrashProposalSummary } from "../contracts";

const complete = appSnapshotSchema.parse(completeFixture);
const copying = appSnapshotSchema.parse(copyingFixture);
const error = appSnapshotSchema.parse(errorFixture);
const emptyProgress = {
  percent: 0,
  copied_bytes: 0,
  bytes_requiring_copy: 0,
  verified_files: 0,
  total_files: 0,
};
const manualIdle = appSnapshotSchema.parse({
  ...complete,
  phase: "idle",
  message_code: "device_detected",
  overall_progress: emptyProgress,
  current_stage: null,
  failure_stage: null,
  settings: { ...complete.settings, automatic_backup: false },
  sources: complete.sources.map((source) => ({
    ...source,
    phase: "idle",
    progress: emptyProgress,
    retirement_outcome: "inactive",
    deletion_ready: false,
  })),
  error: null,
});

function commandR(target: Window | HTMLElement = window) {
  const event = new KeyboardEvent("keydown", {
    key: "r",
    metaKey: true,
    bubbles: true,
    cancelable: true,
  });
  target.dispatchEvent(event);
  return event;
}

function renderPopover(snapshot = complete) {
  const actions = {
    backupNow: vi.fn().mockResolvedValue(undefined),
    cancelBackup: vi.fn().mockResolvedValue(undefined),
    chooseDestination: vi.fn().mockResolvedValue(snapshot),
    completeInitialSetup: vi.fn().mockResolvedValue(snapshot),
    saveBackupRule: vi.fn().mockResolvedValue(snapshot),
    archiveBackupRule: vi.fn().mockResolvedValue(snapshot),
    restoreDjiRule: vi.fn().mockResolvedValue(snapshot),
    testBackupRule: vi.fn().mockResolvedValue({
      matched_volumes: [],
      matched_file_count: 0,
      conflict_rule_names: [],
    }),
    prepareTrash: vi.fn().mockResolvedValue({
      proposal_id: "550e8400-e29b-41d4-a716-446655440000",
      source_id: "11111111-1111-4111-8111-111111111111",
      source_label: "MIC_TX",
      session_count: 1,
      file_count: 9,
      byte_count: 99_000_000,
      destination_summary: "Backup Mic 백업 폴더",
      expires_at: new Date(Date.now() + 5 * 60 * 1_000).toISOString(),
    } satisfies TrashProposalSummary),
    confirmTrash: vi.fn().mockResolvedValue(snapshot),
    setAutostart: vi.fn().mockResolvedValue(snapshot),
    showSettings: vi.fn().mockResolvedValue(undefined),
    setAutomaticBackup: vi.fn().mockResolvedValue(snapshot),
    setM4aConversion: vi.fn().mockResolvedValue(snapshot),
    setAutomaticTrash: vi.fn().mockResolvedValue(snapshot),
    openDestination: vi.fn().mockResolvedValue(undefined),
    openLogs: vi.fn().mockResolvedValue(undefined),
    quitApp: vi.fn().mockResolvedValue(undefined),
  };
  const view = render(<BackupPopover snapshot={snapshot} actions={actions} />);
  return { actions, ...view };
}

describe("BackupPopover", () => {
  it("blocks every backup surface except Quit until setup review is complete", () => {
    renderPopover({ ...complete, setup_state: "needs_destination" });

    expect(screen.getByRole("heading", { name: "백업 폴더를 확인해 주세요" })).toBeInTheDocument();
    expect(screen.queryByRole("status", { name: /녹음기 연결됨/ })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Settings…" })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "지금 백업" })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "백업 폴더" })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "로그 열기" })).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "앱 종료" })).toBeEnabled();
  });

  it("shows a mounted recorder as ready for a manual rescan", () => {
    renderPopover(manualIdle);

    expect(screen.getByText("녹음기 연결됨")).toBeInTheDocument();
    expect(screen.getByText("다시 확인 및 백업을 눌러 새 녹음을 확인하세요")).toBeInTheDocument();
    expect(screen.queryByText("녹음기를 확인하는 중")).not.toBeInTheDocument();
  });

  it("rescans and backs up from the header refresh button", async () => {
    const { actions } = renderPopover(manualIdle);

    fireEvent.click(screen.getByRole("button", { name: "녹음기 다시 확인 및 백업" }));

    await waitFor(() => expect(actions.backupNow).toHaveBeenCalledOnce());
  });

  it("captures Command-R and starts one manual rescan", async () => {
    const { actions } = renderPopover(manualIdle);

    const event = commandR();

    expect(event.defaultPrevented).toBe(true);
    await waitFor(() => expect(actions.backupNow).toHaveBeenCalledOnce());
  });

  it("serializes header, footer, and shortcut rescan requests", async () => {
    let finish!: () => void;
    const deferred = new Promise<void>((resolve) => {
      finish = resolve;
    });
    const { actions } = renderPopover(manualIdle);
    actions.backupNow.mockReturnValue(deferred);

    fireEvent.click(screen.getByRole("button", { name: "녹음기 다시 확인 및 백업" }));
    commandR();
    fireEvent.click(screen.getByRole("button", { name: "지금 백업" }));

    expect(actions.backupNow).toHaveBeenCalledTimes(1);
    finish();
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "녹음기 다시 확인 및 백업" })).toBeEnabled(),
    );
  });

  it("prevents Command-R reload without starting a duplicate active backup", () => {
    const { actions } = renderPopover(copying);

    const event = commandR();

    expect(event.defaultPrevented).toBe(true);
    expect(actions.backupNow).not.toHaveBeenCalled();
  });

  it("cancels an active scan once and stays pending until the snapshot settles", async () => {
    let finish!: () => void;
    const deferred = new Promise<void>((resolve) => {
      finish = resolve;
    });
    const scanning = {
      ...copying,
      phase: "scanning" as const,
      current_stage: null,
    };
    const { actions, rerender } = renderPopover(scanning);
    actions.cancelBackup.mockReturnValue(deferred);

    const cancel = screen.getByRole("button", { name: "백업 취소" });
    expect(screen.queryByRole("button", { name: "지금 백업" })).not.toBeInTheDocument();
    fireEvent.click(cancel);
    fireEvent.click(cancel);

    expect(actions.cancelBackup).toHaveBeenCalledTimes(1);
    expect(screen.getByRole("button", { name: "백업 취소" })).toHaveTextContent("취소 중…");
    expect(screen.getByRole("button", { name: "백업 취소" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "녹음기 다시 확인 및 백업" })).toBeDisabled();

    finish();
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "백업 취소" })).toHaveTextContent("취소 중…"),
    );
    rerender(<BackupPopover snapshot={manualIdle} actions={actions} />);
    expect(screen.queryByRole("button", { name: "백업 취소" })).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "지금 백업" })).toBeEnabled();
  });

  it("leaves Command-R untouched while editing text", () => {
    const { actions } = renderPopover(manualIdle);
    const input = document.createElement("input");
    document.body.append(input);

    const event = commandR(input);

    expect(event.defaultPrevented).toBe(false);
    expect(actions.backupNow).not.toHaveBeenCalled();
    input.remove();
  });

  it("prevents setup-time Command-R reload without starting backup", () => {
    const { actions } = renderPopover({
      ...complete,
      setup_state: "needs_destination",
    });

    const event = commandR();

    expect(event.defaultPrevented).toBe(true);
    expect(actions.backupNow).not.toHaveBeenCalled();
  });

  it("renders zero, one, and three recorder sources without a fixed denominator", () => {
    const zero = renderPopover({
      ...complete,
      phase: "idle",
      sources: [],
    });
    expect(screen.getByRole("status", { name: "0개 녹음기 연결됨" })).toHaveTextContent("0개");
    expect(screen.queryByText("/2")).not.toBeInTheDocument();
    zero.unmount();

    const third = {
      ...complete.sources[1],
      source_id: "44444444-4444-4444-8444-444444444444",
      rule_name: "Sony PCM-A10",
      volume_name: "PCMRECORDER",
    };
    renderPopover({ ...complete, sources: [...complete.sources, third] });
    expect(screen.getByRole("status", { name: "3개 녹음기 연결됨" })).toHaveTextContent("3개");
    expect(screen.getByText("PCMRECORDER")).toBeInTheDocument();
    expect(screen.getByText(/Sony PCM-A10/)).toBeInTheDocument();
  });

  it("shows accessible overall and per-recorder copy progress", () => {
    renderPopover(copying);
    expect(screen.getByText("WAV 파일을 복사하는 중")).toBeInTheDocument();
    expect(
      screen.getByText("WAV 파일을 백업 폴더로 복사하고 있습니다"),
    ).toBeInTheDocument();
    expect(screen.queryByText("원본은 그대로 유지됩니다")).not.toBeInTheDocument();
    expect(screen.getByRole("progressbar", { name: "전체 백업 진행률" })).toHaveAttribute(
      "aria-valuenow",
      "42",
    );
    expect(screen.getByText("MIC_TX")).toBeInTheDocument();
    expect(screen.getByRole("status", { name: "1개 녹음기 연결됨" })).toBeInTheDocument();
    expect(screen.getByLabelText("백업 단계")).toHaveTextContent(
      "전체 WAV 복사WAV 검증128kbps M4A 변환전체 M4A 검증원본 재검증휴지통 이동",
    );
  });

  it("describes the scan as finding files that need backup", () => {
    renderPopover({
      ...copying,
      phase: "scanning",
      current_stage: null,
    });

    expect(screen.getByText("백업할 파일을 찾는 중")).toBeInTheDocument();
    expect(
      screen.getByText("연결된 녹음기에서 백업할 WAV 파일을 확인하고 있습니다"),
    ).toBeInTheDocument();
    expect(screen.getByText("확인 중")).toBeInTheDocument();
    expect(screen.getByRole("progressbar", { name: "백업 파일 검색 상태" })).not.toHaveAttribute(
      "aria-valuenow",
    );
    expect(screen.queryByText("백업 중")).not.toBeInTheDocument();
    expect(screen.queryByText("원본은 그대로 유지됩니다")).not.toBeInTheDocument();
  });

  it("explains that disabling conversion retains external-disk originals", () => {
    renderPopover({
      ...complete,
      artifact_format: "wav",
      settings: { ...complete.settings, m4a_conversion: false },
    });
    expect(screen.getByText("외장 디스크 원본을 유지합니다")).toBeInTheDocument();
  });

  it("shows the failed revalidation stage, safe support code, and current log action", () => {
    renderPopover({
      ...error,
      message_code: "session_contains_unverified_file",
      failure_stage: "source_revalidation",
      error: {
        code: "session_contains_unverified_file",
        message_code: "session_contains_unverified_file",
        retryable: true,
        source_id: "11111111-1111-4111-8111-111111111111",
        source_label: "MIC_TX",
      },
    });
    const alert = screen.getByRole("alert");
    expect(alert).toHaveTextContent("세션에 검증되지 않은 파일이 있습니다");
    expect(alert).toHaveTextContent("오류 코드: session_contains_unverified_file");
    expect(screen.getByRole("button", { name: "오류 로그 열기" })).toBeEnabled();
  });

  it("opens Settings and the current log through narrow commands", async () => {
    const { actions } = renderPopover();
    fireEvent.click(screen.getByRole("button", { name: "Settings…" }));
    await waitFor(() => expect(actions.showSettings).toHaveBeenCalledOnce());
    await waitFor(() => expect(screen.getByRole("button", { name: "로그 열기" })).toBeEnabled());
    fireEvent.click(screen.getByRole("button", { name: "로그 열기" }));
    expect(actions.openLogs).toHaveBeenCalledOnce();
  });

  it("replaces progress with a verified summary after completion", () => {
    renderPopover(complete);
    expect(screen.getByText("백업 검증 완료")).toBeInTheDocument();
    expect(screen.getByText("11개")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "MIC_TX 휴지통으로 이동" })).toBeEnabled();
  });

  it("never offers source retirement for an error or partial result", () => {
    renderPopover(error);
    expect(screen.getByRole("alert")).toHaveTextContent("저장 공간");
    expect(screen.queryByText(/휴지통 이동 준비/)).not.toBeInTheDocument();
  });

  it("confirms Trash movement with the opaque proposal ID only", async () => {
    const { actions } = renderPopover(complete);
    fireEvent.click(screen.getByRole("button", { name: "MIC_TX 휴지통으로 이동" }));
    expect(await screen.findByText("MIC_TX 원본을 휴지통으로 이동할까요?")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "검증 후 휴지통으로 이동" }));
    await waitFor(() => {
      expect(actions.confirmTrash).toHaveBeenCalledWith(
        "550e8400-e29b-41d4-a716-446655440000",
      );
    });
  });

  it("serializes repeated deletion confirmation clicks", async () => {
    let finish!: (value: typeof complete) => void;
    const deferred = new Promise<typeof complete>((resolve) => {
      finish = resolve;
    });
    const { actions } = renderPopover(complete);
    actions.confirmTrash.mockReturnValue(deferred);

    fireEvent.click(screen.getByRole("button", { name: "MIC_TX 휴지통으로 이동" }));
    const confirm = await screen.findByRole("button", { name: "검증 후 휴지통으로 이동" });
    fireEvent.click(confirm);
    fireEvent.click(confirm);
    expect(actions.confirmTrash).toHaveBeenCalledTimes(1);

    finish(complete);
    await waitFor(() => {
      expect(
        screen.queryByText("MIC_TX 원본을 휴지통으로 이동할까요?"),
      ).not.toBeInTheDocument();
    });
  });

  it("dismisses a proposal when Rust revokes deletion readiness", async () => {
    const { actions, rerender } = renderPopover(complete);
    fireEvent.click(screen.getByRole("button", { name: "MIC_TX 휴지통으로 이동" }));
    expect(await screen.findByText("MIC_TX 원본을 휴지통으로 이동할까요?")).toBeInTheDocument();

    const revoked = {
      ...complete,
      revision: complete.revision + 1,
      sources: complete.sources.map((source) =>
        source.source_id === "11111111-1111-4111-8111-111111111111"
          ? { ...source, deletion_ready: false, retirement_outcome: "refused" as const }
          : source,
      ),
    };
    rerender(<BackupPopover snapshot={revoked} actions={actions} />);

    await waitFor(() => {
      expect(
        screen.queryByText("MIC_TX 원본을 휴지통으로 이동할까요?"),
      ).not.toBeInTheDocument();
    });
  });
});
