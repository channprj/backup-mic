import { beforeEach, describe, expect, it, vi } from "vitest";
import completeFixture from "../../../../contracts/fixtures/backup-complete.json";

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke }));

import {
  archiveBackupRule,
  confirmTrash,
  getAppSnapshot,
  prepareTrash,
  restoreDjiRule,
  saveBackupRule,
  setAutomaticTrash,
  showSettings,
  testBackupRule,
} from "../client";

const draft = {
  id: null,
  name: "Zoom H1n",
  archive_directory_name: "Zoom H1n",
  enabled: true,
  volume_name_glob: "ZOOM_*",
  required_path_globs: ["RECORD/**"],
  backup_file_globs: ["RECORD/**/*.WAV"],
  session_directory_globs: ["RECORD/*"],
  filename_prefix: "zoom-",
  filename_suffix: "-field",
};

describe("typed Tauri client", () => {
  beforeEach(() => invoke.mockReset());

  it("validates every snapshot returned by Rust", async () => {
    invoke.mockResolvedValue({ ...completeFixture, unexpected_private_field: "/Volumes/source" });
    await expect(getAppSnapshot()).rejects.toThrow();
  });

  it("sends only a validated rule draft to the exact save command", async () => {
    invoke.mockResolvedValue(completeFixture);
    await saveBackupRule(draft);
    expect(invoke).toHaveBeenCalledWith("save_backup_rule", { draft });
  });

  it("rejects authority-shaped rule data before IPC", async () => {
    expect(() => saveBackupRule({ ...draft, volume_uuid: "private" } as never)).toThrow();
    expect(invoke).not.toHaveBeenCalled();
  });

  it("archives and prepares Trash with validated opaque IDs only", async () => {
    invoke.mockResolvedValueOnce(completeFixture);
    const id = "33333333-3333-4333-8333-333333333333";
    await archiveBackupRule(id);
    expect(invoke).toHaveBeenLastCalledWith("archive_backup_rule", { ruleId: id });

    invoke.mockResolvedValueOnce({
      proposal_id: "550e8400-e29b-41d4-a716-446655440000",
      source_id: id,
      source_label: "ZOOM_TEST",
      session_count: 1,
      file_count: 3,
      byte_count: 10,
      destination_summary: "Zoom H1n 백업 폴더",
      expires_at: "2026-08-10T00:05:00Z",
    });
    await prepareTrash(id);
    expect(invoke).toHaveBeenLastCalledWith("prepare_trash", { sourceId: id });
  });

  it("restores the built-in rule without user configuration", async () => {
    invoke.mockResolvedValue(completeFixture);
    await restoreDjiRule();
    expect(invoke).toHaveBeenCalledWith("restore_dji_rule", undefined);
  });

  it("tests a draft and validates the display-only response", async () => {
    invoke.mockResolvedValue({
      matched_volumes: ["ZOOM_TEST"],
      matched_file_count: 3,
      conflict_rule_names: [],
    });
    await expect(testBackupRule(draft)).resolves.toEqual({
      matched_volumes: ["ZOOM_TEST"],
      matched_file_count: 3,
      conflict_rule_names: [],
    });
    expect(invoke).toHaveBeenCalledWith("test_backup_rule", { draft });
  });

  it("confirms Trash movement with one validated opaque proposal ID", async () => {
    invoke.mockResolvedValue(completeFixture);
    const proposalId = "550e8400-e29b-41d4-a716-446655440000";
    await confirmTrash(proposalId);
    expect(invoke).toHaveBeenCalledWith("confirm_trash", { proposalId });
  });

  it("opens the singleton settings window without frontend window authority", async () => {
    invoke.mockResolvedValue(undefined);
    await showSettings();
    expect(invoke).toHaveBeenCalledWith("show_settings");
  });

  it("passes the explicit automatic Trash acknowledgement", async () => {
    invoke.mockResolvedValue(completeFixture);
    await setAutomaticTrash(true, true);
    expect(invoke).toHaveBeenCalledWith("set_automatic_trash", {
      enabled: true,
      acknowledged: true,
    });
  });
});
