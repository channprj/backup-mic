# DJI Mic Mini 2S Tauri/Rust Automatic Backup Design

**Date:** 2026-08-09
**Status:** Approved
**Target platform:** macOS 13 or newer
**Product name:** DJI Mic Backup
**Product shape:** Login-launched Tauri 2 tray application with a React/shadcn popover and a Rust-owned backup engine

## Problem

DJI Mic Mini 2S exposes each transmitter as a separate writable FAT32 USB volume when the charging case is connected to the Mac. Recordings should be copied automatically to the user's Documents folder, verified strongly, and surfaced through an unmistakable persistent status. Source recordings must never be deleted automatically. The user may delete only a transmitter's complete current WAV snapshot after every source and backup has been verified and the user has explicitly confirmed the action.

Both transmitters originally mounted with the generic name `NO NAME`, and macOS added a numeric suffix to one mount path. Their labels were later changed to `DJI-MIC-1` and `DJI-MIC-2` without changing the physical media. Volume labels and mount paths are therefore display information, never deletion authority.

## Observed Device Contract

The currently connected case provides concrete planning evidence:

- Two independent physical USB volumes are mounted.
- Both report device name `Mic Tx`, media name `Wireless Mic Tx Media`, FAT32, removable and writable media, and approximately 15.6 GB capacity.
- The volumes have stable, distinct UUIDs and may mount in either order or at different times.
- The plan-time inventory contains nine regular WAV files whose names begin with `TX01_` and two whose names begin with `TX02_`.
- An observed source directory is `TX_MIC001_20260809_013327`.
- An observed recording is `TX01_MIC009_20260809_021728_edit.wav`.
- The selected backup destination is `/Users/channprj/Documents/DJI-Mic-Mini-2S`.
- The destination filesystem had approximately 18 GiB free during design, so capacity preflight is mandatory.

The observed UUIDs are pairing inputs, not hardcoded product identifiers. Reformatting a transmitter may change its UUID and must revoke automatic trust until the user pairs it again.

## Goals

1. Start automatically after login and remain available through a macOS tray icon without a normal application window.
2. Detect either or both paired transmitters regardless of mount order, volume label, or mount-point suffix.
3. Automatically discover and back up stable WAV recordings to the confirmed destination.
4. Prove every reported backup using byte count and SHA-256 comparison.
5. Avoid duplicate copies across reconnects and recover safely from interruption.
6. Display current status, file and byte progress, per-transmitter progress, and recent activity in one compact popover.
7. Deliver macOS success, empty, partial-failure, error, deletion-success, and partial-deletion notifications.
8. Require an explicit user confirmation and a fresh Rust-side full-snapshot verification before deletion.
9. Keep recordings and operational metadata local with no analytics, networking, or telemetry.
10. Keep safety-critical behavior in a Tauri-independent Rust crate that can be tested without a WebView or physical microphone.

## Non-Goals

- Audio playback, waveform browsing, transcription, tagging, editing, or cloud upload.
- Renaming or modifying recordings on a transmitter before backup.
- Deleting hidden files, trash folders, unknown file types, symlinks, or source directories.
- Automatically ejecting the case.
- Supporting arbitrary recorders or generic drives with a similar name.
- A permanent normal window, general settings window, or long-term backup browser.
- Cross-platform support in the first version, even though the Rust core remains portable where practical.
- A privileged helper, daemon, App Store sandbox, remote frontend, or embedded web server.

## Settled Architecture

The application follows the rule: **Tauri is the shell; Rust is the product.**

```text
React + Vite + shadcn
  - renders trusted state
  - sends narrow user intents
             │
             │ typed Tauri commands and events
             ▼
Tauri application adapter
  - tray icon and popover window
  - autostart, notifications, folder dialog, Finder opening
  - no backup policy
             │
             │ Rust traits and state snapshots
             ▼
backup-core (pure Rust)
  - identity, scan, copy, hash, ledger, recovery, deletion
             │
             ▼
macOS platform adapter
  - Disk Arbitration callbacks
  - narrow Swift bridge only if direct Rust FFI fails runtime proof
```

