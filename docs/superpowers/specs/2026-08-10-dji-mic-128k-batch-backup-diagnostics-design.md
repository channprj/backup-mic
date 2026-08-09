# DJI Mic Backup 128 kbps Batch Conversion, Diagnostics, and Release Design

**Date:** 2026-08-10
**Status:** Design approved; written specification awaiting user review
**Target platform:** macOS 13 or newer
**Product:** DJI Mic Backup
**Relationship to the existing design:** This document supersedes the per-file conversion order, 192 kbps profile, transmitter-independent retirement boundary, unknown-session-file refusal, setting-command execution, error-log coverage, and release-version behavior in `2026-08-09-dji-mic-m4a-trash-settings-design.md`. The established device identity, canonical-path, no-clobber, SHA-256, SQLite, opaque proposal, Foundation Trash, and narrow IPC trust boundaries remain in force.

## Problem and Current Evidence

The installed app currently performs the following loop for each recording:

```text
copy one WAV -> verify it -> convert it to M4A -> verify the M4A -> continue
```

Its production Apple audio command requests AAC-LC at 192,000 bits per second. The requested workflow is instead a batch pipeline: every WAV must first exist as a durable, source-equal backup in the destination; only after that barrier may the destination WAVs be converted to 128 kbps M4A. No live source may become eligible for retirement until every required conversion and final verification in the run succeeds.

The current source-retirement policy also explains the reported generic failure. A live recognized session contains an externally generated M4A plus its FAT32 AppleDouble metadata sidecar in addition to the WAV recordings. The M4A identifies an FFmpeg/Lavf encoder rather than the app's Apple audio pipeline. Existing policy rejects any unknown or hidden session entry before creating a retirement proposal. That refusal is safe, but the failure is not written to the daily log and the UI falls back to:

```text
작업을 완료하지 못했습니다
원본은 그대로 유지됩니다. 잠시 후 다시 시도해 주세요.
```

The setting failure has the same observability gap. Idle installed-app checks proved that M4A and login-start settings can persist and restore successfully. The reported failure overlapped a long existing-artifact verification run, while synchronous setting commands share the same long-held ledger mutex. The exact original error cannot be recovered because command failures are not durably logged. The design must therefore remove that contention and record every future failure at the command boundary.

The repository has no Headatever `VERSION` file. A read-only run of the bundled Headatever script shows that the first release version on 2026-08-10 will be `0.260810.0`.

## Goals

1. Copy and SHA-256 verify every stable source WAV into the backup destination before any new M4A conversion starts.
2. Convert destination WAVs to AAC-LC M4A at 128 kbps while preserving source sample rate and channel count.
3. Convert every ledger-verified historical destination WAV, not only newly discovered recordings.
4. Require one run-wide conversion and verification barrier across all mounted paired transmitters before granting any live-source retirement authority.
5. Back up and verify additional regular session files, including source M4A and AppleDouble sidecars, before moving a session folder to Trash.
6. Re-read and re-hash live sources and revalidate final destination artifacts after conversion, immediately before retirement authorization.
7. Move complete session folders and eligible empty legacy session folders to macOS Trash; never permanently delete production source material.
8. Keep live sources in place whenever M4A conversion is disabled, even if the WAV copy is verified.
9. Persist settings without blocking the UI or racing a long backup run, and apply a frozen setting snapshot for the duration of each run.
10. Write actionable, privacy-safe failure records for every background and command error to the daily log, with a fallback diagnostic log when the destination log is unavailable.
11. Replace generic UI failures with stage-specific guidance and a direct log shortcut.
12. Initialize and publish Headatever version `0.260810.0` after development verification and before packaging, local installation, and relaunch.

## Non-Goals

- Configurable bitrates, codecs, sample rates, channel remixing, or expert encoder flags.
- Converting files in place on a transmitter.
- Permanently deleting any production source, destination WAV, prior app bundle, or installer rollback artifact.
- Emptying macOS Trash or a removable volume's Trash.
- Following symlinks, recursively accepting nested directories, or backing up device/system directories outside a recognized recording session.
- Treating a lossy M4A hash as equal to its WAV source hash.
- Removing an unknown session entry without first creating and verifying its own destination evidence.
- Making an in-progress run change behavior when a setting is toggled midway through it.
- Publishing a GitHub Release; the requested release scope is commit, annotated tag, ordinary push, local package, install, and run.

