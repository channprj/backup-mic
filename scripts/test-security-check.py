#!/usr/bin/env python3
"""Regression checks: staging must not hide a token and links must not expose external files."""

from pathlib import Path
import random
import shutil
import string
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parent.parent


class SecurityCheckTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name) / "repository"
        self.root.mkdir()
        self.run_command("git", "init", "--quiet")
        (self.root / ".gitleaks.toml").write_bytes((ROOT / ".gitleaks.toml").read_bytes())

    def run_command(self, *command):
        return subprocess.run(command, cwd=self.root, capture_output=True, check=True)

    def scan(self):
        snapshot = Path(self.temp.name) / "snapshot"
        snapshot.mkdir()
        result = subprocess.run(
            ["python3", str(ROOT / "scripts/security-snapshot.py"), str(snapshot)],
            cwd=self.root, capture_output=True,
        )
        self.assertEqual(result.returncode, 0)
        return subprocess.run(
            ["gitleaks", "dir", str(snapshot), "--config", str(self.root / ".gitleaks.toml"),
             "--redact=100", "--no-banner"],
            cwd=self.root, capture_output=True,
        ).returncode

    def token_file(self):
        # Deliberately synthetic; assemble so this test itself contains no credential.
        path = self.root / "configuration.txt"
        sample = "".join(random.Random(42).choices(string.ascii_letters + string.digits, k=36))
        path.write_text("github_token=" + "ghp_" + sample + "\n")
        return path

    def test_untracked_working_token_is_detected(self):
        self.token_file()
        self.assertEqual(self.scan(), 1)

    def test_staged_token_is_detected_after_working_copy_is_cleaned(self):
        path = self.token_file()
        self.run_command("git", "add", "configuration.txt")
        path.write_text("no credentials here\n")
        self.assertEqual(self.scan(), 1)

    def test_external_symlink_content_is_not_read(self):
        external = Path(self.temp.name) / "outside.txt"
        shutil.move(self.token_file(), external)
        (self.root / "linked.txt").symlink_to(external)
        self.assertEqual(self.scan(), 0)

    def test_linked_parent_outside_repository_is_rejected(self):
        directory = self.root / "configuration"
        directory.mkdir()
        (directory / "settings.txt").write_text("public settings\n")
        self.run_command("git", "add", "configuration/settings.txt")
        outside = Path(self.temp.name) / "outside"
        directory.rename(outside)
        directory.symlink_to(outside, target_is_directory=True)
        result = subprocess.run(
            ["python3", str(ROOT / "scripts/security-snapshot.py"),
             str(Path(self.temp.name) / "snapshot")],
            cwd=self.root, capture_output=True,
        )
        self.assertNotEqual(result.returncode, 0)


if __name__ == "__main__":
    unittest.main()
