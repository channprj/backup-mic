import { describe, expect, it } from "vitest";
import completeFixture from "../../../../contracts/fixtures/backup-complete.json";
import { appSnapshotSchema } from "../contracts";
import { activeTitle, errorCopy, isWaitingForDevice, stageLabel } from "../format";

const complete = appSnapshotSchema.parse(completeFixture);
const waiting = appSnapshotSchema.parse({
  ...complete,
  phase: "detecting",
  message_code: "waiting_for_device",
  current_stage: null,
  error: null,
});

describe("waiting-for-recorder copy", () => {
  it("distinguishes a queued manual backup from ordinary device detection", () => {
    expect(isWaitingForDevice(waiting)).toBe(true);
    expect(activeTitle(waiting)).toBe("녹음기 연결을 기다리는 중");
    expect(stageLabel(waiting)).toBe("녹음기를 연결하면 백업을 자동으로 시작합니다.");
    expect(isWaitingForDevice({ ...waiting, message_code: "device_detected" })).toBe(false);
  });

  it("keeps a genuine device removal error concise", () => {
    expect(errorCopy("device_removed")).toEqual({
      title: "녹음기 연결이 끊겼습니다",
      detail: "녹음기를 다시 연결해 주세요.",
    });
  });
});
