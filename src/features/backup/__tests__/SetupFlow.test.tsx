import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import completeFixture from "../../../../contracts/fixtures/backup-complete.json";
import { SetupFlow, type SetupActions } from "../SetupFlow";
import { appSnapshotSchema } from "../contracts";

const complete = appSnapshotSchema.parse(completeFixture);
const needsDestination = { ...complete, setup_state: "needs_destination" as const };
const needsReview = {
  ...complete,
  revision: complete.revision + 1,
  setup_state: "needs_settings_review" as const,
};

function actions(overrides: Partial<SetupActions> = {}): SetupActions {
  return {
    chooseDestination: vi.fn().mockResolvedValue(needsReview),
    completeInitialSetup: vi.fn().mockResolvedValue(complete),
    setAutomaticBackup: vi.fn().mockResolvedValue(needsReview),
    setM4aConversion: vi.fn().mockResolvedValue(needsReview),
    setAutomaticTrash: vi.fn().mockResolvedValue(needsReview),
    setAutostart: vi.fn().mockResolvedValue(needsReview),
    ...overrides,
  };
}

describe("SetupFlow", () => {
  it("requires a destination and immediately advances to settings review after persistence", async () => {
    const setupActions = actions();
    render(<SetupFlow snapshot={needsDestination} actions={setupActions} busy={false} />);

    expect(screen.getByText(/DJI Mic Mini 2S 기본 규칙/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "백업 폴더 선택" }));

    await waitFor(() => expect(setupActions.chooseDestination).toHaveBeenCalledOnce());
    expect(await screen.findByRole("heading", { name: "기본 설정을 확인해 주세요" })).toBeInTheDocument();
    expect(screen.getByText("~/Documents/Backup Mic")).toBeInTheDocument();
  });

  it("reviews every backup default and completes setup only after the explicit confirmation", async () => {
    const setupActions = actions();
    render(<SetupFlow snapshot={needsReview} actions={setupActions} busy={false} />);

    expect(screen.getByRole("switch", { name: "자동으로 백업" })).toBeChecked();
    expect(screen.getByRole("switch", { name: "WAV 백업 후 M4A로 변환" })).toBeChecked();
    expect(screen.getByRole("switch", { name: "백업 후 휴지통으로 이동" })).not.toBeChecked();
    expect(screen.getByRole("switch", { name: "로그인할 때 시작" })).toBeChecked();
    expect(screen.getByText(/DJI Mic Mini 2S/)).toBeInTheDocument();
    expect(screen.getByText(/T01 · T02/)).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "설정 확인 완료" }));
    await waitFor(() => expect(setupActions.completeInitialSetup).toHaveBeenCalledOnce());
    expect(await screen.findByText("백업 준비가 끝났습니다")).toBeInTheDocument();
  });

  it("keeps automatic Trash behind the same acknowledgement used in Settings", async () => {
    const setupActions = actions({
      setAutomaticTrash: vi.fn().mockResolvedValue({
        ...needsReview,
        revision: needsReview.revision + 1,
        retirement_mode: "automatic",
        settings: { ...needsReview.settings, automatic_trash: true },
      }),
    });
    render(<SetupFlow snapshot={needsReview} actions={setupActions} busy={false} />);

    fireEvent.click(screen.getByRole("switch", { name: "백업 후 휴지통으로 이동" }));
    expect(screen.getByRole("alertdialog")).toHaveTextContent("자동 휴지통 이동을 켤까요?");
    expect(setupActions.setAutomaticTrash).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole("button", { name: "확인하고 켜기" }));
    await waitFor(() => expect(setupActions.setAutomaticTrash).toHaveBeenCalledWith(true, true));
  });

  it("serializes repeated completion clicks and leaves failed destination selection blocked", async () => {
    let finish!: (value: typeof complete) => void;
    const completion = new Promise<typeof complete>((resolve) => {
      finish = resolve;
    });
    const setupActions = actions({ completeInitialSetup: vi.fn().mockReturnValue(completion) });
    const { unmount } = render(
      <SetupFlow snapshot={needsReview} actions={setupActions} busy={false} />,
    );
    const completeButton = screen.getByRole("button", { name: "설정 확인 완료" });
    fireEvent.click(completeButton);
    fireEvent.click(completeButton);
    expect(setupActions.completeInitialSetup).toHaveBeenCalledTimes(1);
    finish(complete);
    expect(await screen.findByText("백업 준비가 끝났습니다")).toBeInTheDocument();
    unmount();

    const failedActions = actions({
      chooseDestination: vi.fn().mockRejectedValue({ message_code: "destination_invalid" }),
    });
    render(<SetupFlow snapshot={needsDestination} actions={failedActions} busy={false} />);
    fireEvent.click(screen.getByRole("button", { name: "백업 폴더 선택" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("백업 폴더");
    expect(screen.getByRole("button", { name: "백업 폴더 선택" })).toBeEnabled();
  });
});
