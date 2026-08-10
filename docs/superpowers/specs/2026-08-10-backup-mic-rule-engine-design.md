# Backup Mic Rule Engine and Product Rename Design

**Date:** 2026-08-10
**Status:** Design approved; written specification awaiting user review
**Target platform:** macOS 13 or newer
**Product:** Backup Mic
**Relationship to existing specifications:** This document generalizes the existing DJI Mic Mini 2S device, scan, backup, conversion, verification, and recoverable Trash contracts. The safety guarantees remain mandatory. The fixed two-transmitter product model and DJI-branded public product identity are replaced by user-configurable backup rules. The calendar archive design remains in force beneath each rule directory.

## Goal

Rename the application from **DJI Mic Backup** to **Backup Mic** and let a user configure rules that recognize recorder storage by its mounted volume name and on-disk directory/file structure. Every matched recorder must enter the same verified pipeline that currently protects DJI Mic recordings:

1. stable source inventory;
2. copy-first, no-clobber finalization;
3. source and destination SHA-256 verification;
4. optional AAC-LC 128 kbps M4A conversion from the verified destination WAV;
5. final artifact verification and durable SQLite evidence; and
6. optional, recoverable source retirement through the macOS Trash API after complete revalidation.

The DJI Mic Mini 2S becomes an editable built-in rule rather than a separate code path. Existing installations retain their selected destination, verified evidence, paired transmitter identities, and recoverable data.

## Settled Product Decisions

- The public product name is `Backup Mic`.
- Rules are created, edited, tested, enabled, archived, and restored in the settings window.
- Rule patterns use glob syntax, not regular expressions.
- `*`, `?`, and recursive `**` matching are supported. Paths use `/` separators regardless of the mounted path representation.
- Volume-name and source-path matching are ASCII case-insensitive and normalize separators to `/`, producing the same rule result on FAT32, exFAT, and case-sensitive fixtures.
- Each rule can require directory or file patterns before it is eligible.
- Each rule declares the files to back up and may declare session directories whose complete contents form one retirement unit.
- Rule-specific prefixes and suffixes change only destination file names. They never rename source files.
- A prefix is inserted before the original stem. A suffix is inserted after the original stem and before the output extension.
- A matched WAV follows the existing WAV/M4A setting. Other selected regular files are copied and verified without conversion.
- Automatic backup, M4A conversion, and automatic Trash settings apply uniformly to every enabled rule.
- Multiple rules matching the same mounted volume are a blocking configuration conflict. The app does not guess.
- A failed source remains intact. Failure for one source does not prevent an independently valid source from completing.

## Product and Technical Rename

The active product identifiers become:

| Surface | New value |
| --- | --- |
| Product and window title | `Backup Mic` |
| App bundle | `Backup Mic.app` |
| Bundle identifier | `com.channprj.BackupMic` |
| Rust package and executable | `backup-mic` |
| Rust library crate | `backup_mic_lib` |
| npm package | `backup-mic` |
| New-install default destination | `~/Documents/Backup Mic` |
| Fallback log root | `~/Library/Logs/com.channprj.BackupMic` |

Public UI, current documentation, scripts, package validation, tray labels, notifications, error messages, and build metadata must use `Backup Mic`. DJI wording remains only where it names the `DJI Mic Mini 2S` built-in rule, describes DJI-specific rule defaults, supports legacy migration, or appears in historical design/plan documents.

The prior installed bundle and prior application data are not deleted automatically. The installer must recognize the exact old bundle as a migration predecessor, request it to quit, install `Backup Mic.app` atomically, and move the replaced old bundle to recoverable macOS Trash only after the new bundle passes verification.

## Rule Model

SQLite stores a `backup_rules` record with these logical fields:

| Field | Contract |
| --- | --- |
| `id` | Stable UUID used by evidence and runtime identity |
| `name` | Unique user-visible name; editable without relocating existing artifacts |
| `archive_directory_name` | Safe destination directory; defaults to the initial rule name and becomes immutable after the first verified artifact |
| `enabled` | Whether the rule participates in mount matching |
| `volume_name_glob` | Required mounted volume-name pattern |
| `required_path_globs` | May be empty; when present, every pattern must have at least one match |
| `backup_file_globs` | Non-empty list of files to copy; a match must yield at least one regular file |
| `session_directory_globs` | Optional directories treated as complete retirement units |
| `filename_prefix` | Optional destination stem prefix |
| `filename_suffix` | Optional destination stem suffix |
| `filename_profile` | `preserve` for user rules or the preset-owned `dji_tx_short` normalization profile |
| `preset_kind` | `dji_mic_mini_2s` for the built-in rule, otherwise absent |
| `preset_revision` | Revision used by explicit default restoration |
| `archived_at` | Soft-deletion timestamp when durable evidence references the rule |
| `created_at`, `updated_at` | Auditable timestamps |

