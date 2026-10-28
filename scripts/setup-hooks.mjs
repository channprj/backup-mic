import { execFileSync } from "node:child_process";
import { chmodSync, existsSync } from "node:fs";
import { resolve } from "node:path";

const git = (...args) => execFileSync("git", args, { encoding: "utf8" }).trim();
let root;
try {
  root = git("rev-parse", "--show-toplevel");
} catch {
  // A source archive has no hooks to install, and that must not fail the install.
  console.log("Not inside a Git work tree; skipped installing hooks.");
  process.exit(0);
}
let configured;
try {
  configured = git("config", "--get", "core.hooksPath");
} catch {
  /* Git default. */
}
if (configured && resolve(root, configured) !== resolve(root, ".githooks")) {
  throw new Error(
    `Existing core.hooksPath (${configured}) was preserved. Integrate .githooks/commit-msg into your hooks first.`,
  );
}
const hooks = ["commit-msg"];
for (const hook of hooks) {
  if (
    !configured &&
    existsSync(git("rev-parse", "--git-path", `hooks/${hook}`))
  )
    throw new Error(
      `Existing ${hook} hook was preserved. Integrate .githooks/${hook} into it first.`,
    );
}
for (const hook of hooks) chmodSync(resolve(root, `.githooks/${hook}`), 0o755);
git("config", "--local", "core.hooksPath", ".githooks");
console.log("Installed the Issue commit-msg hook for this repository.");
