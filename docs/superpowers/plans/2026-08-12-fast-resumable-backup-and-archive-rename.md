# Fast Resumable Backup and Archive Rename Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make recorder discovery fast and diagnosable, preserve verified artifacts across rule changes and cancellation, allow archive-folder editing, recover the thirteen currently verifiable DJI-MIC-1 recordings, then build, install, and launch the verified app.

**Architecture:** Separate metadata discovery from content verification, share one stability interval across all matched sources, and locate ledger evidence before any source hash. Treat every stored artifact path as immutable evidence independent of the current rule path, remove production layout relocation, and recover old data through the same no-clobber copy/conversion/evidence primitives. Keep source-retirement revalidation unchanged.

**Tech Stack:** Rust 2024, SQLite/rusqlite, Tauri 2, React 19, TypeScript, Vitest, pnpm, macOS Foundation Trash and `/usr/bin/afconvert`.

## Global Constraints

- Existing artifacts stay at their current paths; archive-folder and date-layout changes apply only to future backups.
- Preserve SHA-256, source metadata, destination containment, no-clobber publication, M4A audio validation and whole-session retirement barriers.
- Do not move, restore or delete source files in recorder Trash during recovery.
- Do not change automatic-backup, automatic-Trash or conversion preferences.
- Use test-driven development and push each meaningful Conventional Commit immediately.
- Do not stop or reinstall the currently running app while it owns an active backup or retirement operation.

---

### Task 1: Remove Automatic Artifact Relocation and Unlock Archive Names

