# DJI Mic Backup M4A, Trash, Logging, and Settings Design

**Date:** 2026-08-09
**Status:** Implemented and locally installed
**Target platform:** macOS 13 or newer
**Product:** DJI Mic Backup
**Relationship to the baseline:** This document supersedes the scheduling, destination-artifact, deletion, settings, and presentation behavior in `2026-08-09-dji-mic-mini-tauri-rust-backup-design.md`. The baseline device-identity, canonical-path, no-clobber, SQLite, and narrow-IPC trust boundaries remain in force.

## Problem and Current Evidence

The installed application successfully copied and SHA-256 verified `TX02_MIC003_20260809_195540_edit.wav`. The source and destination were both 56,859,016 bytes and had the same SHA-256. The SQLite ledger records the successful backup run at 2026-08-09 20:01 KST.

The investigation nevertheless found a real scheduling gap: the orchestrator starts automatic work when Disk Arbitration reports a device lifecycle event, but it does not rescan a transmitter that remains mounted. A recording created after the initial mount can therefore wait indefinitely unless another disk event occurs or the user presses `지금 백업`.

The existing source-retirement flow also permanently unlinks each verified WAV. After the user retired the current TX02 recordings, `/Volumes/DJI-MIC-2/TX_MIC001_20260809_021747` remained as an empty folder. The requested behavior is instead to preserve recoverability by moving a complete verified recording-session folder to the external volume's Trash.

The destination currently stores WAV files. The desired steady state is a verified M4A artifact by default, an inspectable daily log under the selected destination, explicit settings for automation, and a more polished macOS interface.

## Goals

1. Detect recordings added while a paired transmitter remains mounted.
2. Preserve the existing copy-first and source-hash safety boundary before any lossy conversion.
3. Produce AAC-LC M4A files by default and retain WAV only when the user disables conversion.
4. Verify the M4A container, codec, audio shape, duration, valid frames, durability, and content hash before treating it as a backup artifact.
5. Write append-only daily backup history to `<destination>/logs/YYYY/MM/YYMMDD-backup-mic.log`.
6. Move complete verified session folders to the external volume's macOS Trash instead of permanently unlinking files.
7. Support transmitter-by-transmitter automatic Trash movement after each transmitter's complete snapshot succeeds, with the setting off by default.
8. Preserve the existing manual retirement action, but make it use the same Trash and revalidation pipeline.
9. Remove known empty session folders left by a previously completed retirement without recursively deleting unknown content.
10. Add a dedicated settings window and refine the status popover using familiar macOS hierarchy, materials, motion, and accessibility behavior.
11. Rebuild, package, replace the locally installed app, relaunch it, and prove installed-artifact parity after implementation.

## Non-Goals

- Cloud backup, telemetry, audio playback, waveform browsing, tagging, or transcription.
- Supporting non-DJI recorders or arbitrary removable volumes.
- User-configurable codecs, expert encoder flags, or arbitrary scan intervals in this iteration.
- Treating an M4A hash as equal to a WAV hash; AAC is intentionally lossy.
- Recursively deleting or trashing a directory that contains an unknown file, hidden file, symlink, or unexpected nested directory.
- Automatically emptying either the Mac Trash or an external volume's Trash.
- Ejecting the transmitters after backup.

## Settled Product Decisions

- New recordings are checked by a metadata-only periodic scan every 15 seconds while a trusted transmitter remains mounted.
- `지금 백업` always forces a full scan and destination revalidation.
- M4A conversion defaults to on. The fixed initial encoding profile is AAC-LC, 192 kbps constrained VBR, source sample rate, and source channel count.
- Automatic backup defaults to on.
- Automatic source retirement defaults to off and is labeled `백업 후 휴지통으로 이동`.
- Enabling automatic retirement requires a one-time, explicit safety confirmation. Disabling it is immediate.
- Manual and automatic retirement move items to Trash; neither path permanently unlinks production source recordings.
- Daily file logging is mandatory and has no off switch because it is part of the safety audit trail.
- Each transmitter is independent. A completely verified TX01 may be moved to Trash even if TX02 fails, and vice versa.
- The selected backup destination remains `/Users/channprj/Documents/DJI-Mic-Mini-2S` unless the user changes it.

