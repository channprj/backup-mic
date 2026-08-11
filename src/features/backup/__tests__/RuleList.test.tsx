import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import completeFixture from "../../../../contracts/fixtures/backup-complete.json";
import { RuleList, duplicateRuleDraft } from "../RuleList";
import { appSnapshotSchema } from "../contracts";

const rules = appSnapshotSchema.parse(completeFixture).backup_rules;

describe("RuleList", () => {
  it("puts the DJI preset first and exposes edit, duplicate, archive, and restore actions", () => {
    const onEdit = vi.fn();
    const onDuplicate = vi.fn();
    const onArchive = vi.fn().mockResolvedValue(undefined);
    const onRestoreDji = vi.fn().mockResolvedValue(undefined);
    render(
      <RuleList
        rules={rules}
        busyRuleId={null}
        onAdd={vi.fn()}
        onEdit={onEdit}
        onDuplicate={onDuplicate}
        onArchive={onArchive}
        onRestoreDji={onRestoreDji}
      />,
    );

    const items = screen.getAllByRole("listitem");
    expect(items[0]).toHaveTextContent("DJI Mic Mini 2S");
    expect(items[0]).toHaveTextContent("기본 규칙");
    expect(items[0]).toHaveTextContent("DJI Mic Mini 2S/YYYY/MM/");
    fireEvent.click(
      screen.getByRole("button", { name: "DJI Mic Mini 2S 편집" }),
    );
    expect(onEdit).toHaveBeenCalledWith(rules[0]);
    fireEvent.click(screen.getByRole("button", { name: "Zoom H1n 복제" }));
    expect(onDuplicate).toHaveBeenCalledWith(rules[1]);
    expect(
      screen.getByRole("button", { name: "DJI 기본값 복원" }),
    ).toBeEnabled();
  });

  it("creates an enabled unlocked draft without carrying the source rule ID", () => {
    expect(duplicateRuleDraft(rules[0])).toMatchObject({
      id: null,
      name: "DJI Mic Mini 2S 복사본",
      archive_directory_name: "DJI Mic Mini 2S 복사본",
      enabled: true,
      date_folder_layout: "year_month",
    });
    expect(duplicateRuleDraft(rules[0])).not.toHaveProperty(
      "archive_directory_locked",
    );
  });

  it("confirms archival for an evidenced rule before invoking the command", () => {
    const onArchive = vi.fn().mockResolvedValue(undefined);
    render(
      <RuleList
        rules={rules}
        busyRuleId={null}
        onAdd={vi.fn()}
        onEdit={vi.fn()}
        onDuplicate={vi.fn()}
        onArchive={onArchive}
        onRestoreDji={vi.fn()}
      />,
    );
    fireEvent.click(
      screen.getByRole("button", { name: "DJI Mic Mini 2S 보관" }),
    );
    expect(screen.getByRole("alertdialog")).toHaveTextContent(
      "기존 백업 기록은 유지됩니다",
    );
    expect(onArchive).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "규칙 보관" }));
    expect(onArchive).toHaveBeenCalledWith(rules[0].id);
  });

  it("restores an archived DJI preset without removing its row", () => {
    const onRestoreDji = vi.fn().mockResolvedValue(undefined);
    render(
      <RuleList
        rules={[{ ...rules[0], archived: true, enabled: false }]}
        busyRuleId={null}
        onAdd={vi.fn()}
        onEdit={vi.fn()}
        onDuplicate={vi.fn()}
        onArchive={vi.fn()}
        onRestoreDji={onRestoreDji}
      />,
    );

    expect(screen.getByRole("listitem")).toHaveTextContent("보관됨");
    fireEvent.click(screen.getByRole("button", { name: "DJI 기본값 복원" }));
    expect(onRestoreDji).toHaveBeenCalledOnce();
  });
});
