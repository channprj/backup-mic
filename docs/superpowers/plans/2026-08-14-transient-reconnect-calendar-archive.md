# Transient Reconnect and Calendar Archive Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make manual and interrupted backups wait safely for recorder reconnection, and place every verified DJI recording directly under the selected destination's `YYYY/MM` hierarchy.

**Architecture:** Add a process-local retry-intent state machine beside `AppState`, then route manual commands, mount events, cancellation, and interrupted runs through it while preserving the operation guard. Specialize only DJI recording destinations and add an idempotent ledger-backed migration; generic recorder rules remain unchanged.

**Tech Stack:** Rust 2024, Tauri 2, SQLite/rusqlite, React 19, TypeScript 7, Vitest, macOS Foundation Trash, `/usr/bin/afconvert`, Headatever.

## Global Constraints

- A manual request without a recorder waits until a matching recorder starts one backup, the user cancels, or the app exits.
- A reconnect-triggered request runs even when automatic backup is disabled.
- Expected recorder absence must not emit `operation.failed` with `device_removed`; real destination, permission, hash, conversion, ledger, and Trash failures remain errors.
- WAV files are copied and SHA-256 verified before AAC-LC M4A conversion at 128 kbps.
- No source or destination artifact is permanently deleted; verified superseded files move through the macOS Trash adapter.
- DJI recording artifacts use `<destination>/YYYY/MM/YYMMDD-T01_or_T02_<remaining name>.<extension>` without archive, day, or transmitter directories.
- Non-DJI rules retain their configured archive directory and date layout.
- Migration is no-clobber, regular-file-only, SHA-256 verified, ledger-backed, idempotent, and collision-safe.
- Use red-green TDD and push every green Conventional Commit immediately.
- Do not replace a running app while it owns an active backup or retirement operation.

---

### Task 1: Add a Race-Safe Manual Backup Intent

**Files:**
- Create: `src-tauri/src/manual_backup.rs`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/src/app_state.rs`

**Interfaces:**
- Consumes: `backup_core::source::SourceId` and mounted-source sets.
- Produces: `ManualBackupIntent::{request, claim, finish, interrupt_claim, cancel, should_start_for, is_waiting}` and `ManualBackupClaim`.

- [ ] **Step 1: Write failing state-machine tests**

Add tests for an any-source disconnected request, two interrupted source IDs reconnecting one at a time, duplicate claims, remaining targets, and cancellation winning over a late interruption:

```rust
#[test]
fn disconnected_request_waits_until_a_source_can_be_claimed() {
    let intent = ManualBackupIntent::default();
    intent.request();
    intent.wait_for_device();
    assert!(intent.is_waiting());
    let claim = intent.claim(&[source("tx01")]).unwrap();
    assert!(claim.resumed());
}