Pattern lists are stored in normalized child rows rather than executable or opaque configuration. Rule writes are transactional. The backend compiles and validates every glob before committing it, then reads the saved rule back before the UI considers the save complete.

Rule names and archive directory names reject `/`, NUL, control characters, `.`/`..` path components, leading hidden-file markers, and values that normalize to empty. Prefix and suffix may be empty; when present they reject separators, NUL, control characters, and whitespace-only values. A rule name is unique under case-insensitive Unicode normalization. Names, prefixes, and suffixes are limited to 64 Unicode scalar values. Each pattern is limited to 256 Unicode scalar values, each pattern group is limited to 32 entries, recursion is limited to 32 directory levels, and one match attempt visits at most 100,000 entries.

Rules referenced by ledger evidence are archived instead of physically deleted. An unused user rule may be deleted. Archived rules remain readable to verification and migration code but cannot match a newly mounted volume.

## Dynamic Source Identity

The fixed `Transmitter::Tx01` and `Transmitter::Tx02` runtime assumption becomes a dynamic source identity:

```text
RuleSourceId = rule_id + volume_uuid + mount_generation
```

The rule selects what a volume means. The Disk Arbitration descriptor still supplies the authoritative UUID, protocol, internal/removable/writable flags, media identity, capacity, mount root, and mount generation. The UUID and mount generation are frozen into every operation and rechecked before copying, committing evidence, or retiring a source.

User-facing source labels use the rule name and mounted volume name. DJI source labels may additionally show the migrated `TX01` or `TX02` slot. Durable ledger rows retain the source's rule ID and device identity so histories remain distinguishable even when two volumes share a display name.

All existing structures keyed by the two-value transmitter enum—including snapshots, rescan scheduling, batch items, activities, deletion proposals, and verified artifacts—must accept dynamic source IDs. DTOs expose arrays of source snapshots rather than two fixed fields. The command surface remains narrow; it accepts validated rule DTOs and source IDs, never arbitrary filesystem paths or arbitrary settings keys.

## Built-In DJI Mic Mini 2S Rule

Every new ledger receives one enabled, editable preset named `DJI Mic Mini 2S`. Its preset revision owns:

- a `*` volume-name glob so renamed transmitters remain eligible only when the DJI physical-media and path constraints also match;
- the existing accepted DJI media identity and capacity constraints;
- root and `TX_MIC...` session layout recognition;
- DJI WAV file patterns;
- session-directory patterns that preserve complete-folder retirement; and
- the current `TX01_` to `T01_` and `TX02_` to `T02_` filename normalization through the preset's `dji_tx_short` profile.

The preset uses the same rule matcher, scanner, planner, batch pipeline, ledger, and retirement engine as every user-created rule. Its existing accepted media names (`Mic Tx` and `Wireless Mic Tx Media`), USB/removable/writable requirements, and 12-20 GB nominal-capacity range are stored as preset-owned match constraints, not evaluated by a parallel DJI matcher. User-created rules rely on their volume and path globs and the shared external/removable/writable source policy; they do not inherit the DJI capacity or media-name limits.

Existing paired TX01/TX02 rows migrate into rule-device bindings for this preset. Their volume UUID, protocol, media name, capacity, and slot label remain available as stronger trusted-device evidence. A migrated trusted DJI device may still match after its volume label changes, preserving the existing safety contract. A new DJI device is recognized by the preset's generic volume, physical-media, and path constraints without a separate DJI pairing path.

The user may edit or disable the preset. `Restore defaults` replaces its editable pattern fields with the current preset revision after a confirmation dialog; it does not delete ledger evidence or device bindings. If the preset has been archived, restore reactivates it.

## Rule Editor

The settings window adds a `Backup Rules` section below the destination and global backup controls. It contains:

- enabled and archived rule lists;
- `Add rule`, `Edit`, `Duplicate`, `Archive`, and `Restore defaults` actions;
- inputs for rule name, archive directory, volume glob, required paths, backup files, optional session directories, prefix, and suffix;
- inline glob and path-safety diagnostics;
- a read-only destination-name preview; and
- `Test against connected disks`.

