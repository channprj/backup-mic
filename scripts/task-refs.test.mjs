// @vitest-environment node
import { spawnSync } from "node:child_process";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { afterEach, describe, expect, it } from "vitest";
import {
  boardState,
  checkMessage,
  isExempt,
  referencedIssues,
} from "./task-refs.mjs";

const script = fileURLToPath(new URL("task-refs.mjs", import.meta.url));

describe("commit Issue references", () => {
  it("accepts a message that names an Issue in any supported form", () => {
    for (const trailer of ["Refs #12", "Closes #12", "Fixes #12", "refs #12"])
      expect(checkMessage(`feat(x): do a thing\n\n${trailer}\n`)).toBeNull();
    expect(referencedIssues("Refs #7\nCloses #8")).toEqual([7, 8]);
  });

  it("rejects a message with no Issue and explains what to add", () => {
    const problem = checkMessage(
      "feat(x): do a thing\n\nA body with no trailer.",
    );
    expect(problem).toMatch(/does not name a board Issue/);
    expect(problem).toMatch(/Refs #123/);
  });

  it("does not accept a number that is not a reference", () => {
    expect(checkMessage("fix(x): drop the #12 placeholder")).not.toBeNull();
    expect(checkMessage("fix(x): see issue 12")).not.toBeNull();
  });

  it("exempts the commits that tools write", () => {
    expect(isExempt("chore(release): v0.260921.0")).toBe(true);
    expect(isExempt("chore(release): sync v0.260811.4 metadata")).toBe(true);
    expect(isExempt("Merge branch 'main' into topic")).toBe(true);
    expect(isExempt('Revert "feat(x): a thing"')).toBe(true);
    expect(checkMessage("chore(release): v0.260921.0\n")).toBeNull();
    expect(checkMessage("Merge branch 'main'\n")).toBeNull();
    // A release-looking body does not exempt an ordinary subject.
    expect(
      checkMessage("feat(x): a thing\n\nchore(release): v1\n"),
    ).not.toBeNull();
  });

  it("ignores the comment block git strips, and rejects an empty message", () => {
    expect(
      checkMessage("feat(x): a thing\n\nRefs #4\n# Please enter the commit...\n"),
    ).toBeNull();
    expect(checkMessage("# only comments\n")).not.toBeNull();
    expect(checkMessage("   \n")).toMatch(/empty/);
  });
});

describe("commit-msg hook entry point", () => {
  let directory;
  afterEach(() => {
    if (directory) rmSync(directory, { recursive: true, force: true });
  });

  it("fails a real commit message file that names no Issue", () => {
    directory = mkdtempSync(join(tmpdir(), "backup-mic-tasks-"));
    const file = join(directory, "COMMIT_EDITMSG");

    writeFileSync(file, "feat(x): untracked work\n");
    const rejected = spawnSync(process.execPath, [script, "message", file], {
      encoding: "utf8",
    });
    expect(rejected.status).toBe(1);
    expect(rejected.stderr).toMatch(/does not name a board Issue/);

    writeFileSync(file, "feat(x): tracked work\n\nRefs #99\n");
    const accepted = spawnSync(process.execPath, [script, "message", file], {
      encoding: "utf8",
    });
    expect(accepted.status).toBe(0);
  });
});

describe("board check", () => {
  const visible = () => true;

  it("names the Issues that are missing or off the board", () => {
    const answers = {
      1: { onBoard: true },
      2: { onBoard: false },
      3: "missing",
    };
    expect(boardState([1, 2, 3], (n) => answers[n], visible)).toEqual([
      { number: 2, reason: "not on the board" },
      { number: 3, reason: "no such Issue" },
    ]);
    expect(boardState([1], (n) => answers[n], visible)).toEqual([]);
  });

  it("reports nothing to act on when GitHub cannot be reached", () => {
    // gh missing or signed out must not turn into a false failure.
    expect(boardState([1, 2], () => null, visible)).toBeNull();
  });

  it("does not blame Issues when the account cannot see the board", () => {
    // Another signed-in account sees no private board, so every Issue looks off it.
    const asked = [];
    const run = (n) => {
      asked.push(n);
      return { onBoard: false };
    };
    expect(boardState([1, 2], run, () => false)).toBeNull();
    expect(asked).toEqual([]);
  });

  it("does not look for the board when no Issue is referenced", () => {
    expect(
      boardState([], visible, () => {
        throw new Error("the board was looked up");
      }),
    ).toEqual([]);
  });
});