#[test]
fn cancellation_prevents_a_late_retry() {
    let intent = ManualBackupIntent::default();
    intent.request();
    let claim = intent.claim(&[source("tx01")]).unwrap();
    intent.cancel();
    intent.interrupt_claim(claim, [source("tx01")], true);
    assert!(!intent.is_waiting());
}
```

- [ ] **Step 2: Run the focused test and verify red**

```bash
cargo test --manifest-path src-tauri/Cargo.toml -p backup-mic manual_backup::tests -- --nocapture
```

Expected: compilation fails because the module and types do not exist.

- [ ] **Step 3: Implement the mutex-backed state machine**

Represent state as `None`, `PendingAny`, `PendingSources(BTreeSet<SourceId>)`, or `Running { remaining, claimed, resumed }`. `claim` consumes only mounted target IDs, keeps missing targets in `remaining`, and returns no claim when a mount cannot satisfy the request. `finish` restores remaining targets; `cancel` always clears state.

- [ ] **Step 4: Install narrow delegates in `AppState`**

```rust
pub(crate) fn request_manual_backup(&self);
pub(crate) fn claim_manual_backup(&self, sources: &[SourceId]) -> Option<ManualBackupClaim>;
pub(crate) fn manual_backup_should_start_for(&self, source_id: &SourceId) -> bool;
pub(crate) fn manual_backup_is_waiting(&self) -> bool;
pub(crate) fn cancel_manual_backup(&self);
```

- [ ] **Step 5: Verify, commit, and push**

```bash
cargo test --manifest-path src-tauri/Cargo.toml -p backup-mic manual_backup::tests
cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check
git diff --check
git add src-tauri/src/manual_backup.rs src-tauri/src/lib.rs src-tauri/src/app_state.rs
git commit -m "feat(backup): track reconnectable manual requests"
git push origin main
```

### Task 2: Queue, Resume, and Cancel Recorder Backups

**Files:**
- Modify: `src-tauri/src/manual_backup.rs`
- Modify: `src-tauri/src/app_state.rs`
- Modify: `src-tauri/src/commands.rs`
- Modify: `src-tauri/src/orchestrator.rs`
- Modify: `src-tauri/crates/backup-core/src/backup.rs`
- Test: `src-tauri/tests/failure_reporting.rs`

**Interfaces:**
- Consumes: Task 1 intent, `OperationGuard`, `CancellationToken`, matched authority, and audit logging.
- Produces: `request_backup(app, state)`, trigger-aware `start_backup`, and INFO waiting/resume events.

- [ ] **Step 1: Write failing configuration, cancellation, and scheduler tests**

Replace `manual_backup_reports_a_missing_recorder_instead_of_an_invalid_request` with assertions that configuration preflight succeeds without a recorder and the request waits. Add tests proving cancellation clears pending intent, `Busy` retains it, a matching mount selects a manual trigger even with automatic backup off, and duplicate mount events cannot claim twice.

- [ ] **Step 2: Write failing interruption-classification tests**

Inject a source-open failure after invalidating authority and expect `DeviceRemoved`. Keep authority current and expect `CopyFailed`. Add `CancellationToken::is_cancelled()` coverage so user cancellation cannot be reclassified as a reconnect request.

- [ ] **Step 3: Run focused tests and verify red**

```bash
cargo test --manifest-path src-tauri/Cargo.toml -p backup-mic app_state::tests::manual_backup -- --nocapture
cargo test --manifest-path src-tauri/Cargo.toml -p backup-mic orchestrator::tests::manual_reconnect -- --nocapture
cargo test --manifest-path src-tauri/Cargo.toml -p backup-core cancellation -- --nocapture
```

- [ ] **Step 4: Split configuration and mount preflight**

Validate setup and destination before setting intent, but do not require a mounted recorder. With zero matches publish `BackupPhase::Detecting`, `message_code="waiting_for_device"`, no public error/current stage, append `backup.waiting_for_device` at INFO, and return command success without using `FailureReporter`.

- [ ] **Step 5: Make starts trigger-aware and guard-first**

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BackupTrigger {
    Automatic,
    Manual,
}
```

Acquire the operation guard before claiming manual intent. Preserve the request on `Busy`. Pass the claim into the background run; normal completion calls `finish`, cancellation clears it, and a lost source returns only that source ID to the pending set.

- [ ] **Step 6: Schedule a pending request on a matching mount**

`handle_mounted` schedules when automatic backup is enabled or `manual_backup_should_start_for(source_id)` is true. Do not clear intent when the matched-source map is empty. Append `backup.resumed_after_reconnect` at INFO only after an accepted resumed start.

- [ ] **Step 7: Settle expected disconnection without an error**

At each source error boundary preserve `Cancelled`; otherwise map the error to `DeviceRemoved` when frozen authority is no longer current. Publish the waiting snapshot, clear public errors/current stages, retain healthy verified outcomes, and skip `operation.failed` plus error-severity activity for expected disconnection. Real errors keep existing reporting.

- [ ] **Step 8: Make cancellation terminal**

Clear intent before cancelling the operation token. With no operation active, publish the existing cancelled/idle state. Later mounts follow only automatic-backup preference.

- [ ] **Step 9: Verify, commit, and push**

```bash
cargo test --manifest-path src-tauri/Cargo.toml -p backup-mic app_state::tests
cargo test --manifest-path src-tauri/Cargo.toml -p backup-mic orchestrator::tests
cargo test --manifest-path src-tauri/Cargo.toml -p backup-mic --test failure_reporting
cargo test --manifest-path src-tauri/Cargo.toml -p backup-core cancellation
cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check
cargo clippy --manifest-path src-tauri/Cargo.toml -p backup-mic --all-targets -- -D warnings
git add src-tauri/src/manual_backup.rs src-tauri/src/app_state.rs src-tauri/src/commands.rs src-tauri/src/orchestrator.rs src-tauri/crates/backup-core/src/backup.rs src-tauri/tests/failure_reporting.rs
git commit -m "fix(backup): resume safely after recorder reconnect"
git push origin main
```

### Task 3: Render a Cancellable Waiting State

**Files:**
- Modify: `src/features/backup/BackupPopover.tsx`
- Modify: `src/features/backup/format.ts`
- Modify: `src/features/backup/__tests__/BackupPopover.test.tsx`
- Modify: `src/features/backup/__tests__/format.test.ts`

**Interfaces:**
- Consumes: detecting phase plus `message_code="waiting_for_device"`.
- Produces: approved waiting copy and one cancel action without an error alert.

- [ ] **Step 1: Write failing UI tests**

