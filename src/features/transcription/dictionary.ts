import {
  maxDictionaryAliases,
  maxDictionaryEntries,
  maxDictionaryTextChars,
  type DictionaryEntry,
} from "./contracts";

/**
 * The dictionary's text form, shared with WakeNote's `dictionary.txt`: one entry per line as
 * `Canonical term = alias, alias`, a line without `=` is a term with no aliases, and blank lines
 * and `#` comments are ignored.
 */
export const dictionaryTextHeader = "# 정식 표기 = 별칭, 별칭";

export class DictionaryTextError extends Error {
  constructor(
    readonly line: number,
    message: string,
  ) {
    super(`${line}번째 줄: ${message}`);
  }
}

/** ASCII values compare case-insensitively; anything else must match exactly. */
export function dictionaryComparisonKey(value: string): string {
  return /^[\x00-\x7F]*$/.test(value) ? value.toLowerCase() : value;
}

function checkValue(value: string, line: number) {
  if (/\p{Cc}/u.test(value)) {
    throw new DictionaryTextError(line, "제어 문자는 쓸 수 없습니다");
  }
  if ([...value].length > maxDictionaryTextChars) {
    throw new DictionaryTextError(
      line,
      `${maxDictionaryTextChars}자를 넘는 값은 쓸 수 없습니다`,
    );
  }
}

/**
 * Parses the text form, keeping the id and on/off state of every term that already existed so
 * an edit does not reset entries the user switched off.
 */
export function parseDictionaryText(
  text: string,
  previous: readonly DictionaryEntry[],
): DictionaryEntry[] {
  const previousByTerm = new Map(
    previous.map((entry) => [
      dictionaryComparisonKey(entry.term.trim()),
      entry,
    ]),
  );
  const seenTerms = new Set<string>();
  const rows: Array<{ term: string; key: string; aliases: string[] }> = [];

  text.split(/\r?\n/).forEach((rawLine, index) => {
    const lineNumber = index + 1;
    const line = rawLine.trim();
    if (!line || line.startsWith("#")) return;
    if (rows.length >= maxDictionaryEntries) {
      throw new DictionaryTextError(
        lineNumber,
        `사전에는 최대 ${maxDictionaryEntries}개까지 넣을 수 있습니다`,
      );
    }
    const separator = line.indexOf("=");
    const term = (separator < 0 ? line : line.slice(0, separator)).trim();
    if (!term) {
      throw new DictionaryTextError(lineNumber, "정식 표기가 비어 있습니다");
    }
    checkValue(term, lineNumber);
    const key = dictionaryComparisonKey(term);
    if (seenTerms.has(key)) {
      throw new DictionaryTextError(
        lineNumber,
        `정식 표기가 중복됩니다: ${term}`,
      );
    }
    seenTerms.add(key);

    const aliases: string[] = [];
    if (separator >= 0) {
      const seenAliases = new Set<string>();
      for (const rawAlias of line.slice(separator + 1).split(",")) {
        const alias = rawAlias.trim();
        if (!alias) {
          throw new DictionaryTextError(lineNumber, "빈 별칭이 있습니다");
        }
        checkValue(alias, lineNumber);
        const aliasKey = dictionaryComparisonKey(alias);
        if (aliasKey !== key && !seenAliases.has(aliasKey)) {
          seenAliases.add(aliasKey);
          aliases.push(alias);
        }
      }
      if (aliases.length > maxDictionaryAliases) {
        throw new DictionaryTextError(
          lineNumber,
          `별칭은 최대 ${maxDictionaryAliases}개까지 넣을 수 있습니다`,
        );
      }
    }
    rows.push({ term, key, aliases });
  });

  const usedIds = new Set<string>();
  for (const { key } of rows) {
    const existing = previousByTerm.get(key);
    if (existing) usedIds.add(existing.id);
  }
  let nextId = 1;
  return rows.map(({ term, key, aliases }) => {
    const existing = previousByTerm.get(key);
    if (existing)
      return { id: existing.id, term, aliases, enabled: existing.enabled };
    while (usedIds.has(`dictionary-${nextId}`)) nextId += 1;
    const id = `dictionary-${nextId}`;
    usedIds.add(id);
    return { id, term, aliases, enabled: true };
  });
}

/** The text form of the current entries, headed by the format reminder. */
export function formatDictionaryText(
  entries: readonly DictionaryEntry[],
): string {
  const lines = entries
    .map((entry) => {
      const term = entry.term.trim();
      if (!term) return null;
      const aliases = entry.aliases
        .map((alias) => alias.trim())
        .filter(Boolean);
      return aliases.length ? `${term} = ${aliases.join(", ")}` : term;
    })
    .filter((line): line is string => line !== null);
  return `${[dictionaryTextHeader, ...lines].join("\n")}\n`;
}

/** Splits the comma-separated alias field, dropping blanks and repeats of the term. */
export function parseDictionaryAliases(value: string, term: string): string[] {
  const termKey = dictionaryComparisonKey(term.trim());
  const seen = new Set<string>();
  return value
    .split(",")
    .map((alias) => alias.trim())
    .filter((alias) => {
      const key = dictionaryComparisonKey(alias);
      if (!alias || key === termKey || seen.has(key)) return false;
      seen.add(key);
      return true;
    });
}

export function nextDictionaryEntryId(
  entries: readonly DictionaryEntry[],
): string {
  const ids = new Set(entries.map((entry) => entry.id));
  let index = 1;
  while (ids.has(`dictionary-${index}`)) index += 1;
  return `dictionary-${index}`;
}
