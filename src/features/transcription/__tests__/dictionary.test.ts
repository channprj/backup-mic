import { describe, expect, test } from "vitest";
import type { DictionaryEntry } from "../contracts";
import {
  DictionaryTextError,
  formatDictionaryText,
  nextDictionaryEntryId,
  parseDictionaryAliases,
  parseDictionaryText,
} from "../dictionary";

const existing: DictionaryEntry[] = [
  {
    id: "dictionary-1",
    term: "WakeNote",
    aliases: ["wake note"],
    enabled: false,
  },
  { id: "dictionary-4", term: "DJI", aliases: [], enabled: true },
];

describe("dictionary text", () => {
  test("round-trips the entries it formats", () => {
    const text = formatDictionaryText(existing);
    expect(text).toBe("# 정식 표기 = 별칭, 별칭\nWakeNote = wake note\nDJI\n");
    expect(parseDictionaryText(text, existing)).toEqual(existing);
  });

  test("keeps ids and on/off state for known terms and numbers new ones", () => {
    const parsed = parseDictionaryText(
      "# comment\r\n\r\nwakenote = Wake Note, WAKE NOTE, wakenote\nQwen3-ASR = qwen 3 asr\n",
      existing,
    );
    expect(parsed).toEqual([
      {
        id: "dictionary-1",
        term: "wakenote",
        aliases: ["Wake Note"],
        enabled: false,
      },
      {
        id: "dictionary-2",
        term: "Qwen3-ASR",
        aliases: ["qwen 3 asr"],
        enabled: true,
      },
    ]);
  });

  test("reports the line of a malformed entry", () => {
    for (const [text, line, message] of [
      ["WakeNote\n\nwakenote\n", 3, "정식 표기가 중복됩니다"],
      ["WakeNote = wake note,, wake\n", 1, "빈 별칭"],
      ["# ok\n = alias\n", 2, "정식 표기가 비어 있습니다"],
      [`${"가".repeat(129)}\n`, 1, "128자"],
    ] as const) {
      expect(() => parseDictionaryText(text, [])).toThrowError(
        DictionaryTextError,
      );
      try {
        parseDictionaryText(text, []);
      } catch (error) {
        expect((error as DictionaryTextError).line).toBe(line);
        expect((error as Error).message).toContain(message);
      }
    }
  });

  test("splits aliases without blanks or repeats of the term", () => {
    expect(
      parseDictionaryAliases(" wake note, , WakeNote, Wake Note ", "WakeNote"),
    ).toEqual(["wake note"]);
  });

  test("picks the first unused id", () => {
    expect(nextDictionaryEntryId(existing)).toBe("dictionary-2");
  });
});