### Workspace boundaries

```text
src/                              React application
src/components/                   shadcn-based presentation components
src/features/backup/              popover state and typed IPC client
src-tauri/src/                    Tauri application adapter
src-tauri/src/platform/macos/     macOS-only integration
src-tauri/crates/backup-core/     Tauri-independent safety domain
```

The frontend never imports a filesystem, shell, HTTP, database, or hashing implementation. The Tauri adapter never duplicates source eligibility, destination selection, deduplication, verification, or deletion rules.

### Rust backup core

`backup-core` owns:

- Paired-device identity and logical `TX01` / `TX02` labels.
- Mounted-device trust evaluation.
- DJI filename parsing and recording discovery.
- Two-observation stability checking.
- Destination layout and collision handling.
- Destination capacity preflight.
- Streaming source copy and SHA-256 verification.
- SQLite pairing, run, recording, verification, and deletion ledger.
- Duplicate recognition and recovery from interrupted runs.
- Complete-snapshot deletion eligibility.
- Time-limited deletion proposal issuance.
- Live re-verification and file unlink.
- Canonical state machine, progress values, typed errors, and activity events.

The core exposes traits for volumes, filesystems, clocks, ledgers, notifications, and event sinks. Production adapters implement them; tests use temporary directories and deterministic fakes.

### Tauri application adapter

The adapter owns only platform presentation and lifecycle:

- Create and update the tray icon.
- Toggle and position the hidden WebView popover on tray click.
- Keep the Rust process alive when the popover is hidden.
- Register and query login autostart.
- Request and deliver macOS notifications.
- Show the native destination-folder picker.
- Open the destination in Finder.
- Translate core snapshots into serializable IPC data.
- Translate narrow frontend intents into core operations.

Official Tauri plugins are used only for autostart, notification, dialog, positioner, and opener behavior. They are initialized from Rust. The frontend receives no general plugin permission that permits arbitrary filesystem, shell, URL, or network access.

### macOS device adapter

The first implementation calls Disk Arbitration directly from Rust. All Core Foundation pointers, callback contexts, retain/release balancing, and `unsafe` code are isolated under `src-tauri/src/platform/macos/disk_arbitration.rs` and exposed as a safe event stream.

The adapter listens for disk appearance, disappearance, and description changes. It emits a mounted volume only after UUID and mount URL are both available, covering the case in which an appearance callback arrives before mounting finishes.

A Swift bridge is added only if repeatable runtime evidence shows one of these failures:

- Direct FFI misses mount-completion or removal events.
- Session resume or repeated case reconnect produces unrecoverable callback loss.
- Tauri's macOS window APIs and official positioner cannot produce the accepted tray-popover behavior.

If needed, the bridge remains a narrow event or window-position adapter. It never owns copying, hashing, SQLite, eligibility, or deletion.

## Trust and IPC Contract

The WebView is untrusted presentation. It may display authority but never create it.

### Allowed frontend commands

- `get_app_snapshot()`
- `backup_now()`
- `choose_destination()`
- `pair_devices(assignments)`
- `prepare_deletion(transmitter)`
- `confirm_deletion(proposal_id)`
- `set_autostart(enabled)`
- `open_destination()`
- `quit_app()`

There is no general `read_file`, `write_file`, `delete_file`, `run_shell`, `open_url`, or `delete_paths` command.

### State delivery

Rust owns the canonical `AppSnapshot`. React calls `get_app_snapshot()` whenever the popover opens or regains focus and then listens for state-change events while visible. Events improve responsiveness; a later snapshot always repairs a missed event.

The snapshot includes:

