import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import completeFixture from "../../../../contracts/fixtures/backup-complete.json";
import { SetupFlow } from "../SetupFlow";
import { appSnapshotSchema } from "../contracts";

const complete = appSnapshotSchema.parse(completeFixture);

describe("SetupFlow", () => {
  it("requires only a destination and explains the built-in DJI rule", () => {
    const onChooseDestination = vi.fn().mockResolvedValue(undefined);
    render(
      <SetupFlow
        snapshot={{ ...complete, setup_state: "needs_destination" }}
        busy={false}
        onChooseDestination={onChooseDestination}
      />,
    );

    expect(screen.getByText(/DJI Mic Mini 2S 기본 규칙/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "백업 폴더 선택" }));
    expect(onChooseDestination).toHaveBeenCalledOnce();
    expect(screen.queryByText(/TX01|TX02/)).not.toBeInTheDocument();
  });

  it("describes where additional recorder rules are configured", () => {
    render(
      <SetupFlow
        snapshot={complete}
        busy={false}
        onChooseDestination={vi.fn()}
      />,
    );
    expect(screen.getByText(/다른 녹음기는 Settings에서 추가/)).toBeInTheDocument();
  });
});
