import { describe, expect, test } from "vitest";
import transcriptionFixture from "../../../../contracts/fixtures/transcription-snapshot.json";
import {
  transcriptionDictionarySchema,
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
    ]) {
      expect(
        whisperOptionsSchema.safeParse({ ...defaultWhisperOptions, ...refused })
          .success,
        JSON.stringify(refused),
      ).toBe(false);
    }
  });

  test("mirrors the Rust dictionary limits", () => {
    const entry = {
      id: "dictionary-1",
      term: "WakeNote",
      aliases: [],
      enabled: true,
    };
    expect(
      transcriptionDictionarySchema.safeParse({
        enabled: true,
        entries: [entry],
      }).success,
    ).toBe(true);
    for (const refused of [
      { ...entry, term: " WakeNote" },
      { ...entry, term: "가".repeat(129) },
      {
        ...entry,
        aliases: Array.from({ length: 33 }, (_, index) => `a${index}`),
      },
      { ...entry, aliases: ["tab\tinside"] },
      { ...entry, extra: true },
    ]) {
      expect(
        transcriptionDictionarySchema.safeParse({
          enabled: true,
          entries: [refused],
        }).success,
        JSON.stringify(refused),
      ).toBe(false);
    }
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