## Architecture

The existing boundary remains: React renders state, the Tauri adapter owns platform integration, and `backup-core` owns safety policy.

```text
React status popover + dedicated settings window
                 │ narrow typed intents and snapshots
                 ▼
Tauri application adapter
  - scheduling and lifecycle wiring
  - Apple audio-tool invocation
  - macOS FileManager Trash adapter
                 │ policy inputs and platform traits
                 ▼
backup-core
  - scan fingerprinting and backup plans
  - staged copy and hash verification
  - conversion validation policy
  - durable log events and SQLite ledger
  - complete-session Trash eligibility
```

The core gains narrow traits for audio conversion, audio inspection, audit-log writing, and recoverable source disposal. Production implementations remain in the macOS/Tauri layer. Tests use deterministic fakes and temporary Trash directories.

## Mounted-Volume Rescan Scheduler

Disk Arbitration remains authoritative for appearance, disappearance, mount generation, and device identity. A periodic deadline is added only after a trusted mounted device exists.

1. On mount, run the existing stable scan immediately.
2. Every 15 seconds, enumerate visible regular WAV metadata without hashing audio.
3. Build a per-transmitter fingerprint from relative path, byte count, and modification time.
4. If the fingerprint is unchanged, do nothing and create no backup run or activity entry.
5. If it changed, coalesce one pending backup request and run the existing two-observation stability scan.
6. If the operation guard is busy, keep one pending request and start it after the active operation releases the guard.
7. On unmount, cancel the deadline and invalidate pending work and retirement authority.

A file that changes during the two-second settling observation is deferred. Its metadata changes again at the next interval, causing a later retry. The scheduler never treats a volume label or mount path as trust authority.

## Destination Artifact Pipeline

### Common copy-first boundary

Every new source WAV first passes the existing safe-copy contract:

1. Capture source metadata and canonical relative path.
2. Copy the source read-only to an app-owned staging WAV under the destination.
3. Flush, close, and `sync_all` the staging WAV.
4. Re-read the source metadata.
5. Independently hash source and staging WAV.
6. Require identical byte counts and SHA-256 hashes.

The source remains untouched if any step fails. Capacity preflight reserves enough room for the staging WAV, a simultaneous conversion output up to the WAV size, and the existing 10 GiB safety reserve.

### M4A mode

When M4A conversion is enabled:

1. Invoke `/usr/bin/afconvert` with explicit argument boundaries, no shell, AAC-LC, 192 kbps constrained VBR, maximum codec quality, and the source channel count and sample rate.
2. Write to an app-owned `.m4a.part-<uuid>` path on the destination filesystem.
3. Require a successful process exit and reject empty output or unexpected stderr failures.
4. Invoke `/usr/bin/afinfo -x` without a shell and parse its structured XML.
5. Require `m4af`, AAC, the source channel count, the source sample rate, nonzero audio bytes and packets, and valid frames matching the source PCM frame count.
6. Allow only the encoder's declared priming and remainder frames; the reported playable duration must match the source within one AAC packet at the source sample rate.
7. Hash the complete M4A, flush its containing directory after atomic finalization, and record its hash and audio properties in SQLite.
8. Atomically rename the part file to `<original-stem>.m4a` using the existing no-clobber collision contract.
9. Remove only the app-owned staging WAV after the final M4A and ledger transaction are durable.

The final artifact is the M4A. The M4A hash proves later integrity; it is not compared with the source WAV hash. The source WAV hash and decoded audio-shape checks prove that the final artifact came from the verified source.

### WAV mode

When M4A conversion is disabled, the verified staging WAV is atomically finalized with the current `.wav` layout and equal source/destination hashes.

### Existing WAV migration

