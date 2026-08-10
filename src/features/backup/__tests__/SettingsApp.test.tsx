import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import completeFixture from "../../../../contracts/fixtures/backup-complete.json";
import { SettingsView } from "../SettingsApp";
import { appSnapshotSchema } from "../contracts";

const complete = appSnapshotSchema.parse(completeFixture);

function renderSettings() {
  const actions = {
    backupNow: vi.fn().mockResolvedValue(undefined),
    chooseDestination: vi.fn().mockResolvedValue(complete),
    saveBackupRule: vi.fn().mockResolvedValue(complete),
    archiveBackupRule: vi.fn().mockResolvedValue(complete),
    restoreDjiRule: vi.fn().mockResolvedValue(complete),
    testBackupRule: vi.fn().mockResolvedValue({
      matched_volumes: [],
      matched_file_count: 0,
      conflict_rule_names: [],
    }),
    prepareTrash: vi.fn(),
    confirmTrash: vi.fn(),
    setAutostart: vi.fn().mockResolvedValue(complete),
    showSettings: vi.fn().mockResolvedValue(undefined),
    setAutomaticBackup: vi.fn().mockResolvedValue(complete),
    setM4aConversion: vi.fn().mockResolvedValue(complete),
    setAutomaticTrash: vi.fn().mockResolvedValue({
      ...complete,
      revision: complete.revision + 1,
      retirement_mode: "automatic",
      settings: { ...complete.settings, automatic_trash: true },
    }),
    openDestination: vi.fn().mockResolvedValue(undefined),
    openLogs: vi.fn().mockResolvedValue(undefined),
    quitApp: vi.fn().mockResolvedValue(undefined),
  };
  return { actions, ...render(<SettingsView snapshot={complete} actions={actions} />) };
}

