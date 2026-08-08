import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import completeFixture from "../../../../contracts/fixtures/backup-complete.json";

const mocks = vi.hoisted(() => ({
  getAppSnapshot: vi.fn(),
  listenForSnapshots: vi.fn(),
  unlisten: vi.fn(),
}));

vi.mock("../client", () => ({
  getAppSnapshot: mocks.getAppSnapshot,
  listenForSnapshots: mocks.listenForSnapshots,
}));

import { appSnapshotSchema, type AppSnapshot } from "../contracts";
import { useBackupSnapshot } from "../useBackupSnapshot";

const complete = appSnapshotSchema.parse(completeFixture);
const revision = (value: number): AppSnapshot => ({ ...complete, revision: value });

function Harness() {
  const { snapshot, loading } = useBackupSnapshot();
  return <div>{loading ? "loading" : `revision:${snapshot?.revision ?? "none"}`}</div>;
}

describe("useBackupSnapshot", () => {
  beforeEach(() => {
    mocks.getAppSnapshot.mockReset();
    mocks.listenForSnapshots.mockReset();
    mocks.unlisten.mockReset();
  });

  it("subscribes before refresh, repairs lost events, and never regresses revisions", async () => {
    let listener!: (snapshot: AppSnapshot) => void;
    mocks.listenForSnapshots.mockImplementation(async (next) => {
      listener = next;
      return mocks.unlisten;
    });
    mocks.getAppSnapshot
      .mockResolvedValueOnce(revision(14))
      .mockResolvedValueOnce(revision(18))
      .mockResolvedValueOnce(revision(22));

    const view = render(<Harness />);
    expect(await screen.findByText("revision:14")).toBeInTheDocument();
    expect(mocks.listenForSnapshots.mock.invocationCallOrder[0]).toBeLessThan(
      mocks.getAppSnapshot.mock.invocationCallOrder[0],
    );

    act(() => listener(revision(21)));
    expect(screen.getByText("revision:21")).toBeInTheDocument();

    fireEvent.focus(window);
    await waitFor(() => expect(mocks.getAppSnapshot).toHaveBeenCalledTimes(2));
    expect(screen.getByText("revision:21")).toBeInTheDocument();

    fireEvent.focus(window);
    expect(await screen.findByText("revision:22")).toBeInTheDocument();

    view.unmount();
    expect(mocks.unlisten).toHaveBeenCalledOnce();
  });
});