- Overall phase and non-sensitive message.
- TX01 and TX02 mount, scan, copy, verify, and completion states.
- Completed and total file counts.
- Completed and total bytes.
- Overall percentage derived from monotonic copy-and-verification work units, with a file-count fallback for zero-byte totals.
- Current operation stage and a privacy-safe current item ordinal.
- Last successful backup time.
- Whether a complete deletion proposal can be prepared.
- Notification and autostart status.
- Up to 50 persisted recent activity entries, with the newest eight available to the popover.

Absolute source paths, source UUIDs, content hashes, SQLite IDs, and deletion record IDs do not enter routine UI snapshots.

### Deletion proposal

Deletion requires two independent Rust-side verification passes.

1. React sends the logical transmitter label only.
2. Rust resolves the current paired and mounted device.
3. Rust scans all current WAV files and verifies that the entire set has valid destination records and matching destination hashes.
4. Rust returns a display-only proposal with an opaque random ID, transmitter label, file count, byte total, and destination summary.
5. React displays the shadcn confirmation dialog.
6. On confirmation, React returns only the proposal ID.
7. Rust resolves the proposal from private memory and repeats device, mount-generation, path, source metadata, source hash, destination existence, destination path, and destination hash checks.
8. Rust begins unlinking only after the complete preflight succeeds.

A proposal expires after five minutes and is immediately invalidated by device disappearance, mount-generation change, new scan results, backup start, destination change, or another deletion attempt. The frontend cannot extend its lifetime or alter its record set.

## Device Trust

Pairing records:

- Volume UUID.
- Logical transmitter label.
- USB protocol.
- External and removable status.
- Writable status.
- Media name.
- Nominal capacity.
- Pairing time.

A volume label, BSD disk number, or mount path is never sufficient. Runtime matching requires the paired UUID and the expected physical-media properties. Capacity may vary only within a small formatting tolerance.

If a matching UUID presents unexpected physical properties, the app reports an identity mismatch and performs no scan, backup, or delete operation. If reformatting changes the UUID, the app shows an unpaired DJI-shaped device and requires explicit re-pairing.

## Recording Discovery

The scanner enumerates regular, non-symlink files with a case-insensitive `.wav` extension inside visible directories. It ignores `.Spotlight-V100`, `.Trashes`, `.fseventsd`, other hidden entries, directories, symlinks, and unknown extensions.

Every candidate URL is canonicalized and required to remain inside the live mounted-volume root. A candidate becomes stable only after relative path, byte count, and modification time remain identical across two scans separated by two seconds.

The filename parser recognizes the anchored form:

```text
TXdd_MICdigits_yyyyMMdd_HHmmss[_suffix].wav
```

The parsed transmitter prefix must agree with the paired logical label. A mismatch is reported and blocks a complete deletion snapshot. A valid encoded date determines the destination date without timezone conversion. Malformed names fall back to the source modification date in the current calendar and are noted in the ledger.

## Destination and Copy Contract

The user-visible layout is:

```text
DJI-Mic-Mini-2S/
  YYYY/
    YYYY-MM-DD/
      TX01/
        original-filename.wav
      TX02/
        original-filename.wav
```

Before a run, Rust sums bytes that actually need copying and queries available destination capacity. The run proceeds only when available bytes are at least required bytes plus a 10 GiB reserve.

For every new recording, Rust:

1. Captures regular-file metadata.
2. Opens the source read-only.
3. Exclusively creates a UUID-named hidden `.partial` file in the final destination directory.
4. Streams fixed-size chunks while calculating source SHA-256 and updating byte progress.
5. Flushes the buffered writer and calls `sync_all` on the temporary destination.
6. Re-reads source metadata and refuses a changed source.
7. Re-reads the temporary destination and independently computes destination SHA-256.
8. Requires equal byte counts and hashes.
9. Renames the temporary file to its final path on the same destination filesystem.
10. Commits the verified entry to SQLite.