Pattern lists use repeatable text fields rather than comma-separated strings. The UI explains that every required path must match, while any backup-file pattern may contribute files. It shows representative examples such as `ZOOM_*`, `RECORD/**`, and `RECORD/**/*.WAV` without treating them as implicit defaults. The archive directory defaults to the rule name. After the first artifact is verified, it is displayed read-only so renaming the user-visible rule cannot split one recorder's archive across directories.

Testing a rule is a backend read-only operation over the currently observed volumes. It returns sanitized results: matched volume display names, matched recording/file counts, and safe conflict or validation codes. It does not expose UUIDs, hashes, or absolute source paths to the webview and never starts a backup.

When a rule save would overlap another enabled rule on a currently connected disk, the UI displays the conflict before saving. The backend still permits a valid rule that cannot be compared because its target disk is absent; runtime ambiguity remains fail-closed.

## Match and Scan Flow

For each Disk Arbitration mount event, the backend:

1. rejects internal, non-removable, non-writable, or non-USB/external storage according to the existing source policy;
2. canonicalizes the mount root without following a candidate symlink;
3. snapshots enabled rules and compiles their validated globs;
4. matches the sanitized mounted volume name;
5. requires every `required_path_glob` to produce at least one safe entry;
6. collects safe regular files matching any `backup_file_glob`;
7. records session membership for files below matching `session_directory_globs`; and
8. accepts exactly one matching rule with at least one selected file.

Zero matches are unrelated storage and produce no user-facing failure. More than one match produces a rule-conflict activity and UI state, and no backup begins for that volume.

The walker stays under the canonical mount root, does not follow symlinks, rejects hard-to-classify special files, skips `.Trashes` and system metadata directories, and bounds recursion depth and total visited entries. A session directory containing a symlink, nested unrecognized directory, or unsafe entry is reported as incomplete and cannot become Trash-eligible.

The first inventory and the inventory after the existing two-second stability interval must agree on relative path, regular-file identity, size, and modification time. Connected sources retain the 15-second metadata rescan. A changed fingerprint queues a new backup only when automatic backup is enabled. Rules and global preferences are frozen for one run; edits apply to the next run.

## Destination Layout and Naming

To preserve the established calendar archive contract while isolating recorders, recording artifacts use:

```text
<destination>/<archive directory>/YYYY/MM/YYMMDD-<prefix><source stem><suffix>.<extension>
```

For example:

```text
Rule: Zoom H1n
Prefix: zoom-
Suffix: -field
Source: RECORD/FOLDER01/ZOOM0001.WAV

Backup Mic/Zoom H1n/2026/08/260810-zoom-ZOOM0001-field.m4a
```

The calendar date comes from a rule-recognized timestamp when the preset supplies a parser; otherwise it comes from the stable source modification date in the local timezone. A prefix or suffix never changes the extension. With M4A conversion disabled, the final extension remains `.wav`.

Selected non-recording files use a reserved companion hierarchy beneath the same archive directory so they cannot be confused with playable artifacts:

```text
<destination>/<archive directory>/source-extras/<source evidence key>/<safe relative path>
```

The exact evidence key is deterministic and non-sensitive. It cannot expose the source UUID or absolute mount path.

If the planned destination is absent, it is finalized without clobbering. If an existing regular file has the same verified bytes, it is reused. If different content occupies the requested name, the shortest unique SHA-256 prefix is appended to the stem. No existing destination file or directory is overwritten.

## Backup, Conversion, and Independent Failure Boundaries

Each matched source receives its own copy barrier and conversion cohort:

1. create a stable complete inventory;
2. calculate required destination capacity, including staging and reserve;
3. copy every selected source through an app-owned temporary file;
4. flush, close, and verify source/destination size and SHA-256;
5. finalize without clobbering and commit source-copy evidence;
6. when enabled, convert verified destination WAVs with the existing AAC-LC 128 kbps profile;
7. inspect audio container, codec, channels, sample rate, frames, duration, size, and hash;
8. commit a complete per-source conversion barrier; and
9. revalidate the live source snapshot and final artifacts.

No source conversion begins before that source's copy barrier is complete. One source's failure prevents conversion and retirement only for that source. Other source cohorts may complete. The process-wide operation guard may serialize filesystem work, but the ledger represents independent source outcomes.

