import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import packageManifest from "../../../../package.json";
import completeFixture from "../../../../contracts/fixtures/backup-complete.json";
import { SettingsView } from "../SettingsApp";
import { appSnapshotSchema, type AppSnapshot } from "../contracts";

const complete = appSnapshotSchema.parse(completeFixture);

function renderSettings(snapshot: AppSnapshot = complete) {
  const actions = {
    backupNow: vi.fn().mockResolvedValue(undefined),
    cancelBackup: vi.fn().mockResolvedValue(undefined),
    chooseDestination: vi.fn().mockResolvedValue(complete),
    completeInitialSetup: vi.fn().mockResolvedValue(complete),
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
    setFreeSpaceReserve: vi.fn().mockResolvedValue(complete),
    setRescanInterval: vi.fn().mockResolvedValue(complete),
    openDestination: vi.fn().mockResolvedValue(undefined),
    openLogs: vi.fn().mockResolvedValue(undefined),
    quitApp: vi.fn().mockResolvedValue(undefined),
  };
  return { actions, ...render(<SettingsView snapshot={snapshot} actions={actions} />) };
}

describe("SettingsView", () => {
  it("shows the current backup folder and confirms only a persisted change", async () => {
    const { actions } = renderSettings();
    expect(screen.getByText("~/Documents/Backup Mic")).toBeInTheDocument();
    expect(screen.queryByText("선택한 백업 폴더")).not.toBeInTheDocument();

    actions.chooseDestination.mockResolvedValueOnce({
      ...complete,
      revision: complete.revision + 1,
      destination_display: "/Volumes/Archive/Backup Mic",
    });
    fireEvent.click(screen.getByRole("button", { name: "변경…" }));

    expect(await screen.findByText("/Volumes/Archive/Backup Mic")).toBeInTheDocument();
    expect(screen.getByRole("status")).toHaveTextContent("백업 폴더가 변경되었습니다");
  });

  it("keeps the current path and never reports success after cancellation or failure", async () => {
    const { actions } = renderSettings();
    fireEvent.click(screen.getByRole("button", { name: "변경…" }));
    await waitFor(() => expect(actions.chooseDestination).toHaveBeenCalledOnce());
    expect(screen.getByText("~/Documents/Backup Mic")).toBeInTheDocument();
    expect(screen.queryByRole("status")).not.toBeInTheDocument();

    actions.chooseDestination.mockRejectedValueOnce({ message_code: "destination_invalid" });
    fireEvent.click(screen.getByRole("button", { name: "변경…" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("백업 폴더");
    expect(screen.getByText("~/Documents/Backup Mic")).toBeInTheDocument();
    expect(screen.queryByRole("status")).not.toBeInTheDocument();
  });

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

  it("saves a chosen free space reserve and shows what the ledger returned", async () => {
    const { actions } = renderSettings();
    const reserve = screen.getByLabelText("백업 폴더에 남길 여유 공간");
    expect(reserve).toHaveValue("10");

    actions.setFreeSpaceReserve.mockResolvedValueOnce({
      ...complete,
      revision: complete.revision + 1,
      settings: { ...complete.settings, free_space_reserve_gib: 50 },
    });
    fireEvent.change(reserve, { target: { value: "50" } });

    await waitFor(() => expect(actions.setFreeSpaceReserve).toHaveBeenCalledWith(50));
    await waitFor(() =>
      expect(screen.getByLabelText("백업 폴더에 남길 여유 공간")).toHaveValue("50"),
    );
  });

  it("saves a chosen rescan interval", async () => {
    const { actions } = renderSettings();
    const interval = screen.getByLabelText("연결된 녹음기 확인 주기");
    expect(interval).toHaveValue("15");

    actions.setRescanInterval.mockResolvedValueOnce({
      ...complete,
      revision: complete.revision + 1,
      settings: { ...complete.settings, rescan_interval_seconds: 300 },
    });
    fireEvent.change(interval, { target: { value: "300" } });

    await waitFor(() => expect(actions.setRescanInterval).toHaveBeenCalledWith(300));
    await waitFor(() =>
      expect(screen.getByLabelText("연결된 녹음기 확인 주기")).toHaveValue("300"),
    );
  });

  it("restores the previous choice when Rust refuses the value", async () => {
    const { actions } = renderSettings();
    actions.setRescanInterval.mockRejectedValueOnce({ message_code: "invalid_request" });
    const interval = screen.getByLabelText("연결된 녹음기 확인 주기");

    fireEvent.change(interval, { target: { value: "60" } });

    expect(await screen.findByRole("alert")).toHaveTextContent("설정을 저장하지 못했습니다");
    await waitFor(() =>
      expect(screen.getByLabelText("연결된 녹음기 확인 주기")).toHaveValue("15"),
    );
  });

  it("offers a stored value the preset list does not contain", () => {
    // Rust accepts the whole range, so a value outside the offered presets must still show as
    // itself rather than being silently reported as one of the presets.
    renderSettings({
      ...complete,
      settings: { ...complete.settings, rescan_interval_seconds: 47 },
    });
    expect(screen.getByLabelText("연결된 녹음기 확인 주기")).toHaveValue("47");
  });

  it("shows the version the manifest actually declares", () => {
    // The header used to carry a hand-typed version that drifted two releases behind the app.
    renderSettings();
    expect(screen.getByText(`v${packageManifest.version}`)).toBeInTheDocument();
  });

  it("keeps the archive directory editable after evidence exists", () => {
    renderSettings();
    fireEvent.click(screen.getByRole("button", { name: "DJI Mic Mini 2S 편집" }));
    expect(screen.getByLabelText("보관 폴더 이름")).toBeEnabled();
    expect(
      screen.getByText(
        "보관 폴더나 날짜 구조를 바꾸면 검증된 기존 파일도 다음 백업 전에 새 구조로 안전하게 정리됩니다.",
      ),
    ).toBeInTheDocument();
  });
});