An identical existing destination is reused after hash verification. A same-name file with different content is never overwritten; the new name receives an eight-character SHA-256 suffix. Interrupted `.partial` files are recognized by app-owned UUID naming and reconciled conservatively on the next launch.

## SQLite Ledger

The ledger resides in the Tauri application-data directory and uses `rusqlite` transactions. It records:

- Paired devices.
- Backup runs and terminal outcomes.
- Source relative path, size, modification time, and content hash.
- Destination relative path and verification time.
- Deletion eligibility inputs.
- Successful unlink time or unlink failure.
- Bounded recent activity.

The ledger is an index, not sole deletion evidence. A source is never deletable merely because a row says it is verified. Live source and destination hashes remain mandatory.

If the database is corrupt, the app moves the database and its side files into a timestamped quarantine directory, creates a fresh ledger, disables deletion, and reports that destination re-indexing is required. It never reconstructs deletion permission from filenames alone.

## State Model

The core state machine is:

```text
Idle
  -> Detecting
  -> Scanning
  -> CheckingCapacity
  -> Copying
  -> Verifying
  -> CompletedDeletionPending
     | NothingNew
     | PartialFailure
     | Error
```

Each transmitter has an independent sub-state. One transmitter's failure does not cancel the other's already safe work. Overall deletion readiness is presented per transmitter, but only a complete current snapshot can receive a proposal.

Deletion is a separate state machine:

```text
Preparing
  -> AwaitingConfirmation
  -> Revalidating
  -> Deleting
  -> Deleted
     | Refused
     | PartiallyDeleted
```

Multi-file unlink is not atomic on FAT32. If a source unlink fails after earlier files were removed, Rust stops immediately, records every successful removal and the first failure, preserves remaining sources, and reports a prominent partial-deletion outcome. Every source already removed still has a verified backup.

Progress never moves backward or resets between copy and verification. Each byte that needs a new copy contributes one copy work unit and one destination-verification work unit. A reused destination contributes one verification work unit. The progress percentage is completed work units divided by total work units, while the adjacent byte label explicitly reports copied bytes over bytes requiring a new copy. A file increments the completed-file count only after destination verification and ledger commit.

## Approved Popover UX

The accepted popover combines the activity-timeline direction with a focused current-state summary.

### Window behavior

- A single frameless WebView window is normally hidden.
- Left-clicking the tray icon toggles it directly under the tray using constrained screen positioning.
- Clicking elsewhere hides the popover unless setup or destructive confirmation is active.
- The window uses a compact dark system-adjacent surface exactly 380 px wide, grows with content up to 640 px high, and scrolls only the activity region when needed.
- The app remains alive when the window is hidden and has no Dock presence during ordinary operation.

### Information hierarchy

1. App identity and connection indicator.
2. Current-state summary card.
3. Overall progress and per-transmitter progress when active.
4. Recent activity timeline.
5. Contextual primary action.
6. Backup-folder, notification, and autostart utilities.

### Backup progress state

The summary card shows:

- `백업 중`.
- A reminder to keep the case connected and that originals are unchanged.
- Percentage.
- Completed files over total files.
- Completed bytes over total bytes.
- An accessible progress bar.
- TX01 and TX02 progress pills.
- Current phase: copy or SHA-256 verification.

Deletion controls are absent or disabled while any relevant operation is active.

### Completed state

The same card changes to:

- `백업 완료`.
- Total verified file count and bytes.
- TX01 and TX02 verified summaries.
- A recent activity timeline showing verification completion and device detection.
- `검증된 원본 삭제 검토…` only for a transmitter with a complete snapshot.
- `백업 폴더 열기` and `다시 확인` secondary actions.

### Other states

- `대기 중`: no trusted transmitter mounted.
- `마이크 확인 중`: device appeared and identity/scan is underway.
- `새 녹음 없음`: current files already have intact verified destinations.
- `일부 파일 백업 실패`: exact completed and failed counts, deletion unavailable for the affected transmitter.
- `백업 오류`: actionable non-sensitive cause and retry action.
- `일부 원본 삭제 실패`: exact deleted and remaining counts, never a success icon.

