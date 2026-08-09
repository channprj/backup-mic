import { beforeEach, describe, expect, it, vi } from "vitest";
import completeFixture from "../../../../contracts/fixtures/backup-complete.json";

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke }));

import { confirmDeletion, getAppSnapshot, pairDevices } from "../client";

describe("typed Tauri client", () => {
  beforeEach(() => invoke.mockReset());

  it("validates every snapshot returned by Rust", async () => {
    invoke.mockResolvedValue({ ...completeFixture, unexpected_private_field: "/Volumes/source" });
    await expect(getAppSnapshot()).rejects.toThrow();
  });

  it("sends pairing authority as opaque candidate IDs and logical labels only", async () => {
    invoke.mockResolvedValue(completeFixture);
    const assignments = [
      {
        candidate_id: "11111111-1111-4111-8111-111111111111",
        transmitter: "TX01" as const,
      },
      {
        candidate_id: "22222222-2222-4222-8222-222222222222",
        transmitter: "TX02" as const,
      },
    ];
    await pairDevices(assignments);
    expect(invoke).toHaveBeenCalledWith("pair_devices", { assignments });
  });

  it("rejects duplicate transmitter assignments before IPC", async () => {
    await expect(
      pairDevices([
        {
          candidate_id: "11111111-1111-4111-8111-111111111111",
          transmitter: "TX01",
        },
        {
          candidate_id: "22222222-2222-4222-8222-222222222222",
          transmitter: "TX01",
        },
      ]),
    ).rejects.toThrow();
    expect(invoke).not.toHaveBeenCalled();
  });

  it("requires both transmitter assignments before IPC", async () => {
    await expect(
      pairDevices([
        {
          candidate_id: "11111111-1111-4111-8111-111111111111",
          transmitter: "TX01",
        },
      ]),
    ).rejects.toThrow();
    expect(invoke).not.toHaveBeenCalled();
  });

  it("confirms deletion with one validated opaque proposal ID", async () => {
    invoke.mockResolvedValue(completeFixture);
    const proposalId = "550e8400-e29b-41d4-a716-446655440000";
    await confirmDeletion(proposalId);
    expect(invoke).toHaveBeenCalledWith("confirm_trash", { proposalId });
  });
});
