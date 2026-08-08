import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import completeFixture from "../../../../contracts/fixtures/backup-complete.json";
import { SetupFlow } from "../SetupFlow";
import { appSnapshotSchema } from "../contracts";

const candidates = [
  {
    candidate_id: "11111111-1111-4111-8111-111111111111",
    display_name: "DJI 저장 장치 A",
    capacity_bytes: 15_636_365_312,
  },
  {
    candidate_id: "22222222-2222-4222-8222-222222222222",
    display_name: "DJI 저장 장치 B",
    capacity_bytes: 15_636_365_312,
  },
] as const;

function pairingSnapshot(candidateCount = 2) {
  return appSnapshotSchema.parse({
    ...completeFixture,
    phase: "idle",
    setup_state: "needs_pairing",
    pairing_candidates: candidates.slice(0, candidateCount),
  });
}

describe("SetupFlow", () => {
  it("waits until both transmitter volumes are visible", () => {
    render(
      <SetupFlow
        snapshot={pairingSnapshot(1)}
        busy={false}
        onChooseDestination={vi.fn()}
        onPair={vi.fn()}
      />,
    );

    expect(screen.getByRole("status")).toHaveTextContent("두 번째 송신기");
    expect(screen.getByRole("button", { name: "이 송신기로 연결" })).toBeDisabled();
  });

  it("swaps labels and submits two unique opaque assignments", async () => {
    const onPair = vi.fn().mockResolvedValue(undefined);
    render(
      <SetupFlow
        snapshot={pairingSnapshot()}
        busy={false}
        onChooseDestination={vi.fn()}
        onPair={onPair}
      />,
    );

    fireEvent.click(screen.getAllByRole("radio", { name: "TX01로 지정" })[1]);
    fireEvent.click(screen.getByRole("button", { name: "이 송신기로 연결" }));

    await waitFor(() => {
      expect(onPair).toHaveBeenCalledWith([
        { candidate_id: candidates[0].candidate_id, transmitter: "TX02" },
        { candidate_id: candidates[1].candidate_id, transmitter: "TX01" },
      ]);
    });
  });
});