Existing verified destination WAV files are migrated conservatively when M4A mode is first enabled:

1. Re-hash the existing WAV against its ledger entry.
2. Convert and validate an M4A through the same part-file pipeline.
3. Commit the M4A artifact to the ledger.
4. Move the superseded user-visible destination WAV to the Mac Trash rather than permanently deleting it.

Migration never grants external-source retirement unless the live source also passes the current complete-snapshot preflight.

## SQLite and Recovery Model

A forward-only migration adds persisted settings and artifact metadata without rewriting historical hashes:

- output format (`wav` or `m4a`)
- artifact SHA-256 and byte count
- codec, sample rate, channel count, valid frame count, and duration
- conversion status and conversion error code
- source retirement status (`present`, `trash_pending`, `moved_to_trash`, `legacy_deleted`, `failed`)
- retired session relative path when applicable
- automatic-backup, M4A, and automatic-Trash settings

Interrupted M4A part files and staging WAVs are reconciled by app-owned naming. A valid finalized M4A missing only its ledger commit is inspected and hashed before the ledger is repaired. Invalid or ambiguous parts are quarantined or removed only when their app ownership is proven. A corrupt ledger continues to disable retirement until artifacts are re-indexed.

## Daily Audit Log Contract

The selected destination contains:

```text
DJI-Mic-Mini-2S/
  logs/
    2026/
      08/
        260809-backup-mic.log
```

The date and timestamp use the Mac's local timezone at event time. Each UTF-8 line is independently parseable and human-readable:

```text
2026-08-09T20:01:42.613+09:00 INFO backup.verified tx=TX02 source="TX02_MIC003_20260809_195540_edit.wav" output="2026/2026-08-09/TX02/TX02_MIC003_20260809_195540_edit.m4a" source_bytes=56859016 output_bytes=... format=m4a
```

Events cover device detection, scan start, file discovery, copy completion, source/staging hash verification, conversion start/completion, M4A inspection, finalization, run completion, refusal reasons, manual or automatic Trash preflight, Trash movement, migration, and recovery.

The log excludes absolute source paths, volume UUIDs, full content hashes, opaque proposal IDs, and audio data. Values are escaped so a filename cannot forge a second line. One process-wide writer serializes appends. Run-completion and pre-retirement entries are flushed with `sync_data`.

SQLite remains the authoritative transactional ledger. If file-log creation or flush fails, backup artifacts may remain safely completed, but automatic or manual source retirement is refused and the UI reports that the audit trail must be restored first.

## Recoverable Source Retirement

The existing two-pass authority model remains, but the disposal action changes from permanent unlink to macOS Trash.

### Complete-session grouping

Verified candidates are grouped by a top-level source directory whose name matches the anchored DJI session form:

```text
^TX_MIC[0-9]+_[0-9]{8}_[0-9]{6}$
```

Immediately before issuing or executing retirement authority, the core recursively inventories each candidate session without following symlinks. The set must contain only the verified regular WAV candidates for that session. An unknown file, hidden entry, symlink, unexpected nested directory, changed path, or missing candidate refuses the whole session before anything moves.

Root-level verified WAV files that do not belong to a recognized session directory are moved to Trash individually. A recognized session directory is moved as one item so the folder and recordings remain together and recoverable.

### macOS Trash adapter

The production adapter calls Foundation `FileManager.trashItem(at:resultingItemURL:)` through a narrow macOS module. It does not invoke Finder, AppleScript, a shell, `rm`, or direct `.Trashes` path manipulation. macOS therefore chooses the correct Trash on the source volume and resolves name collisions.

The core records only a privacy-safe outcome, not the absolute resulting Trash path. Moving more than one session is reported item by item; if a later move fails, the ledger and UI accurately report a partial retirement and never repeat a completed move.

### Manual and automatic paths

- **Manual:** the user selects `휴지통으로 이동`, sees transmitter, session, file, and byte totals, and confirms a five-minute opaque proposal.
- **Automatic:** after a transmitter's complete backup and final-artifact verification succeeds, the app performs the same fresh preflight without a frontend proposal only when the persisted setting is enabled. There is no weaker automatic path.

