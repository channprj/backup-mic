#!/usr/bin/env python3
"""Snapshot both the index and non-ignored working files without following links outside Git."""

import os
from pathlib import Path
import stat
import subprocess
import sys


def git(*args):
    return subprocess.check_output(["git", *args])


def relative_path(raw):
    path = Path(os.fsdecode(raw))
    if path.is_absolute() or ".." in path.parts or not path.parts:
        raise ValueError("unsafe repository path")
    return path


def snapshot(destination):
    root = Path(git("rev-parse", "--show-toplevel").decode().strip()).resolve()
    for entry in git("ls-files", "--stage", "-z").split(b"\0"):
        if not entry:
            continue
        metadata, name = entry.split(b"\t", 1)
        mode, oid, stage = metadata.split()
        if stage != b"0" or mode not in (b"100644", b"100755", b"120000"):
            raise ValueError("unmerged entries and submodules require separate review")
        output = destination / "index" / relative_path(name)
        output.parent.mkdir(parents=True, exist_ok=True)
        output.write_bytes(git("cat-file", "blob", oid.decode("ascii")))

    names = set(git("ls-files", "--cached", "--others", "--exclude-standard", "-z").split(b"\0"))
    for name in names - {b""}:
        path = relative_path(name)
        source = root / path
        # A linked parent can redirect even lstat/readlink; reject it before reading.
        if not source.parent.resolve().is_relative_to(root):
            raise ValueError("working path escapes the repository")
        try:
            metadata = source.lstat()
        except FileNotFoundError:
            continue  # Deleted working files are still scanned from the index.
        if stat.S_ISLNK(metadata.st_mode):
            content = os.fsencode(os.readlink(source))
        elif stat.S_ISREG(metadata.st_mode):
            content = source.read_bytes()
        else:
            raise ValueError("unsupported working file type")
        output = destination / "worktree" / path
        output.parent.mkdir(parents=True, exist_ok=True)
        output.write_bytes(content)


if __name__ == "__main__":
    try:
        snapshot(Path(sys.argv[1]))
    except (OSError, ValueError, IndexError, subprocess.CalledProcessError) as error:
        sys.exit(f"Security snapshot failed ({type(error).__name__}).")
