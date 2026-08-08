# DJI Mic Mini 2S Automatic Backup Design

**Date:** 2026-08-09
**Status:** Superseded by `2026-08-09-dji-mic-mini-tauri-rust-backup-design.md`
**Target platform:** macOS
**Working product shape:** Historical Swift-native direction retained for comparison; do not implement

## Problem

DJI Mic Mini 2S exposes each transmitter as a separate writable FAT32 USB volume when the charging case is connected to the Mac. Recordings should be copied automatically to the user's Documents folder, verified strongly, and surfaced through an unmistakable success or failure status. Source recordings must never be deleted automatically. The user may delete only the exact recordings whose backups have been verified, through an explicit menu bar action and confirmation.

The central safety problem is that both transmitter volumes use the generic volume name `NO NAME`. Volume name or mount path alone therefore cannot authorize backup or deletion.

## Observed Device Contract

The design is grounded in the currently connected hardware:

- The case presents two independent physical USB volumes. They initially mounted as `/Volumes/NO NAME` and `/Volumes/NO NAME 1` and now mount as `/Volumes/DJI-MIC-1` and `/Volumes/DJI-MIC-2`, demonstrating that volume labels and mount paths are mutable.
- Both report device name `Mic Tx`, media name `Wireless Mic Tx Media`, FAT32, removable and writable media, and approximately 15.6 GB capacity.
- The volumes have distinct UUIDs and may mount in either order or at different times.
- The first transmitter currently contains nine regular WAV files under a directory such as `TX_MIC001_20260809_013327`, with names such as `TX01_MIC001_20260809_013327_edit.wav`.
- The second transmitter currently contains two regular WAV files whose names begin with `TX02_`.
- The destination is the runtime path `/Users/channprj/Documents/DJI-Mic-Mini-2S`.
- The destination filesystem currently has about 18 GiB free, so capacity preflight is mandatory before every copy batch.

The observed UUIDs are pairing inputs, not universal DJI identifiers. Reformatting a transmitter may change its UUID and must invalidate automatic trust until the user pairs it again.

## Goals

1. Start automatically when the user logs in and remain available as a menu bar item without a normal window.
2. Detect either or both paired transmitter volumes regardless of mount order or macOS mount-point suffix.
3. Automatically discover and back up stable WAV recordings to the configured destination.
4. Prove each backup using byte count and SHA-256 comparison before marking it verified.
5. Avoid duplicate copies across reconnects and recover safely from interruption.
6. Tell the user clearly whether a run succeeded, partially failed, or failed, including file count and bytes.
7. Require explicit user confirmation before deleting verified source recordings.
8. Never delete an unknown, changed, unverified, or currently copying file.
9. Keep recordings and operational metadata local to the Mac with no network upload or telemetry.

## Non-Goals

- Audio playback, waveform browsing, transcription, tagging, editing, or cloud upload.
- Renaming or modifying recordings on the transmitter before backup.
- Deleting transmitter system files, hidden files, unknown file types, or empty folders.
- Automatically ejecting the case after backup or deletion.
- Supporting arbitrary cameras, recorders, or generic volumes named `NO NAME`.
- A general-purpose settings window or full backup-browser UI in the first version.

## User Experience

### Menu bar states

The menu bar item has a stable icon plus a state label in its menu:

- **Idle:** No paired transmitter is mounted.
- **Device detected:** At least one paired transmitter mounted and is being scanned.
- **Backing up:** Copy and verification are in progress; deletion controls are disabled.
- **Verified, deletion pending:** All newly discovered files for the completed run are backed up and verified.
- **Partial failure:** At least one file succeeded and at least one could not be backed up or verified. Deletion is disabled for that transmitter until every current WAV file is verified.
- **Error:** The run could not proceed, for example because of insufficient destination space, lost device access, or an unrecognized volume.

The menu shows the most recent run time, per-transmitter counts, total bytes, destination shortcut, and a compact error explanation when relevant. It also provides `Back Up Now`, `Delete Verified Originals…`, `Open Backup Folder`, `Launch at Login`, and `Quit` actions.

### Notifications

- Successful backup: identify the transmitter or transmitters, file count, total size, and that verified originals are awaiting optional deletion.
- Nothing new: say that the devices were checked and no new recordings required copying.
- Partial failure or error: say that source recordings were left untouched where safety could not be proved and direct the user to the menu for detail.
- Deletion complete: report the exact number and size of source files removed, plus any files skipped because they changed.

Notifications complement the persistent menu state; dismissal of a notification never loses the status or grants deletion authority.

### First run and pairing