## Settled Product Decisions

- Automatic backup remains on by default.
- `WAV 백업 후 M4A로 변환` remains on by default.
- The fixed encoder profile is AAC-LC, requested average bitrate 128,000 bits per second, maximum codec quality, source sample rate, and source channel count.
- `백업 후 휴지통으로 이동` remains off by default and still requires explicit acknowledgement before it can be enabled.
- When M4A conversion is on, all new and historical eligible WAVs join the conversion cohort. Any cohort failure preserves every live source.
- When M4A conversion is off, the app may create and verify WAV backups, but neither manual nor automatic source retirement authority is issued.
- A run uses one immutable preference snapshot captured at its start. A saved setting applies to the next run.
- Automatic retirement uses the run-wide barrier only when its setting is on. With automatic retirement off, the same completed barrier enables the existing five-minute manual proposal.
- Destination WAVs superseded by verified M4As are moved to macOS Trash after the cohort commits. They are never unlinked.
- A recognized source session is retired as one folder item, so successfully moving it cannot leave a new empty directory behind.
- An already empty recognized session is moved to Trash only with exact paired-device and ledger evidence.
- The selected destination remains `/Users/channprj/Documents/DJI-Mic-Mini-2S` unless the user changes it.

## Architecture

The existing React, Tauri adapter, and `backup-core` policy boundaries remain. The orchestration changes from a nested per-file transaction to an explicit run state machine.

```text
React status + settings
  - narrow intents
  - exact error code and safe recovery copy
                    |
                    v
Tauri application adapter
  - run coordinator and frozen preferences
  - Apple audio tools
  - Foundation Trash
  - primary and fallback diagnostic sinks
                    |
                    v
backup-core
  - complete source inventory
  - copy cohort and conversion cohort
  - artifact provenance and revalidation
  - run-wide retirement barrier
  - SQLite evidence and recovery policy
```

The run coordinator owns one ordered `BackupRunManifest`. It contains the paired mount generations, destination generation, frozen preferences, stable source observations, additional-file observations, eligible historical WAV artifacts, and the state of every barrier. Frontend snapshots expose only counts, logical transmitter labels, stages, and privacy-safe errors.

## Run State Machine

### Phase 0: Capture authority and preferences

The coordinator acquires the existing operation guard and captures:

- the current paired device identities and mount generations;
- the destination generation and canonical root;
- automatic-backup, M4A-conversion, and automatic-Trash settings;
- a new run ID and start timestamp.

The captured settings never change during the run. A concurrent setting save updates durable preferences and the settings window, but the coordinator reports that the new value applies to the next run.

### Phase 1: Build the complete inventory

Perform the existing two-observation stable scan for all mounted paired transmitters. A recognized session may contain:

- DJI-named regular WAV recordings;
- regular M4A files;
- regular AppleDouble files whose names begin with `._`;
- other regular files with safe UTF-8 relative names.

The inventory does not follow symlinks and does not accept nested directories within a session. A symlink, non-regular entry, unsafe name, or nested directory keeps the entire session ineligible and produces an explicit logged refusal.

Every regular non-WAV session file is an additional-source item. It receives its own byte count, modification time, SHA-256, destination-relative path, and ledger evidence. Additional files are never assumed to be equivalent to the WAV or to the app-produced M4A.

Source M4As are inspected with `afinfo` when possible, but byte-for-byte copy and SHA-256 equality remain the required preservation proof. An inspection failure does not discard the raw copy; it records an unsupported or invalid media classification and prevents source retirement until the raw evidence is complete.

### Phase 2: Copy and verify every source item

For each new WAV:

1. Copy it read-only to an app-owned part path on the destination filesystem.
2. Flush and `sync_all` the copy.
3. Re-read live source metadata.
4. Hash the source and destination copy independently.
5. Require equal byte counts and SHA-256 values.
6. Atomically finalize a visible destination WAV using the existing no-clobber rule.
7. Commit the verified WAV artifact to SQLite.

For each additional-source item:

1. Copy it to `source-extras/<date>/<transmitter>/<session>/` under a collision-safe relative name.
2. Flush, finalize, and hash it through the same no-clobber boundary.
3. Require source/destination byte and SHA-256 equality.
4. Commit separate additional-file evidence to SQLite.

