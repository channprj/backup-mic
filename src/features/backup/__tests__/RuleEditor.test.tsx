import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { RuleEditor } from "../RuleEditor";

function renderEditor() {
  const onSave = vi.fn().mockResolvedValue(undefined);
  const onTest = vi.fn().mockResolvedValue({
    matched_volumes: ["ZOOM_TEST"],
    matched_file_count: 3,
    conflict_rule_names: [],
  });
  render(
    <RuleEditor
      rule={null}
      connectedVolumeNames={["ZOOM_TEST"]}
      busy={false}
      onCancel={vi.fn()}
      onSave={onSave}
      onTest={onTest}
    />,
  );
  return { onSave, onTest };
}

function fillValidRule() {
  if (!screen.queryByLabelText("필수 경로 glob 1")) {
    fireEvent.click(screen.getByRole("button", { name: "필수 경로 추가" }));
  }
  if (!screen.queryByLabelText("세션 폴더 glob 1")) {
    fireEvent.click(screen.getByRole("button", { name: "세션 폴더 추가" }));
  }
  fireEvent.change(screen.getByLabelText("규칙 이름"), { target: { value: "Zoom H1n" } });
  fireEvent.change(screen.getByLabelText("볼륨 이름 glob"), { target: { value: "ZOOM_*" } });
  fireEvent.change(screen.getByLabelText("필수 경로 glob 1"), {
    target: { value: "RECORD/**" },
  });
  fireEvent.change(screen.getByLabelText("백업 파일 glob 1"), {
    target: { value: "RECORD/**/*.WAV" },
  });
  fireEvent.change(screen.getByLabelText("세션 폴더 glob 1"), {
    target: { value: "RECORD/*" },
  });
  fireEvent.change(screen.getByLabelText("파일명 프리픽스"), { target: { value: "zoom-" } });
  fireEvent.change(screen.getByLabelText("파일명 서픽스"), { target: { value: "-field" } });
}

describe("RuleEditor", () => {
  it("previews and saves ordered repeatable patterns", async () => {
    const { onSave, onTest } = renderEditor();
    fillValidRule();

    expect(screen.getByText("Zoom H1n/YYYY/MM/YYMMDD-zoom-ZOOM0001-field.m4a")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "필수 경로 glob 1 제거" })).not.toHaveAttribute(
      "tabindex",
      "-1",
    );

    fireEvent.click(screen.getByRole("button", { name: "연결된 디스크에서 테스트" }));
    await waitFor(() => expect(onTest).toHaveBeenCalledOnce());
    expect(await screen.findByText("ZOOM_TEST · 3개 파일")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "규칙 저장" }));
    await waitFor(() => expect(onSave).toHaveBeenCalledOnce());
    expect(onSave.mock.calls[0][0]).toMatchObject({
      id: null,
      name: "Zoom H1n",
      archive_directory_name: "Zoom H1n",
      required_path_globs: ["RECORD/**"],
      backup_file_globs: ["RECORD/**/*.WAV"],
      session_directory_globs: ["RECORD/*"],
      filename_prefix: "zoom-",
      filename_suffix: "-field",
    });
  });

  for (const [label, value, message] of [
    ["볼륨 이름 glob", "[abc", "올바른 glob 패턴을 입력해 주세요"],
    ["파일명 서픽스", "/private", "경로 구분자는 사용할 수 없습니다"],
    ["규칙 이름", "x".repeat(65), "64자 이하로 입력해 주세요"],
  ]) {
    it(`blocks invalid ${label} values`, async () => {
      const { onSave, onTest } = renderEditor();
      fillValidRule();
      fireEvent.change(screen.getByLabelText(label), { target: { value } });
      fireEvent.click(screen.getByRole("button", { name: "연결된 디스크에서 테스트" }));
      fireEvent.click(screen.getByRole("button", { name: "규칙 저장" }));
      expect((await screen.findAllByText(message)).length).toBeGreaterThan(0);
      expect(onSave).not.toHaveBeenCalled();
      expect(onTest).not.toHaveBeenCalled();
    });
  }

  it("requires at least one backup pattern", async () => {
    const { onSave, onTest } = renderEditor();
    fillValidRule();
    fireEvent.click(screen.getByRole("button", { name: "백업 파일 glob 1 제거" }));
    fireEvent.click(screen.getByRole("button", { name: "규칙 저장" }));
    expect(await screen.findByText("백업 파일 glob을 하나 이상 추가해 주세요")).toBeInTheDocument();
    expect(onSave).not.toHaveBeenCalled();
    expect(onTest).not.toHaveBeenCalled();
  });

  it("reports a connected-disk test failure without submitting a backup", async () => {
    const { onTest } = renderEditor();
    fillValidRule();
    onTest.mockRejectedValueOnce({ message_code: "invalid_rule" });

    fireEvent.click(screen.getByRole("button", { name: "연결된 디스크에서 테스트" }));

    expect(await screen.findByText("규칙을 테스트하지 못했습니다")).toBeInTheDocument();
  });
});
