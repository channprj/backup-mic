import { render, screen } from "@testing-library/react";
import { expect, it, vi } from "vitest";
import completeFixture from "../../../../contracts/fixtures/backup-complete.json";
import { appSnapshotSchema } from "../contracts";

const complete = appSnapshotSchema.parse(completeFixture);

vi.mock("../useBackupSnapshot", () => ({
  useBackupSnapshot: () => ({ snapshot: complete, loading: false, errorCode: null, refresh: vi.fn() }),
}));

import { BackupApp } from "../BackupApp";

it("renders the glanceable backup surface from the shared typed snapshot", () => {
  render(<BackupApp />);
  expect(screen.getByRole("main", { name: "DJI Mic Backup" })).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Settings…" })).toBeInTheDocument();
});