Manual and automatic backup use the same planner. Manual `Back Up Now` requests a scan for every currently matched, non-conflicting source. Reconnecting or rescanning verified content reuses ledger evidence only after the source and destination files are rehashed as required by the existing reuse contract.

## Recoverable Source Retirement

Global automatic Trash and manual Trash actions apply to dynamic sources. A proposal contains an opaque ID, source ID, rule revision, mount and scan generations, destination generation, complete source inventory, durable copy/conversion barrier evidence, and a five-minute expiry.

Immediately before retirement, the backend rechecks:

- the same enabled rule and frozen revision still match;
- the volume UUID, physical properties, mount generation, and canonical mount root;
- every selected source path, size, modification time, and SHA-256;
- every session directory's complete regular-file inventory;
- all destination artifacts and conversion evidence; and
- the durable daily audit-log boundary.

A matched session directory moves to macOS Trash as one recoverable item only when every regular entry has durable evidence and no unsafe or unrecognized entry exists. Files outside a declared session directory move individually. The app uses the Foundation Trash API only; it does not invoke Finder, AppleScript, a shell deletion command, `rm`, or direct `.Trashes` manipulation, and it never empties Trash.

Rule edits, rule archival, device removal, mount change, destination change, new scan, source change, or backup start immediately invalidate outstanding proposals. Disabling M4A conversion retains the existing rule that WAV sources are not made eligible for automatic or manual retirement.

## Data and Artifact Migration

### Application data identity

On first launch under `com.channprj.BackupMic`, if the new ledger is absent and the legacy ledger exists at the prior app-data location, the app:

1. opens the legacy SQLite database through SQLite's online backup mechanism and takes one consistent immutable snapshot, including committed WAL contents;
2. writes the snapshot to an owner-only temporary file under the new app-data directory;
3. opens it, runs integrity checks and schema migrations, and validates required settings and evidence;
4. synchronizes and atomically installs it as `ledger.sqlite3`; and
5. leaves the complete legacy app-data directory unchanged.

If migration fails, the app does not create a partial authoritative ledger. It reports a sanitized startup error in the new fallback log and preserves both the legacy source and any owned temporary recovery evidence.

The selected destination setting is retained. Only a ledger with no configured destination receives the new `~/Documents/Backup Mic` default.

### Ledger generalization

The migration creates the built-in DJI rule, converts paired transmitter rows into DJI rule-device bindings, and gives existing verified recordings, additional files, batches, activities, and retirement evidence a stable source/rule association. It must preserve every existing hash, byte count, timestamp, conversion status, and deletion status. Legacy transmitter labels remain display metadata rather than an enum constraint.

### Existing destination artifacts

Ledger-verified DJI artifacts in the current `YYYY/MM` calendar layout migrate beneath the new rule directory:

```text
YYYY/MM/YYMMDD-T01_....m4a
-> DJI Mic Mini 2S/YYYY/MM/YYMMDD-T01_....m4a
```

Migration is idempotent and uses the existing verified relocation discipline:

1. require a regular old artifact under the canonical destination root;
2. recompute and match ledger size and SHA-256;
3. copy through a synchronized app-owned temporary file;
4. recompute and match the target;
5. move the old artifact to macOS Trash;
6. atomically relocate the ledger path; and
7. remove only empty app-owned legacy calendar directories.

If the old artifact is absent but a verified target exists, the ledger path may be repaired. If neither is verified, the record is skipped and preserved for diagnosis. A name collision follows the same content-reuse or hash-suffix rule as a new backup.

Non-recording legacy `source-extras` evidence is migrated under the DJI rule hierarchy with the same byte-for-byte discipline. Logs remain where written; new runs use the renamed log root and continue to write `YYMMDD-backup-mic.log`.

## Error Handling and User Feedback

The backend returns structured, sanitized errors for:

- invalid or unsafe glob syntax;
- invalid names, prefixes, or suffixes;
- rule not currently matching a connected volume;
- ambiguous matching rules;
- unsafe source entries;
- changing/incomplete source inventories;
- destination conflicts or insufficient capacity;
- migration failures; and
- copy, hash, conversion, ledger, audit-log, or Trash failures.

The settings UI places input errors next to the responsible field. Runtime conflicts name the conflicting rules and volume display name without exposing UUIDs, hashes, or absolute source paths. Activity and daily logs record rule/source-safe identifiers and operation stages. A failed save leaves the prior persisted rule active.

