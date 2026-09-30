#!/usr/bin/env python3
"""Identify the single attached image device without relying on entity order."""

import plistlib
import re
import sys


def fixture_device(entities):
    devices = {
        entry.get("dev-entry", "")
        for entry in entities
        if re.fullmatch(r"/dev/disk[0-9]+", entry.get("dev-entry", ""))
    }
    if len(devices) != 1:
        raise ValueError("ambiguous fixture device")
    device = devices.pop()
    if any(
        not re.fullmatch(re.escape(device) + r"(?:s[0-9]+)?", entry.get("dev-entry", ""))
        for entry in entities
    ):
        raise ValueError("fixture entities do not belong to one image")
    return device


if __name__ == "__main__":
    if len(sys.argv) != 2:
        sys.exit("Usage: fixture-device.py ATTACH_PLIST")
    try:
        with open(sys.argv[1], "rb") as source:
            entities = plistlib.load(source)["system-entities"]
        print(fixture_device(entities))
    except (OSError, ValueError, KeyError, TypeError, AttributeError, plistlib.InvalidFileException):
        sys.exit("Fixture attachment identity could not be verified.")