The progress bar and icons always have text and accessibility labels; color is never the sole status channel. Motion respects reduced-motion preferences.

## Notifications

Rust requests and sends notifications through the Tauri notification plugin. Notifications are secondary to persistent popover state.

- Backup success: transmitter labels, verified file count, bytes, and deletion-pending status.
- Nothing new: devices were checked successfully.
- Partial failure or error: source files affected by uncertainty were left untouched.
- Deletion complete: exact count and bytes removed.
- Partial deletion: exact deleted and remaining counts.

If permission is denied, the app records the state and shows `알림 꺼짐` without repeatedly prompting. Dismissing a notification never loses status or authorizes deletion.

## Error Handling and Recovery

- **Device disappears during scan:** cancel the scan and leave sources untouched.
- **Device disappears during copy:** stop the worker, clean only the app-owned temporary destination, and preserve the source.
- **Destination unavailable or too full:** fail before copying and deletion eligibility.
- **Source changes:** discard the temporary destination and defer the source.
- **Hash mismatch:** discard or quarantine the temporary destination, record failure, and keep the source.
- **App exits or crashes:** mark running ledger entries interrupted and reconcile app-owned temporary or finalized files on the next launch.
- **Repeated mount events:** coalesce by paired UUID and mount generation.
- **Existing verified source remains:** re-check its destination and avoid a duplicate copy.
- **Unknown files beside recordings:** leave untouched; they do not become deletion candidates.
- **Unknown or changed device identity:** show pairing or mismatch state and perform no destructive operation.
- **Ledger corruption:** quarantine, replace, and revoke deletion.
- **Frontend reload or event loss:** call `get_app_snapshot` and recover presentation from Rust.
- **WebView crash:** the Rust process retains canonical state; no filesystem action depends on UI memory.

## Privacy and Security

- No audio, filenames, paths, UUIDs, hashes, logs, or activity data leave the Mac.
- No HTTP client or remote WebView content is included.
- A strict Content Security Policy permits only the bundled frontend.
- Tauri capabilities are attached only to the single local popover window.
- No frontend filesystem, shell, network, or arbitrary opener capability exists.
- Custom command inputs are treated as untrusted and validated in Rust.
- Paths are canonicalized at every trust boundary.
- Logs expose static event codes and aggregate counts; filenames, paths, UUIDs, and hashes are redacted.
- The app is not App Store sandboxed in v1 because automatic access to paired removable volumes is core behavior. It remains an unprivileged per-user process with no helper daemon.

## Testing Strategy

### Rust unit tests

- Device matcher accepts paired devices after label changes and rejects UUID-only, name-only, internal, non-removable, read-only, non-USB, media-name-mismatched, and capacity-mismatched devices.
- DJI parser handles observed TX01/TX02 names, malformed dates, suffixes, and label conflicts.
- Canonical path rules reject traversal, symlinks, and destination-root escape.
- State transitions never reach deletion-ready before complete verification.
- Proposal invalidation fires for every device, scan, destination, time, and operation change.
- Error mapping never exposes sensitive strings in UI DTOs.

### Property tests

- Arbitrary relative-path inputs cannot escape the mounted source or destination roots.
- Arbitrary operation sequences cannot invoke unlink without a current valid proposal and completed live preflight.
- Serialized snapshots never include forbidden sensitive fields.

### Rust integration tests

- Empty paired transmitter returns `NothingNew`.
- Two transmitters mounting in either order are processed independently.
- Stable WAVs reach deterministic destinations with equal source and destination hashes.
- Changing files are deferred.
- Destination-full, permission-denied, disconnect, sync, write, and hash failures preserve sources.
- Crashes before and after final rename reconcile without duplicates.
- Same-content destinations are reused; different-content collisions preserve both.
- Complete-snapshot deletion refuses any changed source or destination before unlink.
- Injected unlink failure records and reports an exact partial deletion.
- Corrupt SQLite disables deletion and preserves evidence.