No conversion starts until every new WAV and every additional-source item in the manifest is verified. Existing identical verified artifacts count toward the barrier only after their current destination size and SHA-256 are rechecked.

Capacity preflight reserves room for all missing WAV copies, all missing additional files, the maximum simultaneous M4A outputs, and the existing 10 GiB safety reserve.

### Phase 3: Convert the complete WAV cohort

When conversion is enabled, the cohort includes:

- every newly verified destination WAV from the current run; and
- every historical ledger-verified destination WAV that has no verified M4A successor.

Before conversion, re-hash each WAV against its ledger evidence. Invoke `/usr/bin/afconvert` directly without a shell:

```text
/usr/bin/afconvert INPUT -o OUTPUT -f m4af -d aac -b 128000 -q 127 -s 2
```

Each output uses an app-owned `.m4a.part-<uuid>` path. A successful process exit alone is insufficient. The app flushes the file and uses `/usr/bin/afinfo -x` to require:

- `m4af` container and AAC codec;
- source sample rate and source channel count;
- nonzero audio bytes and packet count;
- valid frames equal to the source PCM valid frames;
- declared total frames equal to valid, priming, and remainder frames;
- playable duration within one AAC packet of the source duration.

The app hashes the complete M4A, atomically finalizes it, synchronizes the parent directory, and commits the M4A artifact and its WAV provenance in SQLite.

A successfully converted item may remain durable for restart recovery, but no destination WAV is retired and no live source authority is issued until the entire cohort reaches `verified_m4a`.

If conversion is disabled, the run stops after the WAV-copy barrier with a successful `wav_backup_complete_source_retained` outcome. The UI explains that original retirement requires M4A conversion.

### Phase 4: Commit the M4A barrier and retire destination WAVs

After every cohort item is verified:

1. Re-read every final M4A's size, SHA-256, and audio description.
2. Commit the run-wide `m4a_cohort_verified` barrier in SQLite.
3. Flush a matching daily audit event with `sync_data`.
4. Move each superseded destination WAV to macOS Trash through Foundation.
5. Record each WAV cleanup outcome without weakening the M4A barrier.

A destination-WAV Trash failure leaves that WAV in place and logs a warning. It does not destroy or overwrite either artifact. Automatic live-source retirement remains withheld for that run because the Trash subsystem is not healthy; the user may retry after inspecting the log.

### Phase 5: Revalidate the complete live source

Immediately before manual proposal creation or automatic authorization:

1. Re-enumerate every recognized session and root recording without following symlinks.
2. Require the same complete set of WAVs and additional regular files captured by the run manifest.
3. Recheck byte count and modification time for each item.
4. Re-hash every live WAV and additional file.
5. Re-hash every final destination M4A or raw additional-file backup.
6. Require the ledger's WAV-to-M4A provenance and inspected audio properties.
7. Require current paired identity, mount generation, scan generation, destination generation, run-wide M4A barrier, healthy ledger, and flushed primary audit log.

Any mismatch invalidates all run retirement authority. Nothing on the transmitter moves.

### Phase 6: Move complete sources to Trash

With automatic Trash enabled, the app executes the authorized retirement immediately. Otherwise it creates the existing five-minute opaque manual proposal.

For a recognized session, the only target is the session directory itself. Foundation `FileManager.trashItem(at:resultingItemURL:)` chooses the correct removable-volume Trash. The app never calls `remove_file`, `remove_dir`, `rm`, Finder, AppleScript, or a direct `.Trashes` path for production source retirement.

Because additional regular files were backed up and are represented by the manifest, their presence no longer causes a generic refusal. An entry added after the manifest, a changed entry, a symlink, or a nested directory still refuses the whole session.

If an exact recognized session is already empty because a prior app version moved its files individually, it becomes eligible only when the ledger proves retired recordings under that exact directory, the paired device and mount generation still match, and a fresh canonical inventory proves the directory is empty. The empty directory is moved to Trash and logged.

## SQLite and Recovery Model

A forward-only migration adds:

- run-manifest phase and run-wide barrier state;
- immutable preference snapshot per run;
- additional-source file path, size, modification time, SHA-256, copied artifact path, size, SHA-256, and media classification;
- WAV-to-M4A provenance and encoder profile identifier `aac_lc_128k_v1`;
- destination-WAV Trash outcome;
- source revalidation outcome and retirement-authority timestamp;
- structured command-failure activity codes.