describe("SettingsView", () => {
  it("requires a confirmation alert before enabling automatic Trash", async () => {
    const { actions } = renderSettings();
    fireEvent.click(screen.getByRole("switch", { name: "백업 후 휴지통으로 이동" }));
    expect(screen.getByRole("alertdialog")).toHaveTextContent("자동 휴지통 이동을 켤까요?");
    expect(actions.setAutomaticTrash).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole("button", { name: "확인하고 켜기" }));
    await waitFor(() => expect(actions.setAutomaticTrash).toHaveBeenCalledWith(true, true));
    await waitFor(() => expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument());
  });

  it("restores a failed switch and renders an inline error", async () => {
    const { actions } = renderSettings();
    actions.setAutomaticBackup.mockRejectedValue({ message_code: "ledger_operation_failed" });
    const toggle = screen.getByRole("switch", { name: "자동으로 백업" });
    expect(toggle).toBeChecked();
    fireEvent.click(toggle);
    expect(toggle).not.toBeChecked();

    await waitFor(() => expect(toggle).toBeChecked());
    expect(screen.getByRole("alert")).toHaveTextContent("설정을 저장하지 못했습니다");
  });

  it("clears a failed-setting alert after a successful retry and keeps the persisted value", async () => {
    const { actions } = renderSettings();
    actions.setAutomaticBackup
      .mockRejectedValueOnce({ message_code: "settings_persist_failed" })
      .mockResolvedValueOnce({
        ...complete,
        revision: complete.revision + 1,
        settings: { ...complete.settings, automatic_backup: false },
      });
    const toggle = screen.getByRole("switch", { name: "자동으로 백업" });

    fireEvent.click(toggle);
    await waitFor(() => expect(screen.getByRole("alert")).toBeInTheDocument());
    expect(toggle).toBeChecked();

    fireEvent.click(toggle);
    await waitFor(() => expect(screen.queryByRole("alert")).not.toBeInTheDocument());
    expect(toggle).not.toBeChecked();
  });

  it("opens the current log and keeps controls keyboard reachable", () => {
    const { actions } = renderSettings();
    const openLogs = screen.getByRole("button", { name: "로그 열기" });
    expect(openLogs).not.toHaveAttribute("tabindex", "-1");
    expect(screen.getByRole("switch", { name: "WAV 백업 후 M4A로 변환" })).not.toHaveAttribute(
      "tabindex",
      "-1",
    );
    fireEvent.click(openLogs);
    expect(actions.openLogs).toHaveBeenCalledOnce();
  });

  it("preserves another optimistic switch while concurrent commands settle", async () => {
    const { actions } = renderSettings();
    let finishAutomatic!: (snapshot: typeof complete) => void;
    let finishM4a!: (snapshot: typeof complete) => void;
    actions.setAutomaticBackup.mockReturnValue(
      new Promise((resolve) => {
        finishAutomatic = resolve;
      }),
    );
    actions.setM4aConversion.mockReturnValue(
      new Promise((resolve) => {
        finishM4a = resolve;
      }),
    );
    const automatic = screen.getByRole("switch", { name: "자동으로 백업" });
    const m4a = screen.getByRole("switch", { name: "WAV 백업 후 M4A로 변환" });

    fireEvent.click(automatic);
    fireEvent.click(m4a);
    expect(automatic).not.toBeChecked();
    expect(m4a).not.toBeChecked();

    act(() =>
      finishAutomatic({
        ...complete,
        revision: complete.revision + 1,
        settings: { ...complete.settings, automatic_backup: false },
      }),
    );
    await waitFor(() => expect(automatic).not.toBeChecked());
    expect(m4a).not.toBeChecked();

    act(() =>
      finishM4a({
        ...complete,
        revision: complete.revision + 2,
        artifact_format: "wav",
        settings: {
          ...complete.settings,
          automatic_backup: false,
          m4a_conversion: false,
        },
      }),
    );
    await waitFor(() => expect(m4a).not.toBeChecked());
  });

  it("describes the 128kbps backup-folder conversion and next-run behavior", () => {
    const { actions, unmount } = renderSettings();
    const switchControl = screen.getByRole("switch", { name: "WAV 백업 후 M4A로 변환" });
    expect(switchControl.closest(".settings-row")).toHaveTextContent("AAC-LC 128kbps");
    expect(switchControl.closest(".settings-row")).toHaveTextContent("백업 폴더");
    unmount();

    render(<SettingsView snapshot={{ ...complete, setting_applies_next_run: true }} actions={actions} />);
    expect(screen.getByText("다음 백업부터 적용됩니다")).toBeInTheDocument();
  });

  it("opens a duplicate as a new editable rule and replaces the snapshot after save", async () => {
    const { actions } = renderSettings();
    const zoomRule = complete.backup_rules.find(({ name }) => name === "Zoom H1n");
    expect(zoomRule).toBeDefined();

    fireEvent.click(screen.getByRole("button", { name: "Zoom H1n 복제" }));
    expect(screen.getByRole("heading", { name: "녹음기 규칙 추가" })).toBeInTheDocument();
    expect(screen.getByLabelText("규칙 이름")).toHaveValue("Zoom H1n 복사본");
    expect(screen.getByLabelText("보관 폴더 이름")).toBeEnabled();

    fireEvent.click(screen.getByRole("button", { name: "규칙 저장" }));
    await waitFor(() => expect(actions.saveBackupRule).toHaveBeenCalledOnce());
    expect(actions.saveBackupRule.mock.calls[0][0]).toMatchObject({
      id: null,
      enabled: true,
      name: "Zoom H1n 복사본",
      archive_directory_name: "Zoom H1n 복사본",
    });
    await waitFor(() =>
      expect(screen.queryByRole("heading", { name: "녹음기 규칙 추가" })).not.toBeInTheDocument(),
    );
  });

  it("locks the archive directory after evidence exists", () => {
    renderSettings();
    fireEvent.click(screen.getByRole("button", { name: "DJI Mic Mini 2S 편집" }));
    expect(screen.getByLabelText("보관 폴더 이름")).toBeDisabled();
  });
});