Both paths require current paired identity, mount generation, scan generation, complete source set, source metadata and SHA-256, final artifact hash and audio properties, idle operation guard, durable ledger, and flushed audit log.

### Empty-folder reconciliation

The upgrader recognizes an empty session directory left by an earlier successful legacy deletion only when all of these are true:

- its anchored session name and canonical parent are valid;
- it is completely empty at the time of the action;
- the ledger contains retired recordings whose source paths name that directory; and
- the current mounted device still matches the paired identity.

The empty directory is moved to Trash and logged. A merely empty look-alike directory without ledger evidence is left alone.

## Settings and User Interface

### Status popover

The menu-bar popover remains the fast status surface. It uses a calm adaptive material, system typography, and clear hierarchy rather than stacked translucent cards.

- Header: product mark, current state sentence, connected transmitter count, and Settings button.
- Hero status: one dominant progress or completion value.
- Stage sequence: `복사 → 원본 검증 → M4A 변환 → M4A 검증 → 휴지통 이동`.
- TX01/TX02 rows: mount state, current stage, verified count, and retirement outcome.
- Recent activity: concise durable events with a shortcut to the daily log folder.
- Footer: `지금 백업`, destination shortcut, and Quit.

The information density follows CodexBar's glanceable grouped status, while the small set of direct menu actions follows Klack's restraint.

### Dedicated settings window

The Settings button opens or focuses one singleton standard macOS settings window. It contains grouped sections:

**Backup**

- destination path and `변경…`
- `자동으로 백업` switch, default on
- `M4A로 변환` switch, default on
- fixed-profile explanation: AAC-LC, high-quality voice and general recording profile

**Source safety**

- `백업 후 휴지통으로 이동` switch, default off
- inline explanation that each transmitter is handled independently
- a confirmation alert when enabling
- last refusal or partial-retirement state when relevant

**General**

- launch at login
- open backup folder
- open current log folder
- app version

Settings persist only after the Rust command succeeds. A failed write restores the visible switch and shows an inline error.

### Apple-style interaction contract

- Controls react on pointer-down and remain usable during non-destructive transitions.
- Popover and settings transitions use critically damped motion with no decorative bounce.
- Wayfinding is explicit; the settings window has a standard title and close behavior.
- Light and dark appearances use system colors and one material layer per hierarchy level.
- `prefers-reduced-motion` replaces spatial movement with a short cross-fade.
- `prefers-reduced-transparency` uses opaque adaptive surfaces.
- Keyboard focus, VoiceOver names, minimum hit targets, contrast, and Dynamic Type-safe spacing are required.

## Errors and Recovery

- **New file remains active:** defer it and retry after a later stable fingerprint.
- **Periodic scan fails:** keep the source untouched, log the error, and retry while the mount remains trusted.
- **Source changes during copy or conversion:** discard only app-owned staging artifacts and retry later.
- **`afconvert` or `afinfo` unavailable or invalid:** leave the verified staging WAV recoverable, report conversion failure, and never retire the source.
- **M4A inspection mismatch:** quarantine the part artifact and never publish or retire.
- **Destination collision:** reuse an identical valid artifact or apply the existing hash-suffix no-clobber rule.
- **Audit log unavailable:** retain completed backup artifacts but refuse source retirement.
- **Trash API failure:** keep the source in place when no move occurred; otherwise record exact partial outcomes.
- **Unknown session content:** refuse the whole session and name the safe next action without exposing a full private path in the UI.
- **Device removal:** cancel active work, invalidate authority, and never retry until a newly trusted mount generation appears.

## Testing Strategy

### Core tests

