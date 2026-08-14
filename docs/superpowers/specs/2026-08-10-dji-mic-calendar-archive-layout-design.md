# DJI Mic Calendar Archive Layout Design

**Date:** 2026-08-10
**Status:** Approved for implementation on 2026-08-14
**Target platform:** macOS 13 or newer
**Product:** DJI Mic Backup
**Relationship to the existing design:** This document replaces the visible recording-artifact path rules in the existing DJI Mic Backup specifications. Device identity, stable scanning, source-equal WAV copy, SHA-256 verification, AAC-LC 128 kbps conversion, SQLite evidence, collision safety, and recoverable macOS Trash behavior remain unchanged.

## Goal

Store final recording backups in a compact calendar hierarchy:

```text
YYYY/MM/YYMMDD-normalized-file-name.m4a
```

The required example is:

```text
2026/08/260809-T02_MIC003_20260809_195540_edit.m4a
```

The path is relative to the user-selected backup destination.

## Settled Naming Rules

1. The first directory is the four-digit recording year.
2. The second directory is the zero-padded two-digit recording month.
3. The file name begins with the recording date in `YYMMDD` form followed by `-`.
4. A leading source transmitter prefix is normalized only as follows:
   - `TX01_` becomes `T01_`.
   - `TX02_` becomes `T02_`.
   - Matching is ASCII case-insensitive.
5. The rest of the source base name is preserved exactly.
6. With M4A conversion enabled, the final visible extension is `.m4a` and the existing AAC-LC 128 kbps profile remains in force.
7. With M4A conversion disabled, the verified WAV uses the same directory and naming rule with a `.wav` extension. Existing source-retention rules remain in force.
8. A name without a recognized `TX01_` or `TX02_` prefix is not rewritten; it receives only the `YYMMDD-` archive prefix.

The destination date continues to come from a valid timestamp encoded in a DJI recording name. If that timestamp is malformed or unavailable, the existing stable modification-date fallback supplies the calendar date and archive prefix.

## Examples

```text
TX02_MIC003_20260809_195540_edit.wav
-> 2026/08/260809-T02_MIC003_20260809_195540_edit.wav
-> 2026/08/260809-T02_MIC003_20260809_195540_edit.m4a

TX01_MIC001_20260810_045047_edit.wav
-> 2026/08/260810-T01_MIC001_20260810_045047_edit.m4a

conversation.wav, fallback date 2026-08-10
-> 2026/08/260810-conversation.m4a
```

The WAV line in the first example is the verified destination copy created before conversion. After the complete conversion barrier succeeds, the WAV is moved to macOS Trash under the existing policy and the M4A remains as the final archive artifact.

## New Recording Flow

The backup pipeline keeps its current safety order:

1. Scan a stable source recording without following symlinks.
2. Derive the calendar date and normalized archive name.
3. Copy the WAV to an app-owned temporary file under `YYYY/MM`.
4. Flush and independently verify source and destination size and SHA-256.
5. Finalize the visible WAV without overwriting an existing path.
6. Convert the verified destination WAV to AAC-LC M4A at 128 kbps when enabled.
7. Inspect, hash, and commit the M4A evidence to SQLite.
8. Move the superseded destination WAV to macOS Trash only after the existing conversion barrier succeeds.
9. Revalidate live source and final artifact evidence before any source retirement becomes eligible.

No file is converted directly on a transmitter. No existing destination file is overwritten.

## Collision Behavior

For the requested default path:

- If no item exists, use the requested name.
- If a regular file with the same byte count and SHA-256 exists, reuse it.
- If another item or different file occupies the name, append the shortest unique SHA-256 prefix to the stem:

```text
2026/08/260809-T02_MIC003_20260809_195540_edit-1a2b3c4d.m4a
```

The no-clobber finalization and canonical-root checks remain mandatory.

## Existing Backup Migration

Every ledger-verified recording artifact that is present under the current destination is eligible for migration. Supported previous layouts include:

```text
YYYY/YYYY-MM-DD/TX01/file.m4a
YYYY/YYYY-MM-DD/TX02/file.m4a
YYYY/YYYY-MM-DD/file.m4a
```

Migration is performed before the next backup scan:

1. Read the verified artifact path, size, and SHA-256 from SQLite.
2. Require the old artifact to be a regular file inside the canonical destination root.
3. Recompute and require the recorded size and SHA-256.
4. Derive the new `YYYY/MM/YYMMDD-...` target from the verified recording date and normalized file name.
5. Copy through an app-owned temporary file and synchronize it.
6. Recompute and require the target size and SHA-256.
7. Move the previous artifact to macOS Trash.
8. Atomically update the ledger path only after the target and Trash operations succeed.
9. Remove empty legacy transmitter and date directories. The shared year directory remains because it owns the new month directories.

If the old path is absent but a matching verified target already exists, the migration repairs the ledger path without moving anything. If neither path can be verified, it skips that record and preserves the ledger evidence for diagnosis. A collision follows the same hash-suffix rule as a new backup.

Already migrated paths are idempotent and produce no work on later runs.

## Scope Boundaries

- The layout change applies to verified recording WAV and M4A artifacts.
- Raw non-recording session files remain under the existing `source-extras` hierarchy so they cannot be confused with playable archive recordings.
- Daily diagnostic logs already use `logs/YYYY/MM/YYMMDD-backup-mic.log` and are unchanged.
- The selected backup destination and its validation policy are unchanged.
- The design does not introduce MP4 video output. The requested example and the established audio pipeline use the `.m4a` container.

## Error Handling

Any read, copy, synchronization, hash, Trash, or ledger failure aborts that artifact migration without deleting the old verified file. The background operation reports its existing structured error and writes the failure to the daily diagnostic log. A partial successful migration is safe because each artifact is independently verified and ledger relocation is monotonic.

## Verification Requirements

Automated tests must prove:

1. `TX02_MIC003_20260809_195540_edit.wav` plans as `2026/08/260809-T02_MIC003_20260809_195540_edit.wav`.
2. M4A conversion preserves the same archive stem and changes only the extension.
3. TX01 and TX02 recordings share one year/month directory.
4. Unknown prefixes are preserved and receive only the archive-date prefix.
5. Equal content is reused and different-content collisions receive a hash suffix.
6. Each supported previous layout migrates to the new layout.
7. Migration preserves bytes and SHA-256, updates SQLite, moves the prior artifact to Trash, and removes empty legacy directories.
8. A mismatched old artifact is not migrated or removed.
9. Re-running migration is a no-op.
10. Repository-wide Rust tests, Clippy, frontend tests, type checking, and production build pass.

Installed-app verification must prove:

1. The version was increased before packaging.
2. The installed executable matches the packaged executable hash.
3. Connected DJI volumes are detected by the release build.
4. Present verified backup artifacts appear directly under `YYYY/MM` with the `YYMMDD-T01/T02_...m4a` names.
5. File size and SHA-256 match the relocated SQLite evidence.
6. The legacy recording directories are absent when empty.
7. The fresh release log contains successful migration and backup completion events without a new error.

## Release and Git Contract

The implementation is published as verified Conventional Commit checkpoints through ordinary pushes. After implementation and full verification, Headatever increments the version, creates the release commit and annotated tag, and pushes both without rewriting history. The new bundle replaces the local installed app only after its signature, metadata, and checksums pass; the prior app bundle moves to macOS Trash.