```tsx
expect(screen.getByText("녹음기 연결을 기다리는 중")).toBeInTheDocument();
expect(screen.getByText("녹음기를 연결하면 백업을 자동으로 시작합니다.")).toBeInTheDocument();
expect(screen.getByRole("button", { name: "백업 취소" })).toBeEnabled();
expect(screen.queryByText(/오류 코드:/)).not.toBeInTheDocument();
expect(screen.queryByText("원본은 변경되지 않았습니다")).not.toBeInTheDocument();
```

Also assert cancel invokes once and Command-R cannot enqueue a duplicate while waiting.

- [ ] **Step 2: Run the focused suite and verify red**

```bash
pnpm exec vitest run src/features/backup/__tests__/BackupPopover.test.tsx src/features/backup/__tests__/format.test.ts
```

- [ ] **Step 3: Implement waiting copy and controls**

Add `isWaitingForDevice(snapshot)`. Check it before generic detecting copy in `activeTitle` and `stageLabel`. Render `대기 중`, use the approved detail in the footer, hide the undiscovered six-stage sequence, and retain the active-phase cancel button.

- [ ] **Step 4: Remove obsolete copy from exceptional `device_removed` UI**

Keep the error code for real exceptional contexts, but change its detail to `녹음기를 다시 연결해 주세요.`

- [ ] **Step 5: Verify, commit, and push**

```bash
pnpm exec vitest run src/features/backup/__tests__/BackupPopover.test.tsx src/features/backup/__tests__/format.test.ts
pnpm typecheck
pnpm build
git diff --check
git add src/features/backup/BackupPopover.tsx src/features/backup/format.ts src/features/backup/__tests__/BackupPopover.test.tsx src/features/backup/__tests__/format.test.ts
git commit -m "fix(ui): show recorder reconnect waiting state"
git push origin main
```

### Task 4: Plan DJI Recordings Directly Under `YYYY/MM`

**Files:**
- Modify: `src-tauri/crates/backup-core/src/destination.rs`
- Modify: `src-tauri/crates/backup-core/tests/rule_destinations.rs`
- Modify: `src-tauri/crates/backup-core/tests/rule_ledger.rs`

**Interfaces:**
- Consumes: DJI device constraint, `FilenameProfile::DjiTxShort`, `destination_stem`, and archive date.
- Produces: direct-root DJI recording paths; generic and companion paths stay unchanged.

- [ ] **Step 1: Write failing destination tests**

```rust
assert_eq!(
    plan.relative_destination,
    Path::new("2026/08/260809-rec-T01_MIC001_20260809_010203-backup.wav")
);
```

Add TX02, case-insensitive prefix, M4A extension, and same-month co-location cases. Keep Zoom expectations under `Zoom H1n/2026/08`.

- [ ] **Step 2: Run the focused test and verify red**

```bash
cargo test --manifest-path src-tauri/Cargo.toml -p backup-core --test rule_destinations dji -- --nocapture
```

- [ ] **Step 3: Implement a DJI-only recording root policy**

When both device constraint and filename profile are DJI, choose `year/month` without the stored archive directory and ignore day layout for recordings. Keep prefix/suffix and T01/T02 normalization. All other rules keep their configured layout.

- [ ] **Step 4: Verify, commit, and push**

```bash
cargo test --manifest-path src-tauri/Cargo.toml -p backup-core --test rule_destinations
cargo test --manifest-path src-tauri/Cargo.toml -p backup-core --test rule_ledger
cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check
git diff --check
git add src-tauri/crates/backup-core/src/destination.rs src-tauri/crates/backup-core/tests/rule_destinations.rs src-tauri/crates/backup-core/tests/rule_ledger.rs
git commit -m "feat(archive): store DJI recordings by calendar month"
git push origin main
```

### Task 5: Migrate Verified DJI Artifacts Safely

**Files:**
- Modify: `src-tauri/crates/backup-core/src/layout.rs`
- Create: `src-tauri/crates/backup-core/tests/dji_calendar_migration.rs`
- Modify: `src-tauri/src/orchestrator.rs`
- Modify: `src-tauri/tests/multi_rule_backup.rs`

**Interfaces:**
- Consumes: verified recordings, source/rule lookup, verified size/SHA-256, `TrashAdapter`, and cancellation.
- Produces: `migrate_verified_dji_calendar_layout(...) -> Result<Vec<LayoutMigration>, CoreError>`.

- [ ] **Step 1: Write failing migration tests**

Seed these verified layouts and assert direct `YYYY/MM` targets:

```text
DJI Mic Mini 2S/2026/08/14/260814-T01_MIC001_20260814_010203_edit.m4a
DJI Mic Mini 2S/2026/08/260814-T02_MIC002_20260814_010204_edit.m4a
2026/2026-08-14/TX01/TX01_MIC003_20260814_010205_edit.m4a
2026/2026-08-14/TX02_MIC004_20260814_010206_edit.wav
```

