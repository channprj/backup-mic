import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { RuleEditor } from "../RuleEditor";
import type { BackupRule } from "../contracts";

const lockedDjiRule: BackupRule = {
  id: "6d784c99-8b0e-4a32-a0a2-d7730f68cf28",
  name: "DJI Mic Mini 2S",
  archive_directory_name: "DJI Mic Mini 2S",
  enabled: true,
  volume_name_glob: "*",
  required_path_globs: [],
  backup_file_globs: ["*.WAV", "TX_MIC*/*.WAV"],
  session_directory_globs: ["TX_MIC*"],
  filename_prefix: "",
  filename_suffix: "",
  date_folder_layout: "year_month_day",
  archive_directory_locked: true,
  is_dji_preset: true,
  archived: false,
};

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
      artifactFormat="m4a"
      connectedVolumeNames={["ZOOM_TEST"]}
      busy={false}
      onCancel={vi.fn()}
      onSave={onSave}
      onTest={onTest}
    />,
  );
  return { onSave, onTest };
}

function previewDate() {
  const date = new Date();
  const year = String(date.getFullYear());
  const month = String(date.getMonth() + 1).padStart(2, "0");
  const day = String(date.getDate()).padStart(2, "0");
  return { year, month, day, compact: `${year.slice(-2)}${month}${day}` };
}

function fillValidRule() {
  if (!screen.queryByLabelText("필수 경로 glob 1")) {
    fireEvent.click(screen.getByRole("button", { name: "필수 경로 추가" }));
  }
  if (!screen.queryByLabelText("세션 폴더 glob 1")) {
    fireEvent.click(screen.getByRole("button", { name: "세션 폴더 추가" }));
  }
  fireEvent.change(screen.getByLabelText("규칙 이름"), {
    target: { value: "Zoom H1n" },
  });
  fireEvent.change(screen.getByLabelText("볼륨 이름 glob"), {
    target: { value: "ZOOM_*" },
  });
  fireEvent.change(screen.getByLabelText("필수 경로 glob 1"), {
    target: { value: "RECORD/**" },
  });
  fireEvent.change(screen.getByLabelText("백업 파일 glob 1"), {
    target: { value: "RECORD/**/*.WAV" },
  });
  fireEvent.change(screen.getByLabelText("세션 폴더 glob 1"), {
    target: { value: "RECORD/*" },
  });
  fireEvent.change(screen.getByLabelText("파일명 프리픽스"), {
    target: { value: "zoom-" },
  });
  fireEvent.change(screen.getByLabelText("파일명 서픽스"), {
    target: { value: "-field" },
  });
}

describe("RuleEditor", () => {
  it("allows an evidence-backed rule to choose a future archive directory", async () => {
    const onSave = vi.fn().mockResolvedValue(undefined);
    render(
      <RuleEditor
        rule={lockedDjiRule}
        artifactFormat="m4a"
        connectedVolumeNames={[]}
        busy={false}
        onCancel={vi.fn()}
        onSave={onSave}
        onTest={vi.fn()}
      />,
    );

    const archive = screen.getByLabelText("보관 폴더 이름");
    expect(archive).toBeEnabled();
    expect(
      screen.getByText(
        "보관 폴더나 날짜 구조를 바꾸면 검증된 기존 파일도 다음 백업 전에 새 구조로 안전하게 정리됩니다.",
      ),
    ).toBeInTheDocument();
    fireEvent.change(archive, {
      target: { value: "내 DJI 마이크" },
    });
    fireEvent.click(screen.getByRole("button", { name: "규칙 저장" }));

    await waitFor(() => expect(onSave).toHaveBeenCalledOnce());
    expect(onSave.mock.calls[0][0]).toMatchObject({
      name: "DJI Mic Mini 2S",
      archive_directory_name: "내 DJI 마이크",
    });
  });

  it("offers exactly the three approved date folder layouts", () => {
    renderEditor();

    const select = screen.getByRole("combobox", { name: "날짜 폴더 구조" });
    expect(select).toHaveValue("year_month_day");
    expect(
      screen.getAllByRole("option").map((option) => option.textContent),
    ).toEqual(["YYYY/MM/DD/", "YYYY/MM/", "YYMMDD/"]);
  });

  it("previews and saves ordered repeatable patterns", async () => {
    const { onSave, onTest } = renderEditor();
    fillValidRule();

    const { year, month, day, compact } = previewDate();
    expect(
      screen.getByText(
        `Zoom H1n/${year}/${month}/${day}/${compact}-zoom-ZOOM0001-field.m4a`,
      ),
    ).toBeInTheDocument();

    fireEvent.change(screen.getByRole("combobox", { name: "날짜 폴더 구조" }), {
      target: { value: "year_month" },
    });
    expect(
      screen.getByText(
        `Zoom H1n/${year}/${month}/${compact}-zoom-ZOOM0001-field.m4a`,
      ),
    ).toBeInTheDocument();
    fireEvent.change(screen.getByRole("combobox", { name: "날짜 폴더 구조" }), {
      target: { value: "year_month_day" },
    });
    expect(
      screen.getByRole("button", { name: "필수 경로 glob 1 제거" }),
    ).not.toHaveAttribute("tabindex", "-1");

    fireEvent.click(
      screen.getByRole("button", { name: "연결된 디스크에서 테스트" }),
    );
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
      date_folder_layout: "year_month_day",
    });
  });

  it("shows a profile-aware DJI source and WAV result", () => {
    render(
      <RuleEditor
        rule={lockedDjiRule}
        artifactFormat="wav"
        connectedVolumeNames={[]}
        busy={false}
        onCancel={vi.fn()}
        onSave={vi.fn()}
        onTest={vi.fn()}
      />,
    );
    const { year, month, day, compact } = previewDate();

    expect(
      screen.getByText(`TX01_MIC001_${year}${month}${day}_120000.WAV`),
    ).toBeInTheDocument();
    expect(
      screen.getByText(
        `DJI Mic Mini 2S/${year}/${month}/${day}/${compact}-T01_MIC001_${year}${month}${day}_120000.wav`,
      ),
    ).toBeInTheDocument();
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
      fireEvent.click(
        screen.getByRole("button", { name: "연결된 디스크에서 테스트" }),
      );
      fireEvent.click(screen.getByRole("button", { name: "규칙 저장" }));
      expect((await screen.findAllByText(message)).length).toBeGreaterThan(0);
      expect(onSave).not.toHaveBeenCalled();
      expect(onTest).not.toHaveBeenCalled();
    });
  }

  it("requires at least one backup pattern", async () => {
    const { onSave, onTest } = renderEditor();
    fillValidRule();
    fireEvent.click(
      screen.getByRole("button", { name: "백업 파일 glob 1 제거" }),
    );
    fireEvent.click(screen.getByRole("button", { name: "규칙 저장" }));
    expect(
      await screen.findByText("백업 파일 glob을 하나 이상 추가해 주세요"),
    ).toBeInTheDocument();
    expect(onSave).not.toHaveBeenCalled();
    expect(onTest).not.toHaveBeenCalled();
  });

  it("reports a connected-disk test failure without submitting a backup", async () => {
    const { onTest } = renderEditor();
    fillValidRule();
    onTest.mockRejectedValueOnce({ message_code: "invalid_rule" });

    fireEvent.click(
      screen.getByRole("button", { name: "연결된 디스크에서 테스트" }),
    );

    expect(
      await screen.findByText("규칙을 테스트하지 못했습니다"),
    ).toBeInTheDocument();
  });
});
