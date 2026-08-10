import { fireEvent, render, screen } from "@testing-library/react";
import { expect, it, vi } from "vitest";
import { TrashDialog } from "../TrashDialog";
import type { TrashProposalSummary } from "../contracts";

const proposal = {
  proposal_id: "550e8400-e29b-41d4-a716-446655440000",
  source_id: "11111111-1111-4111-8111-111111111111",
  source_label: "MIC_TX",
  session_count: 2,
  file_count: 9,
  byte_count: 99_000_000,
  destination_summary: "DJI-Mic-Mini-2S 백업 폴더",
  expires_at: "2026-08-09T02:25:00Z",
} satisfies TrashProposalSummary;

it("shows recoverable session, file, and byte totals before Trash movement", () => {
  const onConfirm = vi.fn().mockResolvedValue(undefined);
  render(
    <TrashDialog
      proposal={proposal}
      busy={false}
      error={null}
      onOpenChange={vi.fn()}
      onConfirm={onConfirm}
    />,
  );

  expect(screen.getByText("2개 세션")).toBeInTheDocument();
  expect(screen.getByText("MIC_TX 원본을 휴지통으로 이동할까요?")).toBeInTheDocument();
  expect(screen.queryByText(/transmitter/i)).not.toBeInTheDocument();
  expect(screen.getByText("9개 파일")).toBeInTheDocument();
  expect(screen.getByText(/MB/)).toBeInTheDocument();
  expect(screen.getByText(/휴지통이 비워지기 전에는 복구/)).toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "검증 후 휴지통으로 이동" }));
  expect(onConfirm).toHaveBeenCalledWith(proposal.proposal_id);
});