On first launch, the app finds the two currently connected `Mic Tx` volumes and presents a single pairing sheet. It shows each volume's UUID suffix, used capacity, and any transmitter label inferred from recording names. The user confirms the pair and assigns stable logical labels `TX01` and `TX02` when inference is unavailable or conflicting.

Pairing records a compound identity: volume UUID, physical/removable USB status, media name, nominal capacity, and logical transmitter label. The current friendly volume label may be shown to the user but is not authoritative; a volume name or mount path is never sufficient. If a known device is reformatted and its UUID changes, the app reports `Unrecognized Mic Tx` and requires re-pairing before it can back up or delete anything on that volume.

## Architecture

The first version is one native Swift macOS application. A second privileged daemon is unnecessary because all source and destination paths are user-accessible and the app runs within the logged-in GUI session.

### App lifecycle and menu presentation

Owns login-item registration, the menu bar item, menu actions, confirmation sheets, and state rendering. It does not contain backup logic; it observes the backup coordinator's state.

### Device monitor and registry

Uses the macOS disk-arbitration layer to observe volume appearance, disappearance, and mount completion without polling `/Volumes`. It resolves current device properties for every event and compares them with paired device records.

It emits only trusted logical devices to the backup coordinator. Unknown or identity-mismatched devices may be displayed for pairing but cannot reach backup or deletion operations.

### Recording scanner

Enumerates regular, non-symlink WAV files inside non-hidden recording directories. It ignores macOS metadata, hidden folders, trash folders, directories, symlinks, and all unknown file extensions.

A candidate is considered stable only after its path, byte count, and modification time remain unchanged across two observations separated by a short settling interval. Files that disappear or change remain untouched and are retried on a later scan.

### Backup coordinator

Serializes work per transmitter while allowing the two independent transmitters to be tracked as separate jobs. A second mount event cannot start a duplicate job for the same logical device.

Before copying, it calculates the bytes required for recordings that are not already verified and requires those bytes plus a 10 GiB destination reserve. If the preflight fails, no files are copied or deleted and the user receives an actionable space warning.

For each candidate, the coordinator:

1. Chooses a deterministic destination from recording date and logical transmitter.
2. Copies to a hidden temporary filename in the final destination directory.
3. Flushes and closes the temporary destination.
4. Computes SHA-256 for the source and temporary destination and compares the hashes and byte counts.
5. Atomically renames the temporary file to its final name.
6. Commits a verified record to the local ledger.

Interruption before step 5 leaves only a recognizable temporary file. A later run may remove its own stale temporary file after proving that it is not an active copy. Interruption after step 5 but before step 6 is repaired by checking the destination file and recording it rather than copying over it.

### Backup ledger

A local SQLite ledger under the app's Application Support directory records paired devices, source fingerprints, destination paths, content hashes, run status, and deletion eligibility. Transactions keep the final destination and ledger state reconcilable across crashes.

The ledger is an index, not the only evidence. A source is eligible for deletion only when its destination file still exists and re-verifies against the stored content hash. If the ledger is missing or corrupt, all deletion eligibility is revoked until the destination is re-indexed.

### Deletion coordinator

Deletion is a distinct operation that never runs as a continuation of automatic backup.

Selecting `Delete Verified Originals…` opens a confirmation sheet listing transmitter, file count, total bytes, and backup destination. The action is available for a transmitter only when every current WAV file discovered on it belongs to one complete verified snapshot. A partial backup never enables deletion for that transmitter. The two transmitters remain independent, so one fully successful transmitter may be approved even if the other transmitter failed.

On confirmation, every candidate in that complete snapshot must pass all of these checks again:

- The mounted device still matches the paired identity and logical transmitter.
- The source is the same regular WAV file recorded by the verified backup entry.
- The source path remains within the trusted mounted-volume root after resolving filesystem paths.
- Source byte count, modification time, and SHA-256 still match the verified entry.
- The destination file exists and its SHA-256 still matches the verified entry.
- No backup job is active for that device.

The coordinator performs the checks for the whole snapshot before unlinking any file. If any candidate fails, it aborts deletion for that transmitter and removes nothing. Only after the complete preflight succeeds are the snapshot's WAV files unlinked individually and recorded in the ledger. Deletion never recursively removes directories and never touches hidden or unknown files.

## Destination Layout and Collision Rules

The user-visible layout is:

```text
DJI-Mic-Mini-2S/
  YYYY/
    YYYY-MM-DD/
      TX01/
        <original-filename>.wav
      TX02/
        <original-filename>.wav
```

The recording timestamp encoded in a valid DJI filename determines the date. If the name cannot be parsed, the file modification date is used and the ledger records that fallback.

An existing final file with the same content hash is treated as already backed up. The ledger is repaired if necessary. An existing filename with different content is never overwritten; the new file receives a short content-hash suffix and the collision is surfaced in run details.