Cover equal-content reuse, divergent hash suffix, ledger relocation, Trash movement, empty ancestor cleanup, tamper and symlink refusal, cancellation, idempotency, and one untouched generic-rule record.

- [ ] **Step 2: Run the new test and verify red**

```bash
cargo test --manifest-path src-tauri/Cargo.toml -p backup-core --test dji_calendar_migration -- --nocapture
```

- [ ] **Step 3: Implement deterministic target derivation**

For DJI/DjiTxShort records, take the original source stem, derive its valid encoded DJI date or the verified artifact's unambiguous date prefix/ancestors, apply `destination_stem`, retain artifact extension, and form `YYYY/MM/YYMMDD-<stem>.<extension>`. Skip records without one safe date and skip canonical targets.

- [ ] **Step 4: Reuse verified copy, Trash, and ledger primitives**

Require canonical regular files with recorded size/hash; temporary-copy, sync, re-hash, no-clobber finalize, Trash the old artifact, relocate ledger evidence, and prune only empty ancestors below root. Reuse equal targets and suffix divergent collisions with the shortest unique SHA-256 prefix.

- [ ] **Step 5: Run migration before evidence reuse planning**

At the start of the production rule backup, after destination creation and owned-partial cleanup, run resilient migration once. Report real failures through the privacy-safe reporter, append INFO audit records for successful moves, and never migrate companion files.

- [ ] **Step 6: Verify, commit, and push**

```bash
cargo test --manifest-path src-tauri/Cargo.toml -p backup-core --test dji_calendar_migration
cargo test --manifest-path src-tauri/Cargo.toml -p backup-core --test rule_layout_migration
cargo test --manifest-path src-tauri/Cargo.toml -p backup-core --test layout_migration
cargo test --manifest-path src-tauri/Cargo.toml -p backup-mic --test multi_rule_backup
cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check
cargo clippy --manifest-path src-tauri/Cargo.toml --workspace --all-targets -- -D warnings
git add src-tauri/crates/backup-core/src/layout.rs src-tauri/crates/backup-core/tests/dji_calendar_migration.rs src-tauri/src/orchestrator.rs src-tauri/tests/multi_rule_backup.rs
git commit -m "feat(archive): migrate verified DJI recordings safely"
git push origin main
```

### Task 6: Verify, Version, Package, Install, and Launch

**Files:**
- Modify through Headatever: `VERSION`
- Synchronize: `package.json`, `src-tauri/Cargo.toml`, `src-tauri/tauri.conf.json`, `src-tauri/Cargo.lock`
- Produce: `src-tauri/target/release/bundle/macos/Backup Mic.app`
- Install: `/Users/channprj/Applications/Backup Mic.app`

**Interfaces:**
- Consumes: all green implementation commits and release scripts.
- Produces: bumped release metadata, matching package/install hashes, launched app, and Git parity.

- [ ] **Step 1: Run the complete source gate**

```bash
./scripts/check.sh
git diff --check
git status --short --branch
```

- [ ] **Step 2: Inspect runtime safety before replacement**

Read bundle version, process list, ledger operation rows, mounted DJI volumes, and current destination/preferences. Stop if backup or retirement is active. Do not trigger source retirement against a physical recorder during verification.

- [ ] **Step 3: Use Headatever after development is green**

Inspect tags/worktrees/main parity, run the repository-approved Headatever version bump, synchronize the exact version across npm/Tauri/Cargo surfaces, regenerate only required lock metadata, verify consistency, then commit and push without moving an existing tag.

- [ ] **Step 4: Re-run validation and package**

```bash
./scripts/check.sh
./scripts/package-local.sh "$(tr -d '\r\n' < VERSION)"
codesign --verify --deep --strict --verbose=2 "src-tauri/target/release/bundle/macos/Backup Mic.app"
```

Verify identifier, version, architecture, signature, and packaged executable hash.

- [ ] **Step 5: Replace the installed app recoverably**

Use the repository safe install workflow so the existing app moves to macOS Trash, install the exact packaged bundle, and prove packaged/installed executable SHA-256 equality. Never use recursive permanent deletion.

- [ ] **Step 6: Launch and collect runtime evidence**

Prove process path and bundle version. Where safe GUI automation is available, verify the disconnected waiting copy and absence of a new `operation.failed ... device_removed`. If a matching recorder is available, prove one resumed run; otherwise report that physical-device boundary explicitly.

- [ ] **Step 7: Finish with parity proof**

```bash
git status --short --branch
git rev-list --left-right --count HEAD...@{upstream}
git ls-remote origin refs/heads/main
```

Expected: clean tree, `0 0`, remote SHA equal to `HEAD`, installed version equal to `VERSION`, and installed executable hash equal to package hash.
