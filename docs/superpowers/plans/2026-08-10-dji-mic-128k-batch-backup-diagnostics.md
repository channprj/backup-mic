# DJI Mic Backup 128 kbps Batch Backup and Diagnostics Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a run-wide copy-first backup pipeline that converts all new and historical WAV backups to verified AAC-LC 128 kbps M4A, preserves additional session files, records actionable errors, and moves complete verified source sessions to macOS Trash only after final revalidation.

**Architecture:** Keep safety policy in `backup-core`, macOS process and Trash adapters in the Tauri crate, and React limited to narrow commands and snapshots. Replace the current per-file loop with explicit copy and conversion cohorts whose barriers are persisted in SQLite; keep source retirement as a separate fresh revalidation step. Route every command and background failure through a primary destination log plus a privacy-safe fallback log.

**Tech Stack:** Rust 2024, Tauri 2, SQLite/rusqlite, macOS `/usr/bin/afconvert` and `/usr/bin/afinfo`, Foundation `NSFileManager`, React 19, TypeScript, Zod, Vitest, Testing Library, shell acceptance scripts, Headatever.

## Global Constraints

- macOS 13 or newer; release executable must include arm64.
- The selected destination remains `/Users/channprj/Documents/DJI-Mic-Mini-2S` unless changed by the user.
- Automatic backup and `WAV 백업 후 M4A로 변환` default on; automatic Trash defaults off and requires acknowledgement.
- Encoder profile is AAC-LC, requested average bitrate 128,000 bits per second, maximum codec quality, source sample rate, and source channel count.
- Every new WAV is a durable equal-hash destination WAV before any new conversion starts.
- One copy or conversion failure prevents the run-wide barrier and preserves all live source sessions.
- When M4A conversion is off, WAV backup may complete but live source retirement remains unavailable.
- Additional regular session files, including M4A and AppleDouble sidecars, require independent equal-hash backup evidence.
- Production source and superseded destination files move only through macOS Trash; never use permanent unlink, Finder, AppleScript, shell disposal, or direct `.Trashes` manipulation.
- A run captures immutable preferences; saves made during a run apply to the next run.
- Logs never expose absolute source paths, volume UUIDs, full hashes, proposal IDs, raw tool stderr, or audio content.
- Do not hand-edit `VERSION`; initialize and publish it through the bundled Headatever script after source verification and before packaging.
- Follow `$gcpr`: explicit-path staging, Conventional Commit checkpoints, immediate ordinary push, and `0 0` upstream proof after every checkpoint.
- The repository `AGENTS.md` mapping requires inline sequential execution; do not dispatch subagents.

---

### Task 1: Persist every command and background failure

**Files:**
- Create: `src-tauri/src/failure_reporter.rs`
- Create: `src-tauri/tests/failure_reporting.rs`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/src/app_state.rs`
- Modify: `src-tauri/src/commands.rs`
- Modify: `src-tauri/src/orchestrator.rs`
- Modify: `src-tauri/crates/backup-core/src/audit_log.rs`
- Modify: `src-tauri/crates/backup-core/src/error.rs`
- Modify: `src-tauri/crates/backup-core/src/lib.rs`
- Modify: `src-tauri/crates/backup-core/tests/audit_log.rs`

**Interfaces:**
- Consumes: `FileAuditLog`, `AuditEvent`, `AuditValue`, `CoreError::public`, `AppState::destination`.
- Produces:

```rust
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FailureEvent {
    pub operation: &'static str,
    pub stage: &'static str,
    pub transmitter: Option<Transmitter>,
    pub item_name: Option<String>,
    pub error_code: String,
    pub os_kind: Option<String>,
    pub retryable: bool,
}

#[derive(Clone)]
pub struct FailureReporter {
    fallback_root: PathBuf,
}

impl FailureReporter {
    pub fn new(fallback_root: PathBuf) -> Self;
    pub fn report(
        &self,
        primary_destination: Option<&Path>,
        occurred_at: OffsetDateTime,
        event: &FailureEvent,
    ) -> FailureWriteOutcome;
}

