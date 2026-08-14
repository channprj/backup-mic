# Transient Recorder Reconnect Design

**Date:** 2026-08-14
**Status:** Approved for implementation
**Target platform:** macOS 13 or newer
**Product:** Backup Mic
**Relationship to the existing design:** This document extends the approved calendar archive layout and the configurable recorder rule engine. Stable scanning, copy-before-conversion, SHA-256 verification, no-clobber finalization, SQLite evidence, and recoverable macOS Trash behavior remain unchanged.

## Problem

The current `backup_now` command treats zero matched recorders as `CoreError::DeviceRemoved`. The command returns an error, writes `operation.failed`, and the UI shows a failure even when the recorder is intentionally or momentarily disconnected.

The device orchestrator can back up after a later mount only when automatic backup is enabled. A manual request made while no recorder is mounted is discarded, and a manual run interrupted by a transient disconnect has no durable in-process retry intent.

## Approved Behavior

1. `지금 백업` with no matched recorder is a valid request.
2. The request enters a visible waiting state instead of returning `device_removed`.
3. The one-shot request remains pending until one of these terminal events occurs:
   - a matching recorder reconnects and the backup operation starts successfully;
   - the user selects `백업 취소`;
   - the app exits.
4. The request is honored after reconnection even when automatic backup is disabled.
5. A recorder disconnect during a manual run preserves or restores the same one-shot waiting intent. Safe, already-verified work may remain committed; unverified work never authorizes source retirement.
6. Repeated mount events or multiple UI clicks cannot start duplicate concurrent runs.
7. Application restart does not restore the manual request. After restart, only the persisted automatic-backup preference may schedule work.

## State Model

`AppState` owns an atomic, process-local manual backup intent shared by commands and the device orchestrator. It is independent of `operation_active`:

- **clear + no active operation:** normal idle behavior;
- **set + no matched recorder:** waiting for a recorder;
- **set + matched recorder:** eligible for a one-shot start;
- **set + active operation:** the manual request is being serviced;
- **clear + active operation:** an automatic run is in progress.

The intent is cleared only after a backup start is accepted, explicit cancellation, or process exit. If the accepted manual run subsequently loses recorder authority, the intent is restored before returning to the waiting state.

Operation reservation and the existing `Busy` result remain the concurrency authority. A `Busy` start attempt retains the pending intent for a later retry.

## Command and Lifecycle Flow

### Manual request while disconnected

1. Validate setup and destination independently from recorder presence.
2. Set the manual intent.
3. If no matching recorder is mounted, publish the waiting snapshot and return command success.
4. Do not call the failure reporter and do not create an `operation.failed` event.

### Reconnection

1. Validate and match the mounted volume using the existing UUID, device constraint, and rule checks.
2. Schedule a run when either automatic backup is enabled or a manual intent is pending.
3. Attempt the run through the existing operation guard.
4. Clear the manual intent only when the run is accepted.
5. Log the transition as an informational resume event.

### Disconnect during a run

1. The lifecycle handler immediately invalidates the mounted authority as it does today.
2. Source I/O errors are classified as `device_removed` when the frozen source authority is no longer current.
3. A manual run that observes this condition restores the manual intent.
4. Work already verified in the ledger stays reusable; partial or unverified output remains ineligible for source retirement.
5. Once the active operation settles, the UI returns to waiting instead of presenting an expected-disconnect failure.
6. Reconnection starts a fresh stable scan and safely resumes through ledger-backed idempotency.

If another connected recorder can complete while one recorder disappears, its verified result remains committed. The missing recorder is retried after reconnection; no completed artifact is recopied unnecessarily.

## Cancellation

`백업 취소` clears both the pending manual intent and any active operation token. When no operation is active, cancellation exits the waiting state immediately. When a run is active, the existing cancellation checks settle the snapshot only after in-flight safe boundaries are reached.

After cancellation, later reconnection follows only the automatic-backup preference. It must not resurrect the cancelled manual request.

## UI Copy and Interaction

The waiting state is rendered as an active, cancellable state:

- title: `녹음기 연결을 기다리는 중`
- detail: `녹음기를 연결하면 백업을 자동으로 시작합니다.`
- action: `백업 취소`

The command resolves successfully, so the inline failure alert is not shown. The disconnected waiting state does not use the phrase `원본은 변경되지 않았습니다`. On reconnection the existing detailed stages take over: recorder detection, file discovery, capacity check, copy, verification, conversion, revalidation, and Trash movement.

Real failures retain their structured error UI and diagnostic code. The phrase changes in this design do not weaken fail-closed source handling.

## Logging

Expected removable-device lifecycle events are not operation failures:

- `device.removed` remains `WARN`;
- a queued manual request writes `backup.waiting_for_device` at `INFO`;
- a reconnect-triggered manual start writes `backup.resumed_after_reconnect` at `INFO`;
- normal completion writes the existing `backup.run_complete` event.

No `operation.failed operation="backup_now" error_code="device_removed"` event is emitted merely because no recorder is mounted. Actual destination, permission, conversion, hash, ledger, or Trash failures remain `ERROR` and keep their current structured failure reports.

## Calendar Archive Integration

The reconnect work ships with the already approved DJI archive layout:

```text
<selected destination>/YYYY/MM/YYMMDD-T01_or_T02_<remaining source name>.m4a
```

DJI recording artifacts do not receive a TX01/TX02 directory, a day directory, or an extra DJI archive directory. Other configurable recorder rules retain their explicitly configured archive directory so unrelated rule outputs cannot collide.

Existing verified DJI artifacts are migrated with ledger and SHA-256 proof: copy to the new path, synchronize, verify size and hash, move the previous artifact to macOS Trash, update the ledger, and prune only empty legacy directories. Equal-content collisions are reused; divergent collisions receive a short SHA-256 suffix. Every step is idempotent and no old artifact is permanently deleted.

## Verification Requirements

Automated tests must prove:

1. Manual backup without a recorder returns success and publishes the waiting state.
2. No failure report is written for the expected disconnected wait.
3. Reconnection consumes a pending manual request even with automatic backup disabled.
4. A busy start retains the pending request.
5. Repeated mounts cannot create duplicate runs.
6. Explicit cancellation clears waiting intent and prevents a later manual resume.
7. A transient disconnect during a manual run restores waiting intent and preserves verified work.
8. A real non-device error remains a structured failure.
9. Frontend copy and controls render waiting, resume, and cancellation correctly.
10. DJI archive planning and verified legacy migration satisfy the separate approved calendar-layout test matrix.
11. Repository-wide Rust tests, Clippy, frontend tests, type checking, and production build pass.

Installed-app verification must prove the bumped version, packaged/installed executable hash parity, successful launch, waiting UI behavior without a recorder, and reconnect scheduling to the extent a matching physical recorder is available. Any unavailable physical-device or macOS UI automation evidence is reported explicitly rather than inferred.

## Release and Git Contract

The design, implementation, tests, and release metadata are published as logical Conventional Commit checkpoints through ordinary immediate pushes. Headatever increments the version only after development and verification are complete. The new bundle replaces the local installed app only after metadata, signature, and checksums pass; the previous app bundle is moved to macOS Trash rather than permanently deleted.
