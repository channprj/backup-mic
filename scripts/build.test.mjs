// @vitest-environment node
import { readFileSync } from "node:fs";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const script = fileURLToPath(new URL("./build.sh", import.meta.url));
const packageJson = JSON.parse(
  readFileSync(new URL("../package.json", import.meta.url), "utf8"),
);

function select(args) {
  return spawnSync(
    "bash",
    ["-c", 'source "$1"; build_actions "${@:2}"', "bash", script, ...args],
    { encoding: "utf8" },
  );
}

function lines(result) {
  return result.stdout.split("\n").filter((line) => line.length > 0);
}

describe("pnpm build argument selection", () => {
  it("is the script package.json runs", () => {
    expect(packageJson.scripts.build).toBe("bash scripts/build.sh");
  });

  it("selects the frontend build only when called with no arguments", () => {
    const result = select([]);
    expect(result.status).toBe(0);
    expect(lines(result)).toEqual(["frontend"]);
  });

  it("selects packaging and installation for install", () => {
    const result = select(["install"]);
    expect(result.status).toBe(0);
    expect(lines(result)).toEqual(["package", "install"]);
  });

  it("selects packaging, installation, and launch for open", () => {
    const result = select(["open"]);
    expect(result.status).toBe(0);
    expect(lines(result)).toEqual(["package", "install", "launch"]);
    const both = select(["install", "open"]);
    expect(both.status).toBe(0);
    expect(lines(both)).toEqual(["package", "install", "launch"]);
  });

  it("rejects an unknown argument and a bare -- from the shipped entry", () => {
    for (const args of [["bogus"], ["--"]]) {
      const selected = select(args);
      expect(selected.status).toBe(2);
      expect(selected.stderr).toMatch(/unknown argument/);
      expect(selected.stdout).toBe("");

      const entry = spawnSync("bash", [script, ...args], { encoding: "utf8" });
      expect(entry.status).toBe(2);
      expect(entry.stderr).toMatch(/unknown argument/);
      expect(entry.stdout).not.toMatch(/tauri build|Installed app:/);
    }
  });
});