## Security and Safety Boundaries

- The webview never receives arbitrary filesystem capabilities.
- All filesystem access stays in Rust behind fixed commands and validated DTOs.
- Rule testing is read-only.
- Glob syntax is declarative; no regex, shell, executable expression, template evaluation, or environment interpolation is accepted.
- Canonical-root checks, no-symlink traversal, bounded walking, and no-clobber finalization are mandatory.
- Device and rule identities are revalidated at every destructive boundary.
- Source deletion remains recoverable and separately authorized from backup.
- Existing destination, ledger, legacy app data, and legacy app bundle content are never permanently deleted by migration.

## Testing and Acceptance Requirements

### Rule engine and core tests

Automated Rust tests must prove:

1. volume and path glob semantics for `*`, `?`, and `**`;
2. all required paths must match and at least one backup file must match;
3. unsafe paths, symlinks, `.Trashes`, hidden system metadata, excessive recursion, and out-of-root entries are rejected or skipped safely;
4. zero, one, and multiple matching rules produce unrelated, accepted, and conflict outcomes respectively;
5. prefixes and suffixes produce the approved stem and preserve `.wav`/`.m4a` extension behavior;
6. two rules with equal source file names cannot clobber one another;
7. equal content is reused and different-content collisions receive a hash suffix;
8. WAV and non-WAV selected files enter the correct verified pipelines;
9. rule edits are frozen per run and invalidate Trash proposals;
10. source cohorts fail independently;
11. session directories retire only with complete evidence;
12. dynamic sources rescan after 15 seconds and queue changed inventories;
13. the DJI preset matches current fixtures through the generic matcher; and
14. default restoration changes preset configuration without deleting evidence.

### Migration tests

Tests must open a pre-rule legacy database and prove:

1. settings, paired devices, recordings, artifacts, additional files, batches, activities, and retirement evidence survive exactly;
2. TX01/TX02 map to DJI preset source bindings;
3. interrupted, invalid, corrupt, or WAL-backed legacy databases fail safely;
4. application-data migration is atomic and leaves the legacy source unchanged;
5. existing artifacts move under the DJI rule directory with identical size and SHA-256;
6. old artifacts move to recoverable Trash only after target verification;
7. mismatched artifacts remain untouched;
8. re-running each migration is a no-op; and
9. a user-selected destination remains selected.

### Frontend and IPC tests

Tests must prove:

1. exact typed DTOs for dynamic source snapshots and backup rules;
2. exact fixed command registration with no arbitrary setting or path IPC;
3. create, edit, duplicate, enable, archive, and restore-default flows;
4. inline glob/name/prefix/suffix validation;
5. connected-disk rule-test results and ambiguity feedback;
6. global conversion and Trash controls visibly apply to every rule; and
7. Backup Mic naming across setup, popover, settings, dialogs, notifications, and preview states.

### Repository, package, and installed-app gates

Before completion:

1. `./scripts/check.sh` passes in full.
2. Disk-image fixtures with at least two volume labels and different directory layouts prove automatic mount recognition and independent backup.
3. The destructive fixture proves verified session/file Trash behavior without touching a real recorder.
4. The release bundle and DMG build successfully for arm64 macOS 13+.
5. Bundle identifier, executable name, version, deep strict signature, architecture, and checksums match the renamed package contract.
6. The installed app is `/Users/channprj/Applications/Backup Mic.app`, and its executable hash matches the packaged app.
7. A fresh-install fixture receives the DJI preset and `~/Documents/Backup Mic` default.
8. An upgrade fixture preserves the prior destination and DJI evidence.
9. Search confirms no current public product surface still calls the application DJI Mic Backup; allowed DJI occurrences are limited to the preset, device-specific help, migration compatibility, fixtures, and historical documents.
10. The worktree is clean and `HEAD...@{u}` reports `0 0` after every pushed checkpoint.

Real connected-recorder evidence is reported separately if no physical recorder is available during verification. Disk-image fixture evidence does not claim native hardware identity or a real Disk Arbitration device session.

## Release and Git Contract

Implementation is split into independently green Conventional Commit checkpoints and each checkpoint is pushed immediately through the requested `$gcpr` workflow. No history rewrite or force push is permitted. A release/version checkpoint is created only after the implementation, migrations, UI, full checks, and package verification are green. Existing unrelated user changes are never staged.