- unchanged metadata fingerprints schedule no work; added, removed, or changed WAV metadata schedules one coalesced run;
- busy operations preserve exactly one pending rescan;
- M4A is never eligible before verified WAV staging, conversion inspection, atomic finalization, hash, ledger commit, and log flush;
- AAC priming and remainder tolerance accepts the observed valid output and rejects truncated or wrong-duration output;
- M4A and WAV modes produce the correct deterministic extension and collision behavior;
- log paths roll over by local year, month, and `YYMMDD` date;
- log escaping prevents newline or delimiter injection;
- complete-session grouping rejects unknown entries, hidden files, symlinks, traversal, and changed candidates;
- automatic retirement uses every manual preflight invariant;
- empty-folder reconciliation requires both an empty directory and matching legacy ledger evidence.

### Integration tests

- create a WAV after a test volume is already mounted and prove the periodic scheduler discovers it;
- convert representative mono 48 kHz Float32 DJI WAV fixtures to M4A and inspect them through the production Apple tools on macOS;
- interrupt each copy, conversion, finalization, ledger, log, and Trash boundary and prove conservative recovery;
- migrate a verified legacy WAV to M4A without losing either artifact before commit;
- use a fake Trash to prove whole-session movement and precise partial outcomes;
- prove settings persist across app restart and failed writes roll back in the UI;
- validate light, dark, reduced-motion, reduced-transparency, keyboard, and accessibility states.

### Hardware and disposable-volume acceptance

- Keep production transmitters read-only during backup and M4A proof.
- Independently compare live source WAV hashes with the staging evidence recorded by the ledger.
- Validate actual external-volume Trash behavior only on `/Volumes/DJI-DELTEST` or another explicitly disposable FAT32 fixture.
- After production code passes, reconcile the currently empty TX02 session directory only through the ledger-backed empty-folder rule.
- Confirm the production external volume contains no app-owned partial files after the run.

## Delivery and Local Installation

The implementation is complete only after:

1. `./scripts/check.sh` and all new conversion, log, scheduler, settings, and Trash tests pass.
2. The disposable FAT32 Trash acceptance passes without permanent unlinking.
3. Real connected devices pass identity and read-only backup/M4A verification.
4. The UI is checked in copying, converting, completed, refused, partial, settings, light/dark, and reduced-motion states.
5. A release app and DMG are rebuilt.
6. `/Users/channprj/Applications/DJI Mic Backup.app` is replaced through the repository's packaging workflow and relaunched.
7. The release executable and installed executable have identical SHA-256 values.
8. The installed app passes deep strict code-sign verification, creates its status item, reads the persisted settings, writes the expected daily log, and observes the currently mounted paired devices.
9. Every outcome checkpoint is pushed and local/tracking/live-remote parity is `0 0`.

## Acceptance Criteria

- A stable WAV added after a paired transmitter is already mounted starts scanning within one 15-second interval without manual input.
- `TX02_MIC003_20260809_195540_edit.wav` remains represented by a verified final artifact and durable ledger entry throughout migration.
- With default settings, the final user-visible backup artifact is `.m4a`, not `.wav`.
- Every M4A reported as complete has passed container, codec, channel, sample-rate, playable-duration, valid-frame, byte-count, and hash verification.
- The exact daily log path follows `logs/2026/08/260809-backup-mic.log` for 2026-08-09 local events.
- The daily log identifies per-file backup, conversion, verification, and Trash outcomes without exposing absolute source paths or device UUIDs.
- Automatic retirement remains off until explicitly enabled.
- When enabled, a fully verified transmitter may retire independently of the other transmitter.
- A verified DJI session folder is moved intact to the external volume's Trash; production files are never permanently unlinked.
- An unknown or changed item in a session prevents the whole session from moving.
- A ledger-backed empty legacy session folder is moved to Trash and no longer remains at the source root.
- The dedicated settings window controls destination, automatic backup, M4A conversion, automatic Trash movement, autostart, and log-folder access.
- The status popover visibly distinguishes copy, source verification, conversion, M4A verification, and Trash stages.
- Failures preserve the strongest recoverable artifact and never grant weaker retirement authority.
- The rebuilt local installation matches the verified release artifact and runs successfully.