The migration preserves every existing source and artifact digest. Historical WAV records become pending members of the first enabled conversion cohort.

Restart recovery is monotonic:

- a verified WAV remains a valid resumable input;
- a valid finalized M4A with an app-owned recovery marker is re-inspected and re-hashed before ledger repair;
- a partially completed cohort resumes missing items but cannot infer the run-wide barrier;
- destination WAVs remain until the barrier is reconstructed and committed;
- live-source retirement always requires a newly mounted, freshly scanned, fully revalidated manifest.

## Durable Error and Diagnostic Logging

Every returned `CoreError`, adapter error, background-thread failure, Tauri command rejection, setting persistence failure, conversion tool failure, and Trash failure crosses one `FailureReporter` boundary.

The primary sink remains:

```text
<destination>/logs/YYYY/MM/YYMMDD-backup-mic.log
```

An error line contains only privacy-safe fields:

```text
2026-08-10T01:23:45.678+09:00 ERROR operation.failed operation="prepare_trash" stage="source_revalidation" tx=TX01 item="TX01_MIC001_...wav" error_code="session_contains_unverified_file" os_kind="invalid_data" retryable=true
```

The log excludes absolute source paths, volume UUIDs, full hashes, opaque proposal IDs, audio content, and raw command stderr. File names are newline-escaped and length-bounded.

If the destination log cannot be created or synchronized, the reporter writes the same safe event to:

```text
~/Library/Logs/com.channprj.DJIMicBackup/YYYY/MM/YYMMDD-backup-mic.log
```

The fallback sink is used for destination-selection errors, early startup failures, setting failures, and primary-log failures. A primary-log failure continues to prohibit source retirement. Failure reporting itself is best effort and must never replace the original error returned to the caller.

The UI maps known codes to exact Korean recovery guidance. Unknown codes still show a stable support code and `로그 열기`, rather than only “잠시 후 다시 시도해 주세요.” The settings window clears a stale error after the next successful persistence response.

## Setting Persistence and Concurrency

The three backup-preference commands become asynchronous adapters that perform blocking SQLite work off the Tauri UI thread. The backup coordinator no longer holds the ledger mutex across file hashing or Apple tool execution; it locks only for short transactional reads and commits.

For each setting save:

1. Validate the narrow command and acknowledgement.
2. Write the value in a short SQLite transaction.
3. Read it back from SQLite.
4. Update runtime state and publish a fresh snapshot only after equality is proven.
5. Append `setting.saved` or `setting.failed` through `FailureReporter`.
6. Return the persisted snapshot or a structured error.

The settings UI keeps optimistic feedback but displays `다음 백업부터 적용됩니다` while a run is active. Concurrent saves for different keys serialize at the persistence boundary and cannot overwrite each other's returned snapshot fields.

## User Interface Contract

The status sequence becomes:

```text
전체 WAV 복사 -> WAV 검증 -> 128kbps M4A 변환 -> 전체 M4A 검증 -> 원본 재검증 -> 휴지통 이동
```

The settings label becomes `WAV 백업 후 M4A로 변환`. Its description names AAC-LC 128kbps and explains that conversion happens only in the backup folder. When disabled, an inline note states that transmitter originals will remain.

Completion distinguishes:

- all M4As verified and source retained for manual approval;
- all M4As verified and source moved to Trash automatically;
- WAV backup complete but conversion disabled, source retained;
- conversion incomplete, source retained;
- complete-session revalidation refused because content changed;
- source or destination Trash movement failed.

Every failure card includes the safe stage, transmitter when relevant, retry guidance, and a log shortcut.

## Headatever Version and Release Order

No file may hand-write `VERSION`. After all implementation and full source checks pass:

1. Run `headatever init 0 --dry-run` and require the preview `0.260810.0`.
2. Synchronize `package.json`, both Cargo package versions, `src-tauri/tauri.conf.json`, the lockfile, and the visible settings version to `0.260810.0`.
3. Run the complete source check and commit/push that metadata checkpoint.
4. Run the bundled Headatever command `headatever init 0 --push`.
5. Require commit subject `chore(release): v0.260810.0` and annotated tag `v0.260810.0` at that commit.
6. Fetch and prove branch parity `0 0`, then prove the tag exists on the remote and points to the release commit.
7. Only then build the app and DMG, verify signature, metadata, architecture, version, and hashes, install the exact app with rollback, relaunch it, and prove installed/release executable parity.

