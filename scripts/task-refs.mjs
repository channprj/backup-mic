import { execFileSync, spawnSync } from "node:child_process";
import { readFileSync, realpathSync } from "node:fs";
import { fileURLToPath } from "node:url";

/**
 * Every commit that changes the project must name the board Issue it belongs
 * to, so the board and the repository cannot drift apart silently. Release
 * commits are written by the version bump and merges by git, so neither
 * carries an Issue of its own.
 */
const REFERENCE = /(?:^|\s)(?:Refs|Closes|Fixes)\s+#(\d+)\b/gi;
const RELEASE_SUBJECT = /^chore\(release\):\s/;
const MERGE_SUBJECT = /^(?:Merge|Revert)\s/;

export function referencedIssues(message) {
  return [...String(message).matchAll(REFERENCE)].map((m) => Number(m[1]));
}

export function isExempt(message) {
  const subject = String(message).split("\n", 1)[0].trim();
  return RELEASE_SUBJECT.test(subject) || MERGE_SUBJECT.test(subject);
}

/** `null` when the message is acceptable, otherwise why it is not. */
export function checkMessage(message) {
  const text = String(message)
    // A comment block is what git strips before committing.
    .split("\n")
    .filter((line) => !line.startsWith("#"))
    .join("\n");
  if (!text.trim()) return "The commit message is empty.";
  if (isExempt(text)) return null;
  if (referencedIssues(text).length > 0) return null;
  return [
    "This commit does not name a board Issue.",
    "",
    "Every task is tracked on the backup-mic GitHub Project, so the message",
    "needs a trailer naming the Issue it belongs to:",
    "",
    "    Refs #123",
    "",
    "Release and merge commits are exempt. See AGENTS.md.",
  ].join("\n");
}

const git = (...args) => execFileSync("git", args, { encoding: "utf8" }).trim();

/** Reports commits in `range` that name no Issue, newest first. */
export function untrackedCommits(range) {
  const log = git("log", "--no-merges", "--format=%H%x00%B%x01", range);
  return log
    .split("\x01")
    .map((entry) => entry.trim())
    .filter(Boolean)
    .map((entry) => {
      const [sha, message] = entry.split("\x00");
      return { sha, subject: message.split("\n", 1)[0], message };
    })
    .filter((commit) => checkMessage(commit.message) !== null);
}

const REPOSITORY = "channprj/backup-mic";

/**
 * Asks GitHub whether each Issue exists and sits on the board. Returns null
 * when `gh` is missing or not signed in, so a check never blocks work that
 * cannot reach the network.
 */
export function boardState(numbers, run = ghIssue) {
  const unknown = [];
  for (const number of numbers) {
    const answer = run(number);
    if (answer === null) return null;
    if (answer === "missing") unknown.push({ number, reason: "no such Issue" });
    else if (!answer.onBoard)
      unknown.push({ number, reason: "not on the board" });
  }
  return unknown;
}

function ghIssue(number) {
  const result = spawnSync(
    "gh",
    [
      "issue",
      "view",
      String(number),
      "--repo",
      REPOSITORY,
      "--json",
      "number,projectItems",
    ],
    { encoding: "utf8" },
  );
  if (result.error) return null; // gh is not installed
  if (result.status !== 0)
    return /not found|no issue/i.test(result.stderr ?? "") ? "missing" : null;
  try {
    const body = JSON.parse(result.stdout);
    return { onBoard: (body.projectItems ?? []).length > 0 };
  } catch {
    return null;
  }
}

function main(argv) {
  const [mode, ...rest] = argv;
  if (mode === "message") {
    const problem = checkMessage(readFileSync(rest[0], "utf8"));
    if (!problem) return 0;
    process.stderr.write(`${problem}\n`);
    return 1;
  }
  const range = rest[0] ?? "origin/main..HEAD";
  let commits;
  try {
    commits = untrackedCommits(range);
  } catch {
    process.stdout.write(`Could not read ${range}; skipping.\n`);
    return 0;
  }
  if (commits.length === 0) {
    process.stdout.write(`Every commit in ${range} names an Issue.\n`);
    const referenced = [
      ...new Set(
        git("log", "--no-merges", "--format=%B%x01", range)
          .split("\x01")
          .flatMap((message) => referencedIssues(message)),
      ),
    ];
    const unknown = boardState(referenced);
    if (unknown === null) {
      process.stdout.write("Skipped the board check: gh is unavailable.\n");
      return 0;
    }
    if (unknown.length === 0) {
      if (referenced.length)
        process.stdout.write(
          `All ${referenced.length} referenced Issues are on the board.\n`,
        );
      return 0;
    }
    for (const { number, reason } of unknown)
      process.stderr.write(`  #${number}: ${reason}\n`);
    return 1;
  }
  process.stderr.write(`Commits in ${range} with no Issue:\n`);
  for (const c of commits)
    process.stderr.write(`  ${c.sha.slice(0, 7)} ${c.subject}\n`);
  return 1;
}

if (
  process.argv[1] &&
  realpathSync(process.argv[1]) === fileURLToPath(import.meta.url)
)
  process.exit(main(process.argv.slice(2)));