**Files:**
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/src/orchestrator.rs`
- Modify: `src-tauri/src/app_state.rs`
- Modify: `src-tauri/crates/backup-core/src/ledger.rs`
- Modify: `src-tauri/crates/backup-core/src/destination.rs`
- Modify: `src-tauri/crates/backup-core/tests/rule_destinations.rs`
- Modify: `src-tauri/crates/backup-core/tests/rule_ledger.rs`
- Modify: `src-tauri/tests/multi_rule_backup.rs`

**Interfaces:**
- Consumes: existing `BackupRuleDraft`, `VerifiedRecording`, `RuleDestinationPlan` and canonical destination checks.
- Produces: `verified_recording_candidate_for_source(...) -> Option<VerifiedRecording>` and artifact reuse that is independent of the current archive directory.

- [ ] **Step 1: Write failing locked-rename and old-path reuse tests**

Add tests that save a rule with `archive_directory_locked = true`, rename its archive, and expect save success. Add a destination test with an existing M4A under `Old Archive/...`, a current rule using `New Archive`, and expect `DestinationDisposition::Reuse` with the old exact path.

- [ ] **Step 2: Run the focused tests and verify failure**

Run:

```bash
cargo test --manifest-path src-tauri/Cargo.toml -p backup-core --test rule_ledger archive_directory -- --nocapture
cargo test --manifest-path src-tauri/Cargo.toml -p backup-core --test rule_destinations verified_m4a -- --nocapture
```

Expected: rename is rejected and old-archive evidence is rejected before implementation.

- [ ] **Step 3: Stop production migration and relax only archive-path policy**

Remove startup and pre-backup calls to `migrate_legacy_layout`. Remove the save guard that rejects an archive change after evidence, preserve the user's current archive during DJI preset restore, and stop disabling reuse merely because `existing.relative_path` does not begin with the current archive name. Retain safe-relative-path and canonical-root regular-file verification.

- [ ] **Step 4: Add integration coverage for immutable old artifacts**

Back up one file, change archive name and date layout, add a second source file, run again, then assert the first artifact path and bytes are unchanged while only the second artifact uses the new archive/layout.

- [ ] **Step 5: Run focused Rust tests**

```bash
cargo test --manifest-path src-tauri/Cargo.toml -p backup-core --test rule_ledger
cargo test --manifest-path src-tauri/Cargo.toml -p backup-core --test rule_destinations
cargo test --manifest-path src-tauri/Cargo.toml -p backup-mic --test multi_rule_backup
```

- [ ] **Step 6: Commit and push**

```bash
git add src-tauri/src/lib.rs src-tauri/src/orchestrator.rs src-tauri/src/app_state.rs src-tauri/crates/backup-core/src/ledger.rs src-tauri/crates/backup-core/src/destination.rs src-tauri/crates/backup-core/tests/rule_destinations.rs src-tauri/crates/backup-core/tests/rule_ledger.rs src-tauri/tests/multi_rule_backup.rs
git commit -m "fix(backup): preserve artifacts across rule changes"
git push origin refs/heads/main:refs/heads/main
```

### Task 2: Share Stable Discovery and Avoid Planning Hashes

**Files:**
- Modify: `src-tauri/crates/backup-core/src/rule_scanner.rs`
- Modify: `src-tauri/crates/backup-core/src/destination.rs`
- Modify: `src-tauri/crates/backup-core/src/ledger.rs`
- Modify: `src-tauri/crates/backup-core/src/backup.rs`
- Modify: `src-tauri/src/orchestrator.rs`
- Modify: `src-tauri/crates/backup-core/tests/rule_scanner.rs`
- Modify: `src-tauri/crates/backup-core/tests/rule_destinations.rs`
- Modify: `src-tauri/tests/multi_rule_backup.rs`

**Interfaces:**
- Consumes: `RuleScanResult`, `Clock`, `CancellationToken`, source metadata and stored evidence.
- Produces: `RuleScanSnapshot`, `finish_rule_stable_scan(...)`, metadata-first evidence lookup and execution-time source digest verification.

- [ ] **Step 1: Write a failing shared-clock test**

Use a counting `Clock` with two mounted sources. Run `run_matched_sources_with_adapters` and assert exactly one `sleep(STABILITY_INTERVAL)` call, while both sources still require matching first/second metadata snapshots.

- [ ] **Step 2: Write a failing metadata-first planning test**

After the first metadata scan, revoke read permission or inject a source-open fault before execution. Assert preparation can still produce the expected plan and the failure occurs at source verification/copy rather than discovery. Assert an existing metadata-matched recording is returned without a caller-provided hash.

- [ ] **Step 3: Run focused tests and verify failure**

```bash
cargo test --manifest-path src-tauri/Cargo.toml -p backup-core --test rule_scanner
cargo test --manifest-path src-tauri/Cargo.toml -p backup-mic --test multi_rule_backup shared_stability -- --nocapture
```

- [ ] **Step 4: Split stable scan into snapshot and intersection APIs**

Expose a crate-private/public opaque first snapshot that retains file identity. In the orchestrator, scan all eligible sources once, sleep once, scan all sources again, then intersect each pair. Preserve cancellation checks at every current boundary.

- [ ] **Step 5: Add metadata-first ledger lookup**

Add recording and additional-file lookup methods keyed by source id, safe relative path, size and mtime. Validate hydrated evidence. If multiple rows violate uniqueness, return `LedgerCorrupt` rather than picking one.

- [ ] **Step 6: Plan existing evidence without rehashing**

Build an existing-evidence plan from its recorded source hash and exact artifact path. Validate live source content once during `verify_published_artifact`. For a new destination path, derive the default relative path without hashing and let streaming copy compute the digest; retain collision hashing only when the default path is occupied.

- [ ] **Step 7: Preserve no-clobber collision behavior**

If execution discovers an occupied target, compare the computed source digest and choose the shortest hash-suffixed safe target. Never overwrite. Update `DestinationPlan` so an expected hash can be absent for new copies and required for reuse.

- [ ] **Step 8: Run focused and orchestration tests**

```bash
cargo test --manifest-path src-tauri/Cargo.toml -p backup-core --test rule_scanner
cargo test --manifest-path src-tauri/Cargo.toml -p backup-core --test rule_destinations
cargo test --manifest-path src-tauri/Cargo.toml -p backup-core --test backup_flow
cargo test --manifest-path src-tauri/Cargo.toml -p backup-mic --test multi_rule_backup
```

- [ ] **Step 9: Commit and push**

```bash
git add src-tauri/crates/backup-core/src/rule_scanner.rs src-tauri/crates/backup-core/src/destination.rs src-tauri/crates/backup-core/src/ledger.rs src-tauri/crates/backup-core/src/backup.rs src-tauri/src/orchestrator.rs src-tauri/crates/backup-core/tests/rule_scanner.rs src-tauri/crates/backup-core/tests/rule_destinations.rs src-tauri/tests/multi_rule_backup.rs
git commit -m "perf(backup): separate discovery from verification"
git push origin refs/heads/main:refs/heads/main
```

### Task 3: Persist Source Preparation Failures

**Files:**
- Modify: `src-tauri/src/orchestrator.rs`
- Modify: `src-tauri/src/app_state.rs`
- Modify: `src-tauri/src/dto.rs`
- Modify: `src/features/backup/contracts.ts`
- Modify: `src/features/backup/BackupPopover.tsx`
- Modify: `src/features/backup/format.ts`
- Modify: `src-tauri/tests/failure_reporting.rs`
- Modify: `src/features/backup/__tests__/BackupPopover.test.tsx`

**Interfaces:**
- Consumes: `SourceRunOutcome.error`, `FailureReporter`, `PublicError` source decoration and `ActivityEntry`.
- Produces: source-specific `PublicError` in `SourceSnapshotDto.error` plus `source_preparation` diagnostic logging.

- [ ] **Step 1: Write failing backend observability tests**

Inject a preparation-time invalid source and assert the final overall snapshot and source row contain the same source-decorated public error. Assert the privacy-safe failure reporter receives `operation=backup_run`, `stage=source_preparation` and the diagnostic code.

- [ ] **Step 2: Write failing frontend rendering test**

Render two source rows where one has a preparation error. Assert the failed recorder displays the error copy and the healthy recorder result remains visible.

- [ ] **Step 3: Implement source error DTO and reporting**

Add nullable `error` to source snapshots, clear it at operation start, set it from each outcome, and report preparation failures at the point they are converted into `failed_without_run`. Keep path/hash/UUID values out of the public payload.

- [ ] **Step 4: Run focused backend and frontend tests**

```bash
cargo test --manifest-path src-tauri/Cargo.toml -p backup-mic --test failure_reporting
pnpm vitest run src/features/backup/__tests__/BackupPopover.test.tsx src/features/backup/__tests__/contracts.test.ts
```

- [ ] **Step 5: Commit and push**

```bash
git add src-tauri/src/orchestrator.rs src-tauri/src/app_state.rs src-tauri/src/dto.rs src/features/backup/contracts.ts src/features/backup/BackupPopover.tsx src/features/backup/format.ts src-tauri/tests/failure_reporting.rs src/features/backup/__tests__/BackupPopover.test.tsx
git commit -m "fix(backup): surface recorder preparation failures"
git push origin refs/heads/main:refs/heads/main
```

### Task 4: Enable Archive Editing and Normalize Compact Controls

**Files:**
- Modify: `src/features/backup/RuleEditor.tsx`
- Modify: `src/features/backup/RuleList.tsx`
- Modify: `src/index.css`
- Modify: `src/features/backup/__tests__/RuleEditor.test.tsx`
- Modify: `src/features/backup/__tests__/RuleList.test.tsx`
- Modify: `src/features/backup/__tests__/SettingsApp.test.tsx`

**Interfaces:**
- Consumes: unchanged `BackupRuleDraft.archive_directory_name` and `archive_directory_locked` compatibility field.
- Produces: editable archive input with future-files explanation and consistent 11px/32px compact controls.

- [ ] **Step 1: Write failing archive-input tests**

Render a locked DJI rule, change `archive_directory_name`, save, and assert the draft contains the new value. Assert the future-files explanation is present and name edits no longer unexpectedly rewrite an explicitly edited archive.

- [ ] **Step 2: Write failing compact-control assertions**

Assert the restore, add, edit and save actions use the common small button contract and no rule action has a local oversized font class.

- [ ] **Step 3: Implement archive editing and copy**

Remove the locked `disabled` condition. Track whether the archive name was explicitly edited so rule-name synchronization applies only to untouched new drafts. Add the approved explanation and keep validation errors adjacent to the field.

- [ ] **Step 4: Normalize CSS rhythm**

Use 11px labels for buttons/inputs/selects, 32px control height, 10px captions, and the existing 4/8/12/16px spacing tokens. Preserve current light/dark colors and focus states.

- [ ] **Step 5: Run frontend tests and build**

```bash
pnpm vitest run src/features/backup/__tests__/RuleEditor.test.tsx src/features/backup/__tests__/RuleList.test.tsx src/features/backup/__tests__/SettingsApp.test.tsx
pnpm build
```

- [ ] **Step 6: Commit and push**

```bash
git add src/features/backup/RuleEditor.tsx src/features/backup/RuleList.tsx src/index.css src/features/backup/__tests__/RuleEditor.test.tsx src/features/backup/__tests__/RuleList.test.tsx src/features/backup/__tests__/SettingsApp.test.tsx
git commit -m "fix(ui): enable compact archive rule editing"
git push origin refs/heads/main:refs/heads/main
```

### Task 5: Add Verified Artifact Recovery Tool

**Files:**
- Create: `src-tauri/src/bin/recover_verified_recordings.rs`
- Create: `src-tauri/tests/recovery_tool.rs`
- Modify: `src-tauri/src/artifact_pipeline.rs`
- Modify: `src-tauri/crates/backup-core/src/ledger.rs`
- Modify: `scripts/check.sh`

**Interfaces:**
- Consumes: ledger recording id/source evidence, rule destination naming, `AppleAudioTools`, no-clobber copy and `prepare_m4a` verification.
- Produces: a local CLI that accepts `--ledger`, `--destination`, `--source-root` and repeated `--recording-id`, and emits counts only unless diagnostic mode is explicitly requested.

- [ ] **Step 1: Write disposable recovery tests**

Cover source size/hash mismatch, unsafe source root, existing divergent destination, conversion failure and successful recovery. On success assert the original source remains byte-identical, the M4A is verified, and the ledger row points to the new artifact.

- [ ] **Step 2: Run tests and verify the binary is absent**

```bash
cargo test --manifest-path src-tauri/Cargo.toml -p backup-mic --features recovery-tool --test recovery_tool
```

Expected: FAIL until the binary/shared recovery interface exists.

- [ ] **Step 3: Implement recovery from exact ledger evidence**

Canonicalize allowed roots, select records only by explicit id, resolve the matching relative source beneath `--source-root`, verify metadata/hash, copy through a temporary no-clobber WAV, convert and validate M4A, then update the existing row with the verified artifact. Never call Trash adapters.

- [ ] **Step 4: Add dry-run and privacy-safe reporting**

Default to `--dry-run`; require `--apply` for changes. Report selected/recoverable/recovered/failed counts. Diagnostic output may show relative paths but never source hashes.

- [ ] **Step 5: Run focused recovery and artifact tests**

```bash
cargo test --manifest-path src-tauri/Cargo.toml -p backup-mic --features recovery-tool --test recovery_tool
cargo test --manifest-path src-tauri/Cargo.toml -p backup-mic --test artifact_pipeline
```

- [ ] **Step 6: Commit and push**

```bash
git add src-tauri/src/bin/recover_verified_recordings.rs src-tauri/tests/recovery_tool.rs src-tauri/src/artifact_pipeline.rs src-tauri/crates/backup-core/src/ledger.rs scripts/check.sh
git commit -m "feat(recovery): restore verified recorder artifacts"
git push origin refs/heads/main:refs/heads/main
```

### Task 6: Recover the Thirteen DJI-MIC-1 Recordings

**Files:**
- No repository source changes expected.
- Runtime data: `/Users/channprj/Library/Application Support/com.channprj.BackupMic/ledger.sqlite3`
- Destination: `/Volumes/990EVO+/labs/audio-records`
- Read-only source root: `/Volumes/DJI-MIC-1/.Trashes/501`

**Interfaces:**
- Consumes: Task 5 recovery binary and the explicit 13-record manifest derived from current ledger evidence.
- Produces: 13 verified M4A artifacts and updated matching ledger rows while preserving all 13 Trash WAV files.

- [ ] **Step 1: Snapshot and verify current runtime state**

Copy the live SQLite database through SQLite's online backup mechanism to a timestamped diagnostic directory. Record the 13 source sizes/hashes and prove the app has no active backup operation.

- [ ] **Step 2: Run recovery dry-run**

Invoke the recovery binary with the exact 13 recording ids. Require `selected=13`, `recoverable=13`, `failed=0`.

- [ ] **Step 3: Run recovery apply**

Invoke the same exact manifest with `--apply`. If any item fails, stop and retain successful no-clobber artifacts plus the pre-run SQLite snapshot for diagnosis; do not edit source Trash.

- [ ] **Step 4: Independently verify runtime evidence**

For all 13 rows, recompute destination size/hash, inspect M4A codec/sample rate/channels, and compare to SQLite. Recompute all 13 source hashes and prove they remain unchanged in recorder Trash.

### Task 7: Full Verification, Package, Install and Launch

**Files:**
- Generated: `src-tauri/target/release/bundle/macos/Backup Mic.app`
- Generated: `src-tauri/target/release/bundle/dmg/Backup Mic_<version>_aarch64.dmg`

**Interfaces:**
- Consumes: all earlier production and recovery changes.
- Produces: verified installed `/Users/channprj/Applications/Backup Mic.app`, running release process, and clean synchronized Git state.

- [ ] **Step 1: Run repository gates**

```bash
./scripts/check.sh
pnpm test -- --run
pnpm build
cargo test --manifest-path src-tauri/Cargo.toml --workspace
cargo clippy --manifest-path src-tauri/Cargo.toml --workspace --all-targets -- -D warnings
```

- [ ] **Step 2: Review the complete diff and simplify**

Run `git diff <pre-feature-commit>...HEAD`, remove dead compatibility branches and ensure no unrelated user-owned files are staged.

- [ ] **Step 3: Package the release artifact**

```bash
./scripts/package-local.sh "$(tr -d '\r\n' < VERSION)"
```

Require ad-hoc signature, `arm64`, matching bundle version, successful `hdiutil verify`, and recorded executable/DMG SHA-256.

- [ ] **Step 4: Install and launch**

After proving no live operation is active, run:

```bash
./scripts/install-local.sh "src-tauri/target/release/bundle/macos/Backup Mic.app" "$(tr -d '\r\n' < VERSION)"
open "/Users/channprj/Applications/Backup Mic.app"
```

- [ ] **Step 5: Verify installed parity without starting a real backup**

Compare packaged and installed executable hashes, confirm process command and bundle identifier, read current settings and source counts, and ensure automatic backup remains unchanged.

- [ ] **Step 6: Prove Git parity**

```bash
git status --short --branch
git fetch origin main
git rev-list --left-right --count HEAD...origin/main
git ls-remote origin refs/heads/main
```

Expected: clean tree and `0 0` for local/tracking/live remote.