### Frontend tests

- Every Rust phase maps to the approved Korean title, icon, action state, and accessibility text.
- Progress shows percentage, files, bytes, current stage, and TX01/TX02 values.
- Timeline ordering and bounded history render correctly.
- Destructive confirmation sends only the opaque proposal ID.
- Partial failures never render a success icon or active delete button.
- Event loss is repaired by a later snapshot.
- Reduced motion and keyboard navigation work.

### IPC contract tests

Rust produces committed JSON fixtures for snapshots, progress, proposals, outcomes, and errors. Frontend tests validate those fixtures with runtime schemas. CI fails when Rust serialization or frontend expectations drift.

### macOS runtime tests

- Direct Disk Arbitration FFI detects appearance, mount completion, rename, disappearance, reconnect, and independent TX01/TX02 timing.
- Tray clicking opens the accepted popover on the active screen and hides it correctly.
- Autostart survives logout/login and reflects system state.
- Notifications appear when granted and degrade to popover-only status when denied.
- The installed signed bundle matches the tested artifact.

### Safe hardware acceptance

- Inventory all current production recordings read-only immediately before acceptance.
- Back up every inventoried production file without approving deletion.
- Independently compare every source and destination SHA-256.
- Reconnect and prove no duplicate files are created.
- Confirm progress, summary, timeline, and notification outcomes.
- Exercise destructive behavior only in an isolated temporary FAT32 fixture.
- Never run physical deletion acceptance while any production recording remains on a transmitter.

## Acceptance Criteria

- Connecting the paired case triggers backup without opening a terminal or normal app window.
- Both transmitters are handled despite mutable labels, mount paths, and event order.
- Every file reported successful has a durable destination with equal byte count and SHA-256.
- The popover shows overall and per-transmitter progress during backup and a verified summary afterward.
- The recent activity timeline preserves a clear explanation of how the current state was reached.
- Reconnecting unchanged media creates no duplicate destination files.
- React cannot read, write, copy, hash, or delete files except through the narrow Rust commands.
- No deletion begins without an unexpired Rust proposal, explicit confirmation, and complete live re-verification.
- Identity ambiguity, insufficient space, interruption, permission failure, content change, hash mismatch, or ledger damage fails closed.
- Existing production recordings are never used as deletion-test fixtures.
- The installed app starts at login after system approval and communicates success or failure through both popover state and optional notifications.

## Deferred Follow-Up Work

- Destination re-index UI after ledger loss.
- Audio preview and searchable backup history.
- A second backup destination or retention policy.
- Developer ID signing, notarization, and external distribution.
- Automatic safe eject.
- Cross-platform device adapters.

## Official References

- [Tauri System Tray](https://v2.tauri.app/learn/system-tray/)
- [Tauri Window Customization](https://v2.tauri.app/learn/window-customization/)
- [Tauri Positioner](https://v2.tauri.app/reference/javascript/positioner/)
- [Tauri Autostart](https://v2.tauri.app/plugin/autostart/)
- [Tauri Notification](https://v2.tauri.app/reference/javascript/notification/)
- [Tauri Dialog](https://v2.tauri.app/ko/plugin/dialog/)
- [Tauri Opener](https://v2.tauri.app/reference/javascript/opener/)
- [Tauri Security](https://v2.tauri.app/security/)
- [Tauri Command Scopes](https://v2.tauri.app/security/scope/)
- [Apple Disk Arbitration Appeared Callback](https://developer.apple.com/documentation/diskarbitration/1492707-daregisterdiskappearedcallback)
- [Apple Disk Arbitration Description Constants](https://developer.apple.com/documentation/diskarbitration/diskarbitration-constants)