impl CoreError {
    pub fn diagnostic_code(&self) -> &'static str;
    pub fn diagnostic_io_kind(&self) -> Option<std::io::ErrorKind>;
}
```

- [ ] **Step 1: Write failing core tests for safe diagnostic classification**

Add literal assertions to `src-tauri/crates/backup-core/tests/audit_log.rs`:

```rust
#[test]
fn core_errors_expose_stable_codes_without_private_source_text() {
    let error = CoreError::CopyFailed(std::io::Error::new(
        std::io::ErrorKind::PermissionDenied,
        "/Volumes/private/TX01/secret.wav",
    ));
    assert_eq!(error.diagnostic_code(), "copy_failed");
    assert_eq!(
        error.diagnostic_io_kind(),
        Some(std::io::ErrorKind::PermissionDenied)
    );
    assert!(!error.diagnostic_code().contains("/Volumes/"));
}
```

- [ ] **Step 2: Write a failing integration test for primary and fallback logs**

Create `src-tauri/tests/failure_reporting.rs` with a destination root made unusable by placing a regular file where a directory is expected. Assert the primary success path creates `logs/2026/08/260810-backup-mic.log`; assert the unavailable-primary path creates `2026/08/260810-backup-mic.log` directly under a temporary fallback root. Read the fallback text and require `operation="set_m4a_conversion"`, `stage="setting_persistence"`, `error_code="ledger_operation_failed"`, and absence of `/Volumes/`, a 64-character hex digest, and raw error prose.

- [ ] **Step 3: Run the tests and verify the intended RED failures**

Run:

```bash
cargo test --manifest-path src-tauri/Cargo.toml -p backup-core --test audit_log core_errors_expose_stable_codes_without_private_source_text
cargo test --manifest-path src-tauri/Cargo.toml -p dji-mic-backup --test failure_reporting
```

Expected: compilation fails because `diagnostic_code`, `diagnostic_io_kind`, `FailureEvent`, and `FailureReporter` do not exist.

- [ ] **Step 4: Implement the reporter and diagnostic mappings**

Use `FileAuditLog` for both roots so escaping, date routing, and durability remain identical. Convert `ErrorKind` to a fixed snake-case name rather than formatting raw `io::Error`. Make `FailureWriteOutcome` distinguish `Primary`, `Fallback`, `BothFailed`, and return no absolute path. In `lib.rs`, resolve the fallback root through Tauri's app log directory during setup and pass it into `AppState::new`.

- [ ] **Step 5: Route top-level failures through one AppState helper**

Add:

```rust
pub fn report_failure(
    &self,
    operation: &'static str,
    stage: &'static str,
    error: &CoreError,
    transmitter: Option<Transmitter>,
    item_name: Option<&str>,
) -> FailureWriteOutcome;
```

Call it before every `set_error` or `set_deletion_error`, in `prepare_trash`, `confirm_trash`, background backup thread completion, rescan failure, device lifecycle failure, destination selection, preference commands, autostart, and log opening. Sanitize `item_name` to one basename of at most 180 Unicode scalar values.

- [ ] **Step 6: Run focused and crate-wide tests**

```bash
cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check
cargo test --manifest-path src-tauri/Cargo.toml -p backup-core --test audit_log
cargo test --manifest-path src-tauri/Cargo.toml -p dji-mic-backup --test failure_reporting
cargo test --manifest-path src-tauri/Cargo.toml -p dji-mic-backup --test commands
cargo clippy --manifest-path src-tauri/Cargo.toml --workspace --all-targets -- -D warnings
git diff --check
```

- [ ] **Step 7: Commit and push the diagnostic checkpoint**

Explicitly stage the Task 1 paths, commit `fix(logging): persist actionable operation failures`, push, fetch, and require `git rev-list --left-right --count HEAD...@{u}` to print `0 0`.

---

### Task 2: Add persisted batch, additional-file, and conversion-cohort evidence

**Files:**
- Create: `src-tauri/crates/backup-core/migrations/0003_batch_manifests.sql`
- Create: `src-tauri/crates/backup-core/src/batch.rs`
- Create: `src-tauri/crates/backup-core/src/additional_file.rs`
- Create: `src-tauri/crates/backup-core/tests/batch_ledger.rs`
- Modify: `src-tauri/crates/backup-core/src/lib.rs`
- Modify: `src-tauri/crates/backup-core/src/ledger.rs`
- Modify: `src-tauri/crates/backup-core/src/preferences.rs`
- Modify: `src-tauri/crates/backup-core/tests/ledger_recovery.rs`
- Modify: `scripts/verify-backup.sh`

**Interfaces:**
- Consumes: `Ledger`, `BackupPreferences`, `VerifiedRecording`, `OutputFormat`, existing v1/v2 migrations.
- Produces:

```rust
pub const M4A_PROFILE_ID: &str = "aac_lc_128k_v1";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BatchPhase {
    Inventory,
    Copying,
    CopiesVerified,
    Converting,
    M4aCohortVerified,
    SourcesRevalidated,
    Completed,
    Failed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FrozenPreferences {
    pub automatic_backup: bool,
    pub m4a_conversion: bool,
    pub automatic_trash: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AdditionalFileClass {
    M4a,
    AppleDouble,
    Other,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifiedAdditionalFile {
    pub id: String,
    pub transmitter: Transmitter,
    pub source_relative_path: PathBuf,
    pub source_size: u64,
    pub source_mtime_ns: i128,
    pub source_sha256: String,
    pub artifact_relative_path: PathBuf,
    pub artifact_size: u64,
    pub artifact_sha256: String,
    pub classification: AdditionalFileClass,
    pub backup_run_id: String,
}
```

- [ ] **Step 1: Write a failing migration and round-trip test**

In `batch_ledger.rs`, open a new ledger, start run `run-1` with literal frozen preferences, advance through `CopiesVerified`, commit one `VerifiedAdditionalFile`, create a two-recording conversion cohort, mark only one item verified, and assert `commit_m4a_barrier("run-1")` returns `CoreError::InvalidRequest`. Mark the second item, commit the barrier, reopen the database, and assert phase `M4aCohortVerified`, profile `aac_lc_128k_v1`, and the exact additional-file record survive.

- [ ] **Step 2: Write a failing upgrade test for the current v2 schema**

Create a v2 database using migrations 0001 and 0002, insert one historical WAV record with `retirement_status='legacy_deleted'`, reopen through `Ledger::open`, and assert `historical_wav_recordings()` returns that record without changing its source or destination hashes.

- [ ] **Step 3: Run RED tests**

```bash
cargo test --manifest-path src-tauri/Cargo.toml -p backup-core --test batch_ledger
cargo test --manifest-path src-tauri/Cargo.toml -p backup-core --test ledger_recovery upgrade_v2_preserves_historical_wav_evidence
```

Expected: failures name the absent migration, batch types, and ledger methods.

- [ ] **Step 4: Add forward-only schema v3**

Add explicit frozen preference and phase columns to `backup_runs`; add nullable superseded-WAV path, size, and SHA-256 columns to `recordings`; create `additional_files`, `conversion_cohort_items`, and `additional_deletion_items` with foreign keys and uniqueness constraints. Record migration version 3 only after the full SQL batch succeeds.

- [ ] **Step 5: Implement narrow ledger transitions**

Add `begin_batch_run`, `advance_batch_phase`, `commit_verified_additional_file`, `verified_additional_file`, `historical_wav_recordings`, `begin_conversion_cohort`, `mark_conversion_item_verified`, and `commit_m4a_barrier`. Every phase transition checks its exact predecessor, and `commit_m4a_barrier` runs one SQL `NOT EXISTS(status != 'verified_m4a')` guard inside the committing transaction.

- [ ] **Step 6: Keep the independent verifier schema-aware**

Extend `verify-backup.sh` to require schema version 3, read additional-file source/artifact evidence, and report counts without printing absolute paths or full hashes unless `--diagnostic` is explicitly passed.

- [ ] **Step 7: Run ledger and verifier tests**

```bash
cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check
cargo test --manifest-path src-tauri/Cargo.toml -p backup-core --test batch_ledger
cargo test --manifest-path src-tauri/Cargo.toml -p backup-core --test ledger_recovery
shellcheck scripts/verify-backup.sh 2>/dev/null || true
git diff --check
```

- [ ] **Step 8: Commit and push the evidence checkpoint**

Explicitly stage the Task 2 paths, commit `feat(core): persist batch backup evidence`, push, fetch, and prove `0 0` parity.

---

### Task 3: Inventory and back up every regular session file

**Files:**
- Create: `src-tauri/crates/backup-core/tests/additional_files.rs`
- Modify: `src-tauri/crates/backup-core/src/scanner.rs`
- Modify: `src-tauri/crates/backup-core/src/recording.rs`
- Modify: `src-tauri/crates/backup-core/src/destination.rs`
- Modify: `src-tauri/crates/backup-core/src/backup.rs`
- Modify: `src-tauri/crates/backup-core/src/recovery.rs`
- Modify: `src-tauri/crates/backup-core/tests/scanner_safety.rs`
- Modify: `src-tauri/crates/backup-core/tests/backup_flow.rs`
- Modify: `src-tauri/src/orchestrator.rs`
- Modify: `src-tauri/tests/rescan_scheduler.rs`

**Interfaces:**
- Consumes: `VerifiedAdditionalFile`, `AdditionalFileClass`, `hash_file`, safe-relative-path checks, no-clobber finalization.
- Produces:

```rust
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdditionalFileObservation {
    pub relative_path: PathBuf,
    pub file_name: String,
    pub size: u64,
    pub modified_nanos: i128,
    pub classification: AdditionalFileClass,
}

pub struct ScanResult {
    pub recordings: Vec<RecordingObservation>,
    pub additional_files: Vec<AdditionalFileObservation>,
    pub issues: Vec<ScanIssue>,
}

pub fn execute_additional_file_copy(
    context: &BackupItemContext<'_>,
    source: &AdditionalFileObservation,
    destination_relative_path: &Path,
    cancellation: &CancellationToken,
) -> Result<VerifiedAdditionalFile, CoreError>;
```

- [ ] **Step 1: Write failing scanner tests for the observed real session shape**

Create a recognized session containing two WAVs, `TX01_MIC001_...m4a`, and `._TX01_MIC001_...m4a`. Assert `scan_once` returns two recordings and two ordered additional files classified as `M4a` and `AppleDouble`. Add a symlink and nested directory case and assert explicit `ScanIssue::UnsafeSessionEntry` rather than silently following or accepting either entry.

- [ ] **Step 2: Write a failing raw-copy behavior test**

Use literal bytes for an external M4A and AppleDouble sidecar. Call the wished-for `execute_additional_file_copy`, assert destination bytes and SHA-256 equal the sources, source files still exist, and both artifacts are under `source-extras/2026/2026-08-10/TX01/TX_MIC001_20260810_001116/`.

- [ ] **Step 3: Run RED tests**

```bash
cargo test --manifest-path src-tauri/Cargo.toml -p backup-core --test additional_files
cargo test --manifest-path src-tauri/Cargo.toml -p backup-core --test scanner_safety
```

Expected: compile failures for the additional-file observation and copy APIs.

- [ ] **Step 4: Extend inventory and rescan fingerprints**

Scan regular files inside recognized sessions without following symlinks. WAVs continue through DJI name parsing; every other regular safe entry becomes an additional observation. Include item kind, relative path, byte count, and modification time in the metadata fingerprint so a new external M4A or sidecar schedules a run.

- [ ] **Step 5: Implement the raw additional-file copy boundary**

Reuse source canonicalization, metadata-before/after checks, part-file ownership, sync, SHA-256, and no-clobber collision suffixes. Return evidence without writing SQLite; the orchestrator performs the short ledger commit from Task 2. Never invoke audio conversion for an additional-source M4A.

- [ ] **Step 6: Integrate additional files before conversion planning**

Have `run_backup` gather every transmitter inventory first, capacity-plan missing WAV and additional copies, copy additional items alongside the WAV copy cohort, then commit their evidence. If an additional item fails, mark the run failed before conversion and leave all source content untouched.

- [ ] **Step 7: Run focused and workspace tests**

```bash
cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check
cargo test --manifest-path src-tauri/Cargo.toml -p backup-core --test additional_files
cargo test --manifest-path src-tauri/Cargo.toml -p backup-core --test scanner_safety
cargo test --manifest-path src-tauri/Cargo.toml -p backup-core --test backup_flow
cargo test --manifest-path src-tauri/Cargo.toml -p dji-mic-backup --test rescan_scheduler
cargo clippy --manifest-path src-tauri/Cargo.toml --workspace --all-targets -- -D warnings
git diff --check
```

- [ ] **Step 8: Commit and push the additional-file checkpoint**

Explicitly stage the Task 3 paths, commit `feat(backup): preserve additional session files`, push, fetch, and prove `0 0` parity.

---

### Task 4: Enforce the copy-all barrier and make settings concurrency-safe

**Files:**
- Create: `src-tauri/crates/backup-core/tests/batch_barrier.rs`
- Create: `src-tauri/tests/concurrent_settings.rs`
- Modify: `src-tauri/crates/backup-core/src/batch.rs`
- Modify: `src-tauri/crates/backup-core/src/backup.rs`
- Modify: `src-tauri/src/orchestrator.rs`
- Modify: `src-tauri/src/artifact_pipeline.rs`
- Modify: `src-tauri/src/app_state.rs`
- Modify: `src-tauri/src/commands.rs`
- Modify: `src-tauri/tests/artifact_pipeline.rs`
- Modify: `src-tauri/tests/commands.rs`

**Interfaces:**
- Consumes: persisted batch run and additional-file evidence, `BackupPreferences`, `FailureReporter`.
- Produces:

```rust
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum BatchItemKey {
    Recording { transmitter: Transmitter, relative_path: PathBuf },
    Additional { transmitter: Transmitter, relative_path: PathBuf },
}

pub struct CopyBarrier {
    expected: BTreeSet<BatchItemKey>,
    verified: BTreeSet<BatchItemKey>,
    failed: BTreeSet<BatchItemKey>,
}

impl CopyBarrier {
    pub fn new(expected: BTreeSet<BatchItemKey>) -> Result<Self, CoreError>;
    pub fn record_verified(&mut self, key: &BatchItemKey) -> Result<(), CoreError>;
    pub fn record_failed(&mut self, key: &BatchItemKey) -> Result<(), CoreError>;
    pub fn conversion_allowed(&self) -> bool;
}
```

- [ ] **Step 1: Write failing barrier tests**

Create three literal keys across TX01 and TX02. Assert `conversion_allowed()` is false at construction, remains false after two verifications, becomes true only after the third, and can never become true after any key is failed. Assert duplicate and unknown-key transitions return `InvalidRequest`.

- [ ] **Step 2: Write a failing setting-during-backup integration test**

Create an `AppState` with a test ledger and a held operation whose copy observer blocks on a channel. Invoke the real async `set_m4a_conversion_for_state(false)` while the operation is blocked. Require the save future to complete within two seconds, persisted value `false`, runtime setting `false`, frozen run preference `true`, and a `setting.saved` log event. Release the operation and assert its outcome used the original `true` snapshot.

- [ ] **Step 3: Run RED tests**

```bash
cargo test --manifest-path src-tauri/Cargo.toml -p backup-core --test batch_barrier
cargo test --manifest-path src-tauri/Cargo.toml -p dji-mic-backup --test concurrent_settings
```

Expected: absent barrier and async setting helpers cause compilation failure.

- [ ] **Step 4: Split file work from ledger transactions**

Refactor WAV copy and M4A preparation functions to return verified values before they lock the ledger. In the orchestrator, hold `state.ledger` only for `begin_batch_run`, existing-evidence reads, evidence commits, phase transitions, and final outcome writes. Do not hold it while reading or hashing audio, sleeping for stability, invoking Apple tools, or calling Trash.

- [ ] **Step 5: Reorder production orchestration around CopyBarrier**

Build the full expected key set after all scans. Execute every missing copy and reverify every reused artifact, recording success or failure in `CopyBarrier`. Call no `AudioTools::convert` until `conversion_allowed()` is true and the ledger phase transaction commits `CopiesVerified`.

- [ ] **Step 6: Freeze settings and prohibit retirement in WAV-only mode**

Capture `FrozenPreferences` at `begin_batch_run`. If `m4a_conversion` is false, finish with `wav_backup_complete_source_retained`, clear all runtime deletion candidates, set `deletion_ready=false`, and log the retained-source reason. A setting change during the run updates only the next-run runtime preference.

- [ ] **Step 7: Convert setting commands to nonblocking async adapters**

Move SQLite work to `tauri::async_runtime::spawn_blocking`, read the setting back before updating runtime, then append `setting.saved`. On failure call `report_failure` with operation and `setting_persistence` stage before returning the original structured error. Keep automatic-Trash acknowledgement validation before any write.

- [ ] **Step 8: Run focused and broad tests**

```bash
cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check
cargo test --manifest-path src-tauri/Cargo.toml -p backup-core --test batch_barrier
cargo test --manifest-path src-tauri/Cargo.toml -p dji-mic-backup --test concurrent_settings
cargo test --manifest-path src-tauri/Cargo.toml -p dji-mic-backup --test commands
cargo test --manifest-path src-tauri/Cargo.toml -p dji-mic-backup --test artifact_pipeline
cargo clippy --manifest-path src-tauri/Cargo.toml --workspace --all-targets -- -D warnings
git diff --check
```

- [ ] **Step 9: Commit and push the copy-barrier checkpoint**

Explicitly stage the Task 4 paths, commit `fix(backup): enforce copy-first batch safety`, push, fetch, and prove `0 0` parity.

---

### Task 5: Convert the complete WAV cohort to verified 128 kbps M4A

**Files:**
- Modify: `src-tauri/src/platform/macos/audio.rs`
- Modify: `src-tauri/src/artifact_pipeline.rs`
- Modify: `src-tauri/src/orchestrator.rs`
- Modify: `src-tauri/crates/backup-core/src/artifact.rs`
- Modify: `src-tauri/crates/backup-core/src/batch.rs`
- Modify: `src-tauri/crates/backup-core/src/ledger.rs`
- Modify: `src-tauri/tests/apple_audio_tools.rs`
- Modify: `src-tauri/tests/artifact_pipeline.rs`
- Create: `src-tauri/tests/conversion_cohort.rs`
- Modify: `src-tauri/crates/backup-core/tests/batch_ledger.rs`
- Modify: `scripts/check.sh`

**Interfaces:**
- Consumes: `CopyBarrier`, conversion-cohort ledger rows, historical WAV query, `MacTrash`.
- Produces:

```rust
pub const M4A_TARGET_BITRATE_BPS: u32 = 128_000;

pub trait AudioTools: Send + Sync {
    fn inspect(&self, path: &Path) -> Result<AudioDescription, CoreError>;
    fn convert_aac_lc_128k(&self, input: &Path, output: &Path)
        -> Result<(), CoreError>;
}

pub struct PreparedM4aArtifact {
    pub recording: VerifiedRecording,
    pub superseded_wav_relative_path: PathBuf,
    pub superseded_wav_size: u64,
    pub superseded_wav_sha256: String,
}
```

- [ ] **Step 1: Write a failing command-profile test**

Extract a pure `afconvert_arguments(input, output)` helper and assert its `OsString` vector equals:

```rust
[
    input.as_os_str(), "-o", output.as_os_str(), "-f", "m4af",
    "-d", "aac", "-b", "128000", "-q", "127", "-s", "2",
]
```

Also assert no argument is `-c`, no shell path appears, and the old string `192000` is absent.

- [ ] **Step 2: Write a failing cohort-order integration test**

Use three verified WAV fixtures, two current and one historical. A fake `AudioTools` records conversion calls and fails the final one. Assert all three WAV evidence rows exist before the first conversion call, no M4A barrier is committed, no superseded WAV reaches fake Trash, and every live source remains. In the success case assert conversion order is deterministic, all three items reach `verified_m4a`, then and only then the barrier commits.

- [ ] **Step 3: Run RED tests**

```bash
cargo test --manifest-path src-tauri/Cargo.toml -p dji-mic-backup --test apple_audio_tools
cargo test --manifest-path src-tauri/Cargo.toml -p dji-mic-backup --test conversion_cohort
```

Expected: failure names the old 192 kbps API and missing cohort orchestrator.

- [ ] **Step 4: Implement the 128 kbps Apple adapter**

Rename the trait method, use `M4A_TARGET_BITRATE_BPS.to_string()` as the exact `-b` argument, preserve direct `Command::new("/usr/bin/afconvert")`, and keep bounded output and structured `afinfo -x` validation. Do not accept output solely because the process exited successfully.

- [ ] **Step 5: Make M4A preparation ledger-independent**

Have the artifact pipeline return `PreparedM4aArtifact` after part-file conversion, inspection, hashing, finalization, and directory sync. The orchestrator then takes a short ledger lock to replace the artifact and mark the cohort item. Persist superseded WAV evidence so restart cleanup never derives it only from an extension.

- [ ] **Step 6: Build and commit the complete cohort**

Create the cohort from all current WAV artifacts plus `historical_wav_recordings()`. Reverify each input SHA-256. Process every item deterministically. If any item fails, finish the run as failed and preserve all destination WAVs and live sources. If all succeed, call `commit_m4a_barrier`, append and sync `backup.m4a_cohort_verified`, then move superseded destination WAVs to macOS Trash. A destination-WAV Trash failure logs a warning and withholds automatic live-source retirement.

- [ ] **Step 7: Run production Apple integration and full Rust gates**

```bash
cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check
cargo test --manifest-path src-tauri/Cargo.toml -p dji-mic-backup --test apple_audio_tools
cargo test --manifest-path src-tauri/Cargo.toml -p dji-mic-backup --test artifact_pipeline
cargo test --manifest-path src-tauri/Cargo.toml -p dji-mic-backup --test conversion_cohort
cargo test --manifest-path src-tauri/Cargo.toml --workspace --all-targets
cargo clippy --manifest-path src-tauri/Cargo.toml --workspace --all-targets -- -D warnings
git diff --check
```

- [ ] **Step 8: Commit and push the conversion checkpoint**

Explicitly stage the Task 5 paths, commit `feat(backup): convert complete cohorts at 128k`, push, fetch, and prove `0 0` parity.

---

### Task 6: Revalidate and Trash complete sessions only after the run-wide barrier

**Files:**
- Create: `src-tauri/crates/backup-core/tests/run_retirement.rs`
- Modify: `src-tauri/crates/backup-core/src/deletion.rs`
- Modify: `src-tauri/crates/backup-core/src/ledger.rs`
- Modify: `src-tauri/crates/backup-core/src/state.rs`
- Modify: `src-tauri/src/orchestrator.rs`
- Modify: `src-tauri/src/app_state.rs`
- Modify: `src-tauri/tests/macos_trash.rs`
- Modify: `src-tauri/tests/fat32_trash_acceptance.rs`
- Modify: `src-tauri/crates/backup-core/tests/fat32_deletion_acceptance.rs`
- Modify: `src-tauri/crates/backup-core/tests/deletion_guard.rs`

**Interfaces:**
- Consumes: `VerifiedAdditionalFile`, run phase `M4aCohortVerified`, `MacTrash`, deletion proposal TTL.
- Produces:

```rust
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdditionalDeletionCandidate {
    pub additional_file_id: String,
    pub source_relative_path: PathBuf,
    pub source_size: u64,
    pub source_mtime_ns: i128,
    pub source_sha256: String,
    pub destination_relative_path: PathBuf,
    pub destination_size: u64,
    pub destination_sha256: String,
}

pub struct CompleteDeletionSnapshot {
    pub context: DeletionContext,
    pub recordings: Vec<DeletionCandidate>,
    pub additional_files: Vec<AdditionalDeletionCandidate>,
    pub current_source_paths: BTreeSet<PathBuf>,
    pub m4a_barrier_run_id: String,
}
```

- [ ] **Step 1: Write the exact reported-session regression test**

Create a recognized session with two WAV candidates, one backed external M4A candidate, and its backed AppleDouble candidate. Commit the M4A barrier and prepare/confirm retirement through a fake Trash that renames the directory. Assert one session target, four proposed files, source directory absent, destination M4As and raw extras present, and ledger outcomes complete.

- [ ] **Step 2: Write fail-closed mutation tests**

From the same fixture, separately add an unbacked file, modify the external M4A, replace the sidecar with a symlink, change the destination M4A, remove the run barrier, and set conversion preference false. Each case must return `DeletionPreflightRefused`, make zero Trash calls, and preserve the complete source directory.

- [ ] **Step 3: Write an empty legacy session test**

Create one exact empty `TX_MIC001_20260810_001116` with matching ledger evidence and one empty look-alike without evidence. Assert only the exact ledger-backed directory is passed to Trash and no recursive or permanent delete API is invoked.

- [ ] **Step 4: Run RED tests**

```bash
cargo test --manifest-path src-tauri/Cargo.toml -p backup-core --test run_retirement
cargo test --manifest-path src-tauri/Cargo.toml -p backup-core --test deletion_guard
```

Expected: missing additional candidates and barrier evidence prevent compilation.

- [ ] **Step 5: Extend complete-snapshot verification**

Compare the fresh scanner path set with the union of recording and additional-file candidates. Recheck safe relative paths, metadata, source hashes, destination hashes, WAV-to-M4A audio evidence, paired identity, mount and scan generations, destination generation, ledger health, primary log durability, and committed M4A barrier before constructing any `RetirementTarget`.

- [ ] **Step 6: Extend deletion ledger outcomes without weakening legacy rows**

Record additional-file outcomes in `additional_deletion_items` and count them in proposal summaries. Group all candidates under a recognized top-level session into one directory target. Keep root WAV files as individual targets; keep additional files outside a recognized session ineligible.

- [ ] **Step 7: Integrate manual and automatic paths**

Populate runtime deletion candidates only after source revalidation. If automatic Trash is off, expose the five-minute proposal. If on, call the same prepare and confirm logic. If conversion is off or any run-wide precondition fails, clear deletion readiness and log the exact refusal.

- [ ] **Step 8: Run safety and FAT32 tests**

```bash
cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check
cargo test --manifest-path src-tauri/Cargo.toml -p backup-core --test run_retirement
cargo test --manifest-path src-tauri/Cargo.toml -p backup-core --test deletion_guard
cargo test --manifest-path src-tauri/Cargo.toml -p dji-mic-backup --test macos_trash
cargo test --manifest-path src-tauri/Cargo.toml --workspace --all-targets
cargo clippy --manifest-path src-tauri/Cargo.toml --workspace --all-targets -- -D warnings
git diff --check
```

Run the real FAT32 acceptance only through Task 8's isolated harness; do not point these tests at a production transmitter.

- [ ] **Step 9: Commit and push the retirement checkpoint**

Explicitly stage the Task 6 paths, commit `fix(safety): revalidate complete sessions before trash`, push, fetch, and prove `0 0` parity.

---

### Task 7: Show batch stages, specific errors, and next-run setting behavior

**Files:**
- Modify: `src-tauri/crates/backup-core/src/state.rs`
- Modify: `src-tauri/src/dto.rs`
- Modify: `src-tauri/src/app_state.rs`
- Modify: `src/features/backup/contracts.ts`
- Modify: `src/features/backup/format.ts`
- Modify: `src/features/backup/BackupPopover.tsx`
- Modify: `src/features/backup/SettingsApp.tsx`
- Modify: `src/features/backup/useBackupSnapshot.ts`
- Modify: `src/features/backup/__tests__/contracts.test.ts`
- Modify: `src/features/backup/__tests__/BackupPopover.test.tsx`
- Modify: `src/features/backup/__tests__/SettingsApp.test.tsx`
- Modify: `src/features/backup/__tests__/useBackupSnapshot.test.tsx`
- Modify: `contracts/README.md`
- Modify: `contracts/fixtures/backup-complete.json`
- Modify: `contracts/fixtures/backup-copying.json`
- Modify: `contracts/fixtures/error-destination-full.json`
- Modify: `src/preview.tsx`
- Modify: `src/index.css`

**Interfaces:**
- Consumes: batch phases, `PublicError.message_code`, frozen preferences, current run state.
- Produces:

```ts
type CurrentStage =
  | "copy"
  | "source_verification"
  | "conversion"
  | "artifact_verification"
  | "source_revalidation"
  | "trash";

type AppSnapshot = {
  // existing strict fields
  setting_applies_next_run: boolean;
  failure_stage: CurrentStage | null;
};
```

- [ ] **Step 1: Write failing strict-contract tests**

Require `source_revalidation`, `setting_applies_next_run`, and `failure_stage` in the Rust fixtures and Zod schema. Assert unknown fields still fail `.strict()` and snapshots still exclude paths, UUIDs, hashes, raw tool output, and proposal internals.

- [ ] **Step 2: Write failing UI behavior tests**

Assert the visible ordered sequence is exactly `전체 WAV 복사WAV 검증128kbps M4A 변환전체 M4A 검증원본 재검증휴지통 이동`. Assert the M4A switch label is `WAV 백업 후 M4A로 변환`, its description contains `AAC-LC 128kbps` and `백업 폴더`, and a disabled conversion snapshot renders `외장 디스크 원본을 유지합니다`.

For `{ message_code: "session_contains_unverified_file", failure_stage: "source_revalidation" }`, require a specific title, safe support code, and enabled `로그 열기` button. For a failed setting command followed by a successful retry, require the alert to disappear and the persisted switch value to remain.

- [ ] **Step 3: Run RED frontend tests**

```bash
pnpm test -- src/features/backup/__tests__/contracts.test.ts src/features/backup/__tests__/BackupPopover.test.tsx src/features/backup/__tests__/SettingsApp.test.tsx
```

Expected: schema and visible-copy assertions fail against the current five-stage 192 kbps UI.

- [ ] **Step 4: Extend Rust and TypeScript snapshots together**

Add `CurrentStage::SourceRevalidation`, serialize it as `source_revalidation`, and populate `setting_applies_next_run` when an operation is active. Preserve the failed stage instead of clearing all stage evidence before publishing the error snapshot.

- [ ] **Step 5: Implement exact Korean recovery copy**

Add mappings for copy, ledger, audit log, Apple conversion, artifact validation, session extra, proposal, setting persistence, and Trash failures. Render `오류 코드: <message_code>` and `로그 열기`; never render a full path or hash. Clear local action errors when the next command returns successfully.

- [ ] **Step 6: Render the new batch workflow and setting semantics**

Update popover steps and completion states. Rename and explain the conversion setting, show `다음 백업부터 적용됩니다` during active work, and state that conversion-off keeps originals. Keep automatic Trash acknowledgement and default-off behavior unchanged.

- [ ] **Step 7: Run frontend, contract, and Rust IPC gates**

```bash
pnpm test
pnpm typecheck
pnpm build
cargo test --manifest-path src-tauri/Cargo.toml -p dji-mic-backup --test ipc_contract
cargo test --manifest-path src-tauri/Cargo.toml -p dji-mic-backup --test commands
git diff --check
```

- [ ] **Step 8: Commit and push the UI checkpoint**

Explicitly stage the Task 7 paths, commit `feat(ui): explain batch backup and recovery failures`, push, fetch, and prove `0 0` parity.

---

### Task 8: Extend independent verification, FAT32 acceptance, and operating docs

**Files:**
- Modify: `scripts/verify-backup.sh`
- Modify: `scripts/accept-deletion-fixture.sh`
- Modify: `scripts/check.sh`
- Modify: `scripts/package-local.sh`
- Modify: `scripts/install-local.sh`
- Modify: `src-tauri/tests/fat32_trash_acceptance.rs`
- Modify: `src-tauri/crates/backup-core/tests/fat32_deletion_acceptance.rs`
- Modify: `README.md`
- Modify: `docs/superpowers/specs/2026-08-10-dji-mic-128k-batch-backup-diagnostics-design.md`

**Interfaces:**
- Consumes: schema v3, `aac_lc_128k_v1`, primary/fallback log paths, current Foundation Trash adapter.
- Produces: independent verifier coverage for live WAV, final M4A, raw extras, barriers, and versioned packages.

- [ ] **Step 1: Add a failing verifier fixture**

Create a temporary destination ledger with two live WAV recordings, verified M4As, one external source M4A raw copy, one AppleDouble raw copy, and a committed run barrier. Run `verify-backup.sh` and assert success. Remove the AppleDouble artifact, change one final M4A hash, and remove the barrier in separate cases; require a nonzero exit and privacy-safe count-based errors.

- [ ] **Step 2: Extend the disposable FAT32 fixture**

Generate at least two deterministic PCM WAVs, one independent M4A, and one AppleDouble sidecar inside one exact session on `/Volumes/DJI-DELTEST`. Keep the sentinel, removable/external/FAT32 identity checks, 64 MiB disposable image, exact mount point, and teardown trap. Do not remove the sidecar in fixture setup.

- [ ] **Step 3: Make the FAT32 acceptance prove the full pipeline**

Run production inventory and policy with a fixture destination. Assert all WAV copies are verified before conversion begins, M4As pass production Apple inspection, raw extras have equal hashes, the whole session leaves its original location through Foundation Trash, and destination artifacts remain. Keep direct `.Trashes` inspection outside production code and only where the disposable acceptance needs recovery proof.

- [ ] **Step 4: Strengthen static and behavioral gates**

Keep the production permanent-delete scan. Add behavioral tests to `check.sh` for the exact 128 kbps profile, copy barrier, conversion-disabled source retention, fallback logging, and concurrent settings. Avoid source-grep assertions for behavior already exercised by tests.

- [ ] **Step 5: Update packaging and installation version checks**

Make `package-local.sh` and `install-local.sh` accept the expected Headatever version as an argument or read the validated `VERSION` file after Task 9 creates it. Verify `CFBundleShortVersionString` in addition to bundle ID, macOS floor, arm64, signature, and executable hash. Preserve rollback and move the prior app to Trash.

- [ ] **Step 6: Update README and close the implemented spec**

Document the exact order, 128 kbps profile, historical migration, additional-file backup, conversion-off source retention, run-wide failure behavior, primary and fallback log locations, manual/automatic Trash behavior, Headatever release order, verifier syntax, and disposable-volume acceptance. Change the design status to `Implemented and locally installed` only after Task 9 proves that state.

- [ ] **Step 7: Run source and FAT32 gates**

```bash
./scripts/check.sh
./scripts/accept-deletion-fixture.sh
git diff --check
```

Require every Rust, Clippy, frontend, typecheck, production build, verifier fixture, and FAT32 acceptance result to pass without skipped relevant tests.

- [ ] **Step 8: Commit and push the verification checkpoint**

Explicitly stage the Task 8 paths except the design-status line if installation is not yet proven. Commit `test(release): verify batch backup and recoverable cleanup`, push, fetch, and prove `0 0` parity. Defer the final implemented-status line to Task 9.

---

### Task 9: Initialize Headatever, package, reinstall, run, and audit completion

**Files:**
- Create through Headatever only: `VERSION`
- Modify before Headatever: `package.json`
- Modify before Headatever: `pnpm-lock.yaml`
- Modify before Headatever: `src-tauri/Cargo.toml`
- Modify before Headatever: `src-tauri/Cargo.lock`
- Modify before Headatever: `src-tauri/crates/backup-core/Cargo.toml`
- Modify before Headatever: `src-tauri/tauri.conf.json`
- Modify before Headatever: `src/features/backup/SettingsApp.tsx`
- Modify after installed proof: `docs/superpowers/specs/2026-08-10-dji-mic-128k-batch-backup-diagnostics-design.md`
- Modify after installed proof: `docs/superpowers/plans/2026-08-10-dji-mic-128k-batch-backup-diagnostics.md`

**Interfaces:**
- Consumes: bundled `/Users/channprj/.agents/skills/headatever/scripts/headatever.sh`, package/install scripts, complete verification suite.
- Produces: release version, annotated tag, verified app/DMG, replaced local installation, running app, final Git and tag parity.

- [ ] **Step 1: Run the final source audit before versioning**

```bash
./scripts/check.sh
./scripts/accept-deletion-fixture.sh
git status --short
git rev-list --left-right --count HEAD...@{u}
```

Require a clean tree and `0 0`. Re-read every acceptance criterion in the approved design and record the exact test or runtime evidence that covers it.

- [ ] **Step 2: Dry-run Headatever and compute the only release version**

```bash
/Users/channprj/.agents/skills/headatever/scripts/headatever.sh init 0 --dry-run
```

On 2026-08-10 require `0.260810.0`. If the local date changed, use the script's actual result consistently and do not force the old date.

- [ ] **Step 3: Synchronize release metadata without creating VERSION**

Update package, lockfile, both Cargo package versions, Tauri version, and visible settings version to the dry-run value. Run:

```bash
pnpm install --lockfile-only
cargo check --manifest-path src-tauri/Cargo.toml --workspace
./scripts/check.sh
git diff --check
```

Explicitly stage only the metadata paths, commit `build(release): prepare v0.260810.0`, push, fetch, and prove `0 0`.

- [ ] **Step 4: Let Headatever create VERSION, commit, tag, and push**

```bash
/Users/channprj/.agents/skills/headatever/scripts/headatever.sh init 0 --push
```

Do not create or edit `VERSION` through any other command. Require subject `chore(release): v0.260810.0`, annotated tag type `tag`, and local tag target equal to `HEAD`.

- [ ] **Step 5: Prove branch and tag publication**

```bash
git fetch origin main --tags
git rev-list --left-right --count HEAD...@{u}
git rev-list --left-right --count HEAD...origin/main
git ls-remote --exit-code --tags origin refs/tags/v0.260810.0
git cat-file -t v0.260810.0
git rev-parse v0.260810.0^{}
git rev-parse HEAD
```

Require both counts `0 0`, remote tag presence, local object type `tag`, and equal peeled-tag/HEAD commits.

- [ ] **Step 6: Build and verify release artifacts**

```bash
./scripts/package-local.sh
```

Require a deep-strict valid app, verified DMG, bundle ID `com.channprj.DJIMicBackup`, minimum macOS `13.0`, arm64, version `0.260810.0`, and printed app executable and DMG SHA-256 values.

- [ ] **Step 7: Install with rollback and relaunch**

```bash
./scripts/install-local.sh 'src-tauri/target/release/bundle/macos/DJI Mic Backup.app'
open '/Users/channprj/Applications/DJI Mic Backup.app'
```

Require the prior app to move to macOS Trash, the new app to pass the installer verification, and one exact installed process path `/Users/channprj/Applications/DJI Mic Backup.app/Contents/MacOS/dji-mic-backup`.

- [ ] **Step 8: Prove installed runtime behavior without automatic source retirement**

Verify installed/release executable hashes match, strict code signing passes, version metadata matches, the status item exists, automatic Trash remains off unless previously acknowledged, settings persist across restart, and startup/scan events append to the primary daily log. During a controlled slow read-only backup, toggle M4A off and back on; require both saves to succeed, apply to the next run, and produce `setting.saved` without a generic error.

If real live WAVs exist, keep automatic Trash off and use the independent verifier to prove copy-all then 128 kbps M4A conversion. If no live WAV exists, report that hardware production conversion could not be exercised and rely only on the production Apple integration plus disposable FAT32 proof; do not manufacture or write a fixture onto a production transmitter.

- [ ] **Step 9: Close documentation and push the final build checkpoint**

Mark the approved design and this plan implemented only after Steps 6-8 pass. Explicitly stage those two docs and any release-script evidence changes, commit `build: install the verified 128k backup release`, push, fetch, and prove `0 0` branch parity. Do not move or recreate the already published Headatever tag.

- [ ] **Step 10: Run the completion audit**

Re-read the complete user request and approved design. Confirm source order, historical conversion, 128 kbps profile, additional-file backup, error logs, setting persistence, source revalidation, whole-session and empty-folder Trash, conversion-off retention, version, build, installation, runtime, clean tree, checkpoint history, upstream parity, and remote tag evidence. Only then report completion.

---

## Execution Order and Checkpoints

Execute Tasks 1 through 9 sequentially because later retirement authority depends on earlier evidence. Each task must finish green, be explicitly staged, committed, pushed, and proven `0 0` before the next task starts. Never publish a half migration, producer without its consumer, or known failing intermediate state.

Expected implementation checkpoints after this plan:

1. `fix(logging): persist actionable operation failures`
2. `feat(core): persist batch backup evidence`
3. `feat(backup): preserve additional session files`
4. `fix(backup): enforce copy-first batch safety`
5. `feat(backup): convert complete cohorts at 128k`
6. `fix(safety): revalidate complete sessions before trash`
7. `feat(ui): explain batch backup and recovery failures`
8. `test(release): verify batch backup and recoverable cleanup`
9. `build(release): prepare v0.260810.0`
10. `chore(release): v0.260810.0` created and tagged only by Headatever
11. `build: install the verified 128k backup release`
