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

function renderPopover(snapshot = complete) {
  const actions = {
    backupNow: vi.fn().mockResolvedValue(undefined),
    chooseDestination: vi.fn().mockResolvedValue(snapshot),
    pairDevices: vi.fn().mockResolvedValue(snapshot),
    prepareTrash: vi.fn().mockResolvedValue({
      proposal_id: "550e8400-e29b-41d4-a716-446655440000",
      transmitter: "TX01",
      session_count: 1,
      file_count: 9,
      byte_count: 99_000_000,
      destination_summary: "DJI-Mic-Mini-2S 백업 폴더",
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
  it("shows accessible overall and per-transmitter progress while originals remain protected", () => {
    renderPopover(copying);
    expect(screen.getByText("백업 중")).toBeInTheDocument();
    expect(screen.getByText("원본은 그대로 유지됩니다")).toBeInTheDocument();
    expect(screen.getByRole("progressbar", { name: "전체 백업 진행률" })).toHaveAttribute(
      "aria-valuenow",
      "42",
    );
    expect(screen.getByText("TX01")).toBeInTheDocument();
    expect(screen.getByText("TX02")).toBeInTheDocument();
    expect(screen.getByLabelText("백업 단계")).toHaveTextContent(
      "복사원본 검증M4A 변환M4A 검증휴지통 이동",
    );
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
    expect(screen.getByRole("button", { name: "TX01 휴지통으로 이동" })).toBeEnabled();
  });

  it("never offers source retirement for an error or partial result", () => {
    renderPopover(error);
    expect(screen.getByRole("alert")).toHaveTextContent("저장 공간");
    expect(screen.queryByText(/휴지통 이동 준비/)).not.toBeInTheDocument();
  });

  it("confirms Trash movement with the opaque proposal ID only", async () => {
    const { actions } = renderPopover(complete);
    fireEvent.click(screen.getByRole("button", { name: "TX01 휴지통으로 이동" }));
    expect(await screen.findByText("TX01 원본을 휴지통으로 이동할까요?")).toBeInTheDocument();
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

    fireEvent.click(screen.getByRole("button", { name: "TX01 휴지통으로 이동" }));
    const confirm = await screen.findByRole("button", { name: "검증 후 휴지통으로 이동" });
    fireEvent.click(confirm);
    fireEvent.click(confirm);
    expect(actions.confirmTrash).toHaveBeenCalledTimes(1);

    finish(complete);
    await waitFor(() => {
      expect(
        screen.queryByText("TX01 원본을 휴지통으로 이동할까요?"),
      ).not.toBeInTheDocument();
    });
  });

  it("dismisses a proposal when Rust revokes deletion readiness", async () => {
    const { actions, rerender } = renderPopover(complete);
    fireEvent.click(screen.getByRole("button", { name: "TX01 휴지통으로 이동" }));
    expect(await screen.findByText("TX01 원본을 휴지통으로 이동할까요?")).toBeInTheDocument();

    const revoked = {
      ...complete,
      revision: complete.revision + 1,
      transmitters: complete.transmitters.map((transmitter) =>
        transmitter.transmitter === "TX01"
          ? { ...transmitter, deletion_ready: false, retirement_outcome: "refused" as const }
          : transmitter,
      ),
    };
    rerender(<BackupPopover snapshot={revoked} actions={actions} />);

    await waitFor(() => {
      expect(
        screen.queryByText("TX01 원본을 휴지통으로 이동할까요?"),
      ).not.toBeInTheDocument();
    });
  });
});