## State and Recovery Rules

- **Device disconnect during scan:** cancel the scan and leave all files untouched.
- **Disconnect during copy:** cancel safely, retain or clean only the app-owned temporary destination, and leave source files untouched.
- **Destination full or unwritable:** fail before deletion eligibility is granted.
- **Hash mismatch:** discard or quarantine the temporary destination, mark the source failed, and keep it on the transmitter.
- **App crash:** reconstruct incomplete runs from the ledger and temporary-file convention at next launch.
- **Repeated mount notification:** coalesce events by logical device and current job identity.
- **Already-backed-up source still present:** verify the existing destination, mark it deletion-eligible, and do not create another copy.
- **Mixed results:** preserve successful verified backups and report them, but grant no deletion eligibility to that transmitter until a later run verifies every current WAV file.
- **Unknown files beside recordings:** leave them untouched and mention their presence only when helpful; they do not make verified WAV backups fail.
- **Two transmitters mount or unmount independently:** one device's failure does not cancel the other's safe work, and the combined UI retains per-device outcomes.

## Privacy and Security

- No audio, filenames, hashes, logs, or usage data leave the Mac.
- Operational logs exclude audio content and use paths only where required for local diagnosis.
- Filesystem paths are canonicalized before trust-boundary checks to prevent symlink or traversal mistakes.
- Unknown devices are read only far enough to display pairing information; no copy or delete action is authorized.
- The app requests only macOS permissions actually needed for the selected Documents destination and notification delivery.
- No privileged helper or administrator access is required.

## Testing Strategy

### Unit tests

- Device identity matching accepts the two paired devices and rejects name-only, UUID-only, internal, non-removable, and media-name-mismatched devices.
- DJI filename parsing maps known names to the correct transmitter and date and handles malformed names safely.
- Destination-path creation prevents traversal and resolves collision names deterministically.
- Backup state transitions prohibit deletion eligibility before copy finalization, hash verification, and ledger commit.
- Deletion eligibility is revoked by any source or destination identity, metadata, or hash change.
- Capacity preflight includes only uncopied bytes and preserves the configured reserve.

### Integration tests with disk fixtures

- Empty paired transmitter produces a successful `nothing new` result.
- Two transmitters mounting in either order are identified and processed independently.
- A stable WAV file reaches the deterministic destination with identical bytes and a verified ledger entry.
- A file changing during the settling interval is deferred.
- Disconnect, copy failure, destination-full, permission-denied, and hash-mismatch simulations leave source files intact.
- Crash-recovery simulations reconcile temporary files and final files without duplicate copies.
- Already-backed-up sources become deletion-eligible without recopying.
- Filename collisions with different content preserve both files without overwrite.
- Manual deletion requires a fully verified transmitter snapshot, aborts before unlinking if any file changed, and then removes only the exact snapshot's WAV files.

### Hardware acceptance tests

- Connect the real case with both transmitters and confirm both volumes are detected even when macOS assigns different mount-point suffixes.
- Back up a disposable sample recording from each transmitter and compare source and destination hashes independently.
- Disconnect during a disposable large-file copy and confirm the source remains present and the next connection recovers.
- Change a disposable source after backup and confirm deletion refuses it.
- Prove deletion against an isolated temporary FAT32 fixture containing only disposable WAV files. Exercise physical deletion only if a separate, user-approved transmitter state contains no production recordings.
- Restart and log out/in to confirm login launch, persisted pairing, notifications, and ledger recovery.

The existing eleven recordings are not deletion-test fixtures. Physical deletion testing is skipped while any production recording remains on a transmitter.

## Acceptance Criteria

- Connecting the paired case triggers backup without opening the app or a terminal.
- Both transmitters are handled despite identical volume names and variable mount paths.
- Every file reported as backed up has a durable destination whose byte count and SHA-256 match the source.
- A transmitter whose complete current WAV snapshot is verified is clearly shown as awaiting optional deletion.
- No source file is ever deleted without a complete verified transmitter snapshot, the menu action, confirmation, and immediate source-plus-destination re-verification.
- Any ambiguity, insufficient space, interruption, permission problem, or content mismatch fails closed and leaves the relevant source file intact.
- Reconnecting unchanged media creates no duplicate backup files.
- The user can determine the last outcome, affected file count, and next required action from the menu bar and notification.

## Deferred Follow-Up Work

- Optional retention rules or a second backup target.
- Audio preview and Finder-style history browser.
- Exportable diagnostics and destination re-indexing UI beyond the recovery behavior required for deletion safety.
- Automatic safe eject after all user-approved deletion work completes.