If the release date changes before this phase, rerun the Headatever dry run and use its actual nondecreasing result everywhere instead of `0.260810.0`.

## Testing Strategy

### Core tests

- conversion never begins until all new WAV and additional-file copies are source-equal and durable;
- one failed copy prevents every conversion in that run;
- one failed M4A conversion prevents the run-wide barrier and every live-source retirement;
- 128 kbps profile identity is recorded and a different profile cannot satisfy the cohort;
- historical verified WAVs join the first enabled cohort;
- conversion-disabled runs retain live source and issue no retirement proposal;
- source M4A and AppleDouble files require independent raw-copy hash evidence;
- a newly added or changed session entry invalidates the complete manifest;
- successful whole-session Trash movement leaves no source directory;
- empty legacy session retirement requires exact ledger and identity evidence;
- restart recovery cannot synthesize a run-wide barrier from partial item success;
- setting saves are transactionally read back and concurrent keys cannot lose updates;
- every `CoreError` maps to a structured, privacy-safe diagnostic event.

### Integration tests

- generate several deterministic WAVs and prove no Apple conversion starts before the last WAV copy is verified;
- invoke production `afconvert` at 128 kbps and validate output with production `afinfo` parsing;
- convert a mixed cohort of new and historical WAV artifacts;
- inject failure at copy, conversion, inspection, finalization, ledger, audit, source revalidation, destination-WAV Trash, and source Trash boundaries;
- reproduce a session containing WAV, external M4A, and `._` sidecar, then prove all are backed before the whole session moves;
- run a setting save during a deliberately slow backup operation and prove it persists without freezing or returning a generic error;
- make the primary log unavailable and prove the fallback log contains the safe error while source retirement remains disabled;
- prove UI errors expose the stable error code and log action, then clear after a successful retry.

### Disposable FAT32 acceptance

The existing isolated `/Volumes/DJI-DELTEST` fixture is extended with:

- at least two WAVs;
- an independently generated M4A;
- an AppleDouble sidecar;
- a sentinel proving the disposable volume identity.

Acceptance must prove the complete copy barrier, 128 kbps M4A barrier, raw extra-file evidence, fresh revalidation, and Foundation movement of the entire session folder. The production policy never manipulates `.Trashes` directly.

### Installed-app acceptance

- deep strict code-sign verification passes;
- bundle ID, minimum macOS, arm64 architecture, and Headatever version are exact;
- installed and release executable SHA-256 values match;
- the app relaunches from `/Users/channprj/Applications/DJI Mic Backup.app` and exposes its status item;
- settings persist across the installed-app restart;
- the destination daily log records startup and scan activity;
- a controlled installed-app setting save during active work succeeds and records `setting.saved`;
- real transmitter validation is read-only until the run-wide M4A barrier is independently proven;
- automatic Trash remains off unless the user has already enabled and acknowledged it.

## Acceptance Criteria

1. Every new source WAV is a durable, equal-hash destination WAV before the first new conversion starts.
2. Every eligible new and historical WAV has a verified AAC-LC 128 kbps M4A successor when conversion is enabled.
3. One cohort failure preserves all live transmitter sources and prevents all retirement proposals.
4. Conversion-disabled runs preserve live transmitter sources even when WAV backup succeeds.
5. External session M4As and AppleDouble sidecars have independent equal-hash destination evidence before session retirement.
6. Source retirement rechecks the complete live inventory, live hashes, final artifacts, provenance, device generations, ledger, and audit durability.
7. Recognized sessions move as whole folders to macOS Trash; eligible empty legacy folders also move to Trash; production never permanently deletes them.
8. Every background and command failure is present in the primary or fallback daily log with a stable safe error code.
9. The reported setting-save scenario is covered by a concurrent-operation regression test and installed-app runtime proof.
10. The source, frontend, Rust, Clippy, typecheck, production build, FAT32 acceptance, packaging, signature, version, installation, runtime, and Git/tag parity gates all pass.
11. Headatever commit and annotated tag are published before the final package and local installation.
12. The final installed app is running, its executable matches the verified release, and the prior app bundle was moved to macOS Trash.
