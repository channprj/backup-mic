import { describe, expect, test } from "vitest";
import transcriptionFixture from "../../../../contracts/fixtures/transcription-snapshot.json";
import {
  transcriptionOptionsSchema,
  transcriptionSnapshotSchema,
  whisperOptionsSchema,
} from "../contracts";
import { defaultWhisperOptions } from "../options";

describe("transcription contracts", () => {
  test("accepts the committed Rust snapshot fixture", () => {
    const parsed = transcriptionSnapshotSchema.parse(transcriptionFixture);
    expect(parsed.selected_model.id).toBe("medium");
    expect(parsed.models).toHaveLength(15);
    expect(parsed.whisper.temperature_increment).toBe(0.2);
  });

  test("refuses fields Rust does not send", () => {
    expect(
      transcriptionSnapshotSchema.safeParse({
        ...transcriptionFixture,
        model_path: "x",
      }).success,
    ).toBe(false);
    expect(
      transcriptionSnapshotSchema.safeParse({
        ...transcriptionFixture,
        whisper: { ...transcriptionFixture.whisper, vad: true },
      }).success,
    ).toBe(false);
  });

  test("mirrors the Rust defaults and ranges for whisper options", () => {
    expect(whisperOptionsSchema.parse(defaultWhisperOptions)).toEqual(
      defaultWhisperOptions,
    );
    for (const refused of [
      { beam_size: 0 },
      { best_of: 9 },
      { temperature: 1.5 },
      { logprob_threshold: 0.5 },
      { threads: 33 },
      { max_segment_length: 1001 },
      { initial_prompt: "가".repeat(601) },
    ]) {
      expect(
        whisperOptionsSchema.safeParse({ ...defaultWhisperOptions, ...refused })
          .success,
        JSON.stringify(refused),
      ).toBe(false);
    }
    expect(
      whisperOptionsSchema.safeParse({
        ...defaultWhisperOptions,
        initial_prompt: "가".repeat(600),
      }).success,
    ).toBe(true);
  });

  test("never sends options without a transcript format", () => {
    expect(
      transcriptionOptionsSchema.safeParse({
        include_existing_recordings: false,
        formats: { txt: false, srt: false, vtt: false, json: false },
        whisper: defaultWhisperOptions,
      }).success,
    ).toBe(false);
  });

  test("keeps the fixture free from absolute paths and identifiers", () => {
    const serialized = JSON.stringify(transcriptionFixture).toLowerCase();
    for (const forbidden of [
      "/volumes/",
      "/users/",
      "sha256",
      "recording_id",
    ]) {
      expect(serialized).not.toContain(forbidden);
    }
  });
});
