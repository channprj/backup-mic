# DJI Mic Backup M4A, Trash, Logging, and Settings Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make mounted transmitters rescan automatically, produce verified AAC-LC M4A backups by default, record a durable daily audit log, move complete verified recording sessions to macOS Trash, expose the safety controls in a native-feeling settings window, and rebuild and reinstall the verified app locally.

**Architecture:** Keep policy in `backup-core`, keep macOS process and Foundation integrations in the Tauri adapter, and keep React limited to typed snapshots and narrow commands. Extend the existing copy-first SHA-256 boundary with a verified artifact model, use SQLite plus an append-only destination log as retirement authority, and make manual and automatic cleanup converge on one complete-session revalidation pipeline.

**Tech Stack:** Rust 2024 workspace, Tauri 2, React 19, TypeScript 5, Vite, Vitest, SQLite, `/usr/bin/afconvert`, `/usr/bin/afinfo -x`, Foundation `NSFileManager`, `objc2` 0.6, `objc2-foundation` 0.3, `quick-xml` 0.41, pnpm, macOS 13+.

## Global Constraints

- Treat `/Volumes/DJI-MIC-1` and `/Volumes/DJI-MIC-2` as production data. Never use them for destructive acceptance tests; only move items from them after the complete live revalidation contract passes.
- Production source retirement must call Foundation `trashItemAtURL:resultingItemURL:error:`. It must not call `remove_file`, `remove_dir`, `rm`, Finder, AppleScript, or manipulate `.Trashes` directly.
- A recognized session is anchored by `^TX_MIC[0-9]+_[0-9]{8}_[0-9]{6}$`. Trash the directory as one item only when its complete recursive inventory contains exactly the currently verified regular WAV candidates and no hidden entry, symlink, or nested directory.
- Root-level verified WAVs may be moved to Trash individually. Any ambiguous or changed entry refuses the affected transmitter before the first move.
- Automatic Trash movement is persisted, defaults to off, and requires explicit acknowledgement when enabled. Manual and automatic paths use the same revalidation and audit requirements.
- Automatic backup and M4A conversion are persisted and default to on. A trusted mounted transmitter receives a metadata-only rescan every 15 seconds; an unchanged fingerprint creates no run and no activity entry.
- The M4A profile is fixed to AAC-LC, 192 kbps constrained VBR, source sample rate and channel count. Invoke Apple tools with `Command` argument arrays, never through a shell.
- M4A is eligible only after source/staging WAV size and SHA-256 equality, `afinfo -x` validation, final artifact SHA-256, atomic finalization, directory synchronization, SQLite commit, and required audit-log flush.
- The daily log path is exactly `logs/YYYY/MM/YYMMDD-backup-mic.log` using local time. Log fields must not expose absolute source paths, device UUIDs, full hashes, opaque proposal IDs, or audio content.
- Preserve the 10 GiB destination reserve and budget simultaneous staging WAV plus conversion output.
- Preserve paired-device identity, canonical-path, mount-generation, scan-generation, operation-guard, no-clobber, and source-change checks from the baseline design.
- Keep IPC narrow and typed. Do not expose a general settings key/value command, raw filesystem path mutation, general process execution, or arbitrary Trash path.
- Use test-driven implementation: add a focused failing test, observe the expected failure, implement the smallest complete behavior, rerun the focused test, then rerun the relevant suite.
- Use `$gcpr` outcome checkpoints. Explicitly stage only files for the checkpoint, create a Conventional Commit, push immediately, and prove local/tracking/live-remote parity is `0 0` after every push.
- Preserve unrelated user-owned and untracked files.

---

## Source Contract and File Map

- Approved design: `docs/superpowers/specs/2026-08-09-dji-mic-m4a-trash-settings-design.md`
- Core policy: `src-tauri/crates/backup-core/src/`
- SQLite migrations: `src-tauri/crates/backup-core/migrations/`
- Tauri orchestration and IPC: `src-tauri/src/`
- macOS adapters: `src-tauri/src/platform/macos/`
- React feature: `src/features/backup/`
- Styling and window routing: `src/index.css`, `src/App.tsx`
- Packaging and acceptance: `scripts/`, `README.md`

The implementation keeps legacy database column and table names readable for forward migration, but all new product copy and APIs call the operation “Trash” or “source retirement.”

---

### Task 1: Persist artifact metadata and typed automation preferences

**Files:**
- Create: `src-tauri/crates/backup-core/migrations/0002_artifacts_and_preferences.sql`
- Create: `src-tauri/crates/backup-core/src/artifact.rs`
- Create: `src-tauri/crates/backup-core/src/preferences.rs`
- Modify: `src-tauri/crates/backup-core/src/lib.rs`
- Modify: `src-tauri/crates/backup-core/src/ledger.rs`
- Modify: `src-tauri/crates/backup-core/src/state.rs`
- Modify: `src-tauri/crates/backup-core/src/error.rs`
- Modify: `src-tauri/crates/backup-core/tests/ledger_recovery.rs`

**Interfaces:**

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OutputFormat { Wav, M4a }

pub struct VerifiedArtifact {
    pub relative_path: PathBuf,
    pub format: OutputFormat,
    pub byte_count: u64,
    pub sha256: String,
    pub audio: Option<VerifiedAudioProperties>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PreferenceKey { AutomaticBackup, M4aConversion, AutomaticTrash }

pub struct BackupPreferences {
    pub automatic_backup: bool,
    pub m4a_conversion: bool,
    pub automatic_trash: bool,
}
```

- [x] **Step 1: Add migration-first ledger tests**

Add tests proving a database created from `0001_initial.sql` upgrades without losing existing recordings; historical rows read as `OutputFormat::Wav`; missing preferences yield `{ automatic_backup: true, m4a_conversion: true, automatic_trash: false }`; and typed preference writes survive a new `Ledger` instance.

- [x] **Step 2: Run the focused tests and observe the missing schema/model failure**

Run:

```bash
cargo test -p backup-core --test ledger_recovery artifact_and_preferences
```

Expected: compilation fails because the artifact and preference APIs do not exist.

- [x] **Step 3: Add the forward-only schema migration**

Extend `recordings` with artifact format, codec, sample rate, channel count, valid frames, duration microseconds, conversion status, conversion error code, retirement status, and retired-session relative path. Preserve existing destination path, byte-count, and SHA columns as the finalized artifact fields so historical data remains readable. Add `CHECK` constraints for `wav|m4a`, known conversion states, and known retirement states. Do not rewrite historical hashes.

- [x] **Step 4: Add typed preference persistence**

Use the existing settings table with stable keys `automatic_backup`, `m4a_conversion`, and `automatic_trash`. Implement only `read_preferences()` and `set_preference(PreferenceKey, bool, occurred_at)`; reject arbitrary public keys. Decode missing values through the documented defaults and return a typed error for malformed stored JSON.

- [x] **Step 5: Generalize verified recording reads and writes**

Replace destination-only construction with `VerifiedArtifact`. Keep the verified source WAV byte count and SHA-256 separate from the artifact byte count and SHA-256. Update eligibility queries so an M4A row is eligible only when conversion is complete and all required audio fields are present.

- [x] **Step 6: Run format, focused tests, and the core suite**

```bash
cargo fmt --all -- --check
cargo test -p backup-core --test ledger_recovery
cargo test -p backup-core
```

Expected: all pass; a migrated legacy fixture remains readable and default settings match the approved contract.

- [x] **Step 7: Commit and push the persistence checkpoint**

```bash
git add src-tauri/crates/backup-core/migrations/0002_artifacts_and_preferences.sql \
  src-tauri/crates/backup-core/src/artifact.rs \
  src-tauri/crates/backup-core/src/preferences.rs \
  src-tauri/crates/backup-core/src/lib.rs \
  src-tauri/crates/backup-core/src/ledger.rs \
  src-tauri/crates/backup-core/src/state.rs \
  src-tauri/crates/backup-core/src/error.rs \
  src-tauri/crates/backup-core/tests/ledger_recovery.rs
git commit -m "feat(core): persist backup artifacts and automation settings"
git push
git fetch origin main
git rev-list --left-right --count HEAD...origin/main
git rev-list --left-right --count HEAD...refs/remotes/origin/main
```

Expected: both parity commands print `0 0`.

---

### Task 2: Write a privacy-safe durable daily audit log

**Files:**
- Create: `src-tauri/crates/backup-core/src/audit_log.rs`
- Create: `src-tauri/crates/backup-core/tests/audit_log.rs`
- Modify: `src-tauri/crates/backup-core/src/lib.rs`
- Modify: `src-tauri/crates/backup-core/src/error.rs`
- Modify: `src-tauri/src/app_state.rs`
- Modify: `src-tauri/src/orchestrator.rs`

**Interfaces:**

```rust
pub enum AuditDurability { Buffered, SyncData }
pub enum AuditValue<'a> { Text(&'a str), Unsigned(u64), Boolean(bool) }

pub struct AuditEvent<'a> {
    pub occurred_at: OffsetDateTime,
    pub level: AuditLevel,
    pub code: &'static str,
    pub transmitter: Option<Transmitter>,
    pub fields: &'a [(&'static str, AuditValue<'a>)],
}

pub trait AuditSink: Send + Sync {
    fn append(&self, event: &AuditEvent<'_>, durability: AuditDurability)
        -> Result<PathBuf, CoreError>;
}
```

- [x] **Step 1: Add failing path, escaping, privacy, and durability tests**

Cover the KST instant `2026-08-09T20:01:42.613+09:00` mapping to `logs/2026/08/260809-backup-mic.log`; quote and backslash escaping; CR/LF normalization; rejection of unknown field names; absence of absolute source paths, UUID keys, full hashes, and proposal identifiers; concurrent append serialization; and `SyncData` propagation.

- [x] **Step 2: Run the audit-log test and observe the missing module failure**

```bash
cargo test -p backup-core --test audit_log
```

- [x] **Step 3: Implement `FileAuditLog`**

Create parent directories lazily, open the dated file in append mode, serialize one event under one process-wide mutex, encode timestamps with local offset, and terminate each event with exactly one newline. Permit only an explicit field-name allowlist such as `source`, `output`, `source_bytes`, `output_bytes`, `format`, `count`, `reason`, and `mode`.

- [x] **Step 4: Wire lifecycle and run events**

Initialize the sink from the configured destination. Log device detection, scan start, discovery, copy verification, conversion, artifact verification, finalization, refusal, Trash preflight/outcome, recovery, and run completion as those events become available. Use `SyncData` for run completion and immediately before source-retirement authority; a log error may leave a completed artifact but must set retirement ineligible.

- [x] **Step 5: Verify focused and application tests**

```bash
cargo test -p backup-core --test audit_log
cargo test --workspace --all-targets
```

- [x] **Step 6: Commit and push the audit checkpoint**

Stage only the files listed for this task and commit:

```bash
git commit -m "feat(core): write durable daily backup audit logs"
git push
```

Fetch and prove both Git parity counts are `0 0`.

---

### Task 3: Rescan trusted mounted transmitters every 15 seconds

**Files:**
- Create: `src-tauri/src/rescan.rs`
- Create: `src-tauri/tests/rescan_scheduler.rs`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/src/orchestrator.rs`
- Modify: `src-tauri/src/app_state.rs`
- Modify: `src-tauri/crates/backup-core/src/scanner.rs`
- Create: `src-tauri/crates/backup-core/tests/scanner_safety.rs`

**Interfaces:**

```rust
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScanFingerprint(Vec<(PathBuf, u64, SystemTime)>);

pub struct RescanScheduler {
    interval: Duration,
    next_due: HashMap<Transmitter, Instant>,
    fingerprints: HashMap<Transmitter, ScanFingerprint>,
    pending: bool,
}

pub enum RescanDecision { Unchanged, RequestBackup, KeepPending }
```

- [x] **Step 1: Add deterministic scheduler tests**

Prove mount schedules an immediate scan; unchanged metadata at the deadline yields `Unchanged`; added, removed, resized, or modification-time-changed visible WAVs yield one `RequestBackup`; repeated changes while the operation guard is busy preserve exactly one pending request; unmount clears the deadline, fingerprint, and pending authority; and disabling automatic backup prevents deadline work while `지금 백업` remains forceful.

- [x] **Step 2: Run the focused tests and observe the missing scheduler failure**

```bash
cargo test -p dji-mic-backup --test rescan_scheduler
```

- [x] **Step 3: Implement metadata-only fingerprints**

Reuse canonical scanner policy but do not open or hash audio. Sort `(relative_path, byte_count, modified_at)` deterministically. Hidden paths, symlinks, non-WAV files, and unstable metadata remain outside the fingerprint in the same way they remain outside discovery.

- [x] **Step 4: Integrate deadline polling and operation coalescing**

While at least one trusted device is mounted, poll the scheduler without blocking lifecycle messages. On change, set the existing `backup_pending` flag; if an operation is active, retain one pending bit and start it when the guard releases. On unmount, invalidate the associated mount and scan generations before clearing work.

- [x] **Step 5: Prove the original stuck-mounted scenario**

Use a temporary mounted-volume fixture: observe the initial fingerprint, create a stable DJI-named WAV without a lifecycle event, advance the fake clock by 15 seconds, and assert a backup request is issued. Assert the following unchanged interval creates neither a run nor an activity item.

- [x] **Step 6: Run suites, commit, and push**

```bash
cargo fmt --all -- --check
cargo test -p backup-core scanner
cargo test -p dji-mic-backup --test rescan_scheduler
cargo test --workspace --all-targets
git commit -m "fix(backup): rescan mounted transmitters for new recordings"
git push
```

Explicitly stage the listed files and prove live parity `0 0`.

---

### Task 4: Produce and verify M4A artifacts after the safe WAV copy

**Files:**
- Create: `src-tauri/src/platform/macos/audio.rs`
- Create: `src-tauri/src/artifact_pipeline.rs`
- Create: `src-tauri/tests/apple_audio_tools.rs`
- Modify: `src-tauri/src/platform/macos/mod.rs`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/src/orchestrator.rs`
- Modify: `src-tauri/Cargo.toml`
- Modify: `src-tauri/Cargo.lock`
- Modify: `src-tauri/crates/backup-core/src/backup.rs`
- Modify: `src-tauri/crates/backup-core/src/destination.rs`
- Modify: `src-tauri/crates/backup-core/src/recovery.rs`
- Modify: `src-tauri/crates/backup-core/src/artifact.rs`
- Create: `src-tauri/crates/backup-core/tests/backup_flow.rs`
- Create: `src-tauri/crates/backup-core/tests/destination_safety.rs`
- Create: `src-tauri/crates/backup-core/tests/recovery.rs`

**Interfaces:**

```rust
pub struct AudioDescription {
    pub container: String,
    pub codec: String,
    pub sample_rate_hz: u32,
    pub channels: u16,
    pub audio_bytes: u64,
    pub packets: u64,
    pub valid_frames: u64,
    pub priming_frames: u64,
    pub remainder_frames: u64,
    pub duration_micros: u64,
}

pub trait AudioTools: Send + Sync {
    fn inspect(&self, path: &Path) -> Result<AudioDescription, ArtifactError>;
    fn convert_aac_lc_192k(&self, input: &Path, output: &Path)
        -> Result<(), ArtifactError>;
}

pub enum ArtifactRequest<'a> {
    Wav { verified_staging: &'a Path },
    M4a { verified_staging: &'a Path, source_audio: AudioDescription },
}
```

- [x] **Step 1: Add failing artifact-policy tests with fake audio tools**

Assert that conversion cannot start before equal source/staging WAV bytes and hashes; final `.m4a` is not published before inspection and hashing; wrong container, non-AAC codec, wrong sample rate, wrong channels, zero packets, zero audio bytes, mismatched valid frames, or duration outside one AAC packet is rejected; a declared priming/remainder result is accepted; WAV mode preserves equal source/artifact hashes; and capacity includes staging plus output plus reserve.

- [x] **Step 2: Add macOS integration fixtures and observe failure**

Generate a small deterministic PCM WAV in the test, invoke production `/usr/bin/afconvert` and `/usr/bin/afinfo -x`, and assert the parsed result is M4A/AAC with matching shape and nonzero frames. The first run must fail because `AppleAudioTools` is absent.

- [x] **Step 3: Add pinned dependencies and the Apple tool adapter**

Add `quick-xml = { version = "=0.41.0", features = ["serialize"] }`. Invoke exactly:

```text
/usr/bin/afconvert INPUT -o OUTPUT -f m4af -d aac -b 192000 -q 127 -s 2
/usr/bin/afinfo -x PATH
```

Pass every token as a separate `Command::arg`, reject signal termination and nonzero status, cap captured output, and parse only the required XML fields. Do not accept localized plain-text output.

- [x] **Step 4: Split the copy and artifact boundaries**

Refactor `execute_backup_item_observed` so it first returns a durable, source-equal app-owned staging WAV. The artifact pipeline then chooses WAV or M4A from `BackupPreferences`, validates the result, hashes it, applies the existing no-clobber final name, atomically renames on the destination filesystem, synchronizes the parent directory, commits `VerifiedArtifact`, and only then removes the app-owned staging WAV.

- [x] **Step 5: Implement conservative restart recovery**

Recognize only app-owned staging and `.m4a.part-<uuid>` names. Reinspect a finalized M4A before repairing a missing ledger commit. Quarantine invalid or ambiguous conversion output and retain a verified staging WAV when Apple tooling fails. Never infer source-retirement authority from a recovered artifact without a live complete-snapshot check.

- [x] **Step 6: Implement verified legacy WAV migration**

When M4A mode is enabled, rehash an existing ledger-backed WAV, convert it through the same inspection/finalization path, commit the new artifact, then send the superseded destination WAV to a Mac-local Trash adapter supplied by Task 5. Until that adapter exists, retain the old WAV and record migration as pending; do not permanently unlink it.

- [x] **Step 7: Run focused, integration, and workspace suites**

```bash
cargo test -p backup-core backup_flow
cargo test -p backup-core destination_safety
cargo test -p backup-core recovery
cargo test -p dji-mic-backup --test apple_audio_tools
cargo test --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
```

- [x] **Step 8: Commit and push the artifact checkpoint**

Explicitly stage this task's files, commit `feat(backup): create verified m4a artifacts`, push, fetch, and prove both parity commands return `0 0`.

---

### Task 5: Replace permanent deletion with whole-session macOS Trash movement

**Files:**
- Create: `src-tauri/src/platform/macos/trash.rs`
- Create: `src-tauri/crates/backup-core/tests/session_retirement.rs`
- Create: `src-tauri/tests/macos_trash.rs`
- Modify: `src-tauri/src/platform/macos/mod.rs`
- Modify: `src-tauri/src/orchestrator.rs`
- Modify: `src-tauri/src/app_state.rs`
- Modify: `src-tauri/src/commands.rs`
- Modify: `src-tauri/Cargo.toml`
- Modify: `src-tauri/Cargo.lock`
- Modify: `src-tauri/crates/backup-core/src/deletion.rs`
- Modify: `src-tauri/crates/backup-core/src/ledger.rs`
- Modify: `src-tauri/crates/backup-core/src/state.rs`
- Modify: `src-tauri/crates/backup-core/src/error.rs`
- Create: `src-tauri/crates/backup-core/tests/deletion_guard.rs`
- Modify: `src-tauri/crates/backup-core/tests/fat32_deletion_acceptance.rs`

**Interfaces:**

```rust
pub enum RetirementTarget {
    Session { relative_directory: PathBuf, recordings: Vec<String> },
    RootFile { relative_path: PathBuf, recording: String },
}

pub trait TrashAdapter: Send + Sync {
    fn move_to_trash(&self, absolute_path: &Path) -> Result<(), TrashError>;
}

pub struct RetirementPlan {
    pub transmitter: Transmitter,
    pub targets: Vec<RetirementTarget>,
    pub recording_count: usize,
    pub byte_count: u64,
}
```

- [x] **Step 1: Add failing complete-session inventory tests**

Cover an exact verified session, two sessions, root-level WAVs, anchored-name mismatch, traversal, hidden entries, symlinks, nested directories, unknown files, missing candidates, changed metadata, changed hash, source/artifact mismatch, stale mount generation, stale scan generation, unavailable audit log, and a later target failing after an earlier successful target.

- [x] **Step 2: Add a regression test proving production policy never unlinks**

Remove `fs::remove_file` from the production retirement path. A source scan test must fail if `deletion.rs` or the macOS adapter contains `remove_file`, `remove_dir`, direct `.Trashes`, shell execution, Finder, or AppleScript disposal logic.

- [x] **Step 3: Implement grouping and shared revalidation**

Build `RetirementPlan` only after a fresh canonical inventory and rehash. For a recognized session, compare the complete recursive set with the verified candidate set before creating one directory target. Keep the five-minute opaque manual proposal, but have confirmation re-run the same function used by automatic retirement.

- [x] **Step 4: Implement the Foundation adapter**

Add target-specific dependencies:

```toml
[target.'cfg(target_os = "macos")'.dependencies]
objc2 = "=0.6.4"
objc2-foundation = { version = "=0.3.2", features = ["NSFileManager", "NSError", "NSURL"] }
```

Convert the already-authorized absolute `Path` with `NSURL::from_file_path` and call `NSFileManager::defaultManager().trashItemAtURL_resultingItemURL_error`. Return a privacy-safe typed error and do not retain or expose the resulting absolute Trash URL.

- [x] **Step 5: Implement manual and automatic retirement**

Rename the narrow IPC actions to `prepare_trash` and `confirm_trash`. After each independently successful transmitter backup, call the same retirement planner only when `automatic_trash` is true. Write and `sync_data` the preflight log before the first move. Record each target outcome so a partial run never repeats a completed move.

- [x] **Step 6: Reconcile the known empty legacy session safely**

Add an empty-folder candidate only when the anchored directory is empty, the ledger has legacy-retired recordings under that exact relative directory, the current paired identity matches, and the mount generation is current. Move the directory with the same Trash adapter and log `trash.legacy_empty_session`; leave every look-alike without ledger evidence untouched.

- [x] **Step 7: Add disposable-volume acceptance**

Update the fixture harness to use `/Volumes/DJI-DELTEST` only after verifying the volume name, removable/external properties, and fixture sentinel. Prove the whole session disappears from its original location, remains recoverable through macOS Trash behavior, and no permanent unlink occurs. Skip with an explicit reason when the disposable volume is absent.

- [x] **Step 8: Run safety tests, commit, and push**

```bash
cargo test -p backup-core --test session_retirement
cargo test -p backup-core --test deletion_guard
cargo test -p dji-mic-backup --test macos_trash
cargo test --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
```

Commit `feat(safety): move verified sessions to macos trash`, push, and prove `0 0` parity.

---

### Task 6: Add narrow settings IPC and a singleton settings window

**Files:**
- Modify: `src-tauri/src/commands.rs`
- Modify: `src-tauri/src/dto.rs`
- Modify: `src-tauri/src/app_state.rs`
- Modify: `src-tauri/src/window.rs`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/tauri.conf.json`
- Modify: `src-tauri/capabilities/main.json`
- Create: `src-tauri/tests/commands.rs`
- Create: `src-tauri/tests/runtime_shell.rs`
- Modify: `src-tauri/tests/ipc_contract.rs`

**Interfaces:**

```rust
#[tauri::command]
fn show_settings(app: AppHandle) -> Result<(), PublicError>;

#[tauri::command]
fn set_automatic_backup(state: State<'_, AppState>, enabled: bool)
    -> Result<AppSnapshotDto, PublicError>;

#[tauri::command]
fn set_m4a_conversion(state: State<'_, AppState>, enabled: bool)
    -> Result<AppSnapshotDto, PublicError>;

#[tauri::command]
fn set_automatic_trash(
    state: State<'_, AppState>, enabled: bool, acknowledged: bool
) -> Result<AppSnapshotDto, PublicError>;

#[tauri::command]
fn open_logs(app: AppHandle, state: State<'_, AppState>) -> Result<(), PublicError>;
```

- [x] **Step 1: Add failing command and privacy contract tests**

Assert the registered command set is exactly `get_app_snapshot`, `backup_now`, `choose_destination`, `pair_devices`, `prepare_trash`, `confirm_trash`, `set_autostart`, `open_destination`, `quit_app`, `show_settings`, `set_automatic_backup`, `set_m4a_conversion`, `set_automatic_trash`, and `open_logs`. Assert no command accepts an arbitrary key, path, process, UUID, hash, or Trash destination. Assert enabling automatic Trash without `acknowledged: true` is rejected and does not persist.

- [x] **Step 2: Add the settings window configuration**

Keep `main` as the 380×640 hidden undecorated popover. Add a hidden singleton `settings` window using `index.html?window=settings`, standard title-bar behavior, adaptive background, approximately 540×620 content size, no always-on-top, and no automatic hide on focus loss. Add both labels to the minimum capability allowlist.

- [x] **Step 3: Implement transactional settings commands**

Persist first, update runtime state second, and return a fresh snapshot. On failure leave both the database and visible runtime value unchanged. `open_logs` opens only the destination's computed `logs/YYYY/MM` directory; it never accepts a frontend path.

- [x] **Step 4: Update window lifecycle behavior**

`show_settings` hides the popover, shows and focuses the existing settings singleton, and never creates duplicates. Closing settings hides it. Losing settings focus does not hide it. Main popover retains its current focus-loss behavior.

- [x] **Step 5: Run IPC and runtime tests**

```bash
cargo test -p dji-mic-backup --test commands
cargo test -p dji-mic-backup --test ipc_contract
cargo test -p dji-mic-backup --test runtime_shell
cargo test --workspace --all-targets
```

- [x] **Step 6: Commit and push the settings backend checkpoint**

Commit `feat(settings): add backup automation controls`, push, and prove `0 0` parity.

---

### Task 7: Refine the popover and build the Apple-style settings UI

**Files:**
- Create: `src/features/backup/SettingsApp.tsx`
- Create: `src/features/backup/TrashDialog.tsx`
- Create: `src/features/backup/__tests__/SettingsApp.test.tsx`
- Create: `src/features/backup/__tests__/TrashDialog.test.tsx`
- Modify: `src/App.tsx`
- Modify: `src/index.css`
- Modify: `src/preview.tsx`
- Modify: `src/features/backup/BackupApp.tsx`
- Modify: `src/features/backup/BackupPopover.tsx`
- Modify: `src/features/backup/client.ts`
- Modify: `src/features/backup/contracts.ts`
- Modify: `src/features/backup/format.ts`
- Modify: `src/features/backup/useBackupSnapshot.ts`
- Create: `src/features/backup/__tests__/BackupApp.test.tsx`
- Modify: `src/features/backup/__tests__/BackupPopover.test.tsx`
- Modify: `src/features/backup/__tests__/client.test.ts`
- Modify: `src/features/backup/__tests__/contracts.test.ts`
- Modify: `src/features/backup/__tests__/useBackupSnapshot.test.tsx`
- Modify: `src/App.test.tsx`
- Delete: `src/features/backup/DeletionDialog.tsx`
- Modify: `contracts/README.md`
- Modify: `contracts/fixtures/backup-complete.json`
- Modify: `contracts/fixtures/backup-copying.json`
- Modify: `contracts/fixtures/error-destination-full.json`
- Create: `contracts/fixtures/trash-proposal.json`
- Create: `contracts/fixtures/partial-trash.json`
- Delete: `contracts/fixtures/deletion-proposal.json`
- Delete: `contracts/fixtures/partial-deletion.json`

**Frontend contract additions:**

```ts
type CurrentStage =
  | "copy"
  | "source_verification"
  | "conversion"
  | "artifact_verification"
  | "trash";

type BackupSettings = {
  automatic_backup: boolean;
  m4a_conversion: boolean;
  automatic_trash: boolean;
  autostart: boolean;
};
```

- [ ] **Step 1: Add failing strict-contract tests**

Extend the Zod schema and JSON fixture tests for artifact format, the five stage values, settings, retirement mode/outcome, and current-log availability. Keep `.strict()` at every object boundary and keep absolute paths, UUIDs, hashes, proposal internals, and audio metadata out of UI snapshots.

- [ ] **Step 2: Add interaction tests before components**

Prove `Settings…` calls `show_settings`; the popover exposes the sequence `복사 → 원본 검증 → M4A 변환 → M4A 검증 → 휴지통 이동`; `휴지통으로 이동` reports session/file/byte totals; automatic Trash requires a confirmation alert; a failed setting command restores the switch and renders an inline error; `로그 열기` invokes `open_logs`; and settings remain keyboard operable.

- [ ] **Step 3: Route each Tauri window to one root component**

Render `SettingsApp` only when `window=settings` is present; otherwise render `BackupApp`. Keep one shared typed client. Do not infer state from the DOM or duplicate persistence in `localStorage`.

- [ ] **Step 4: Implement the glanceable popover**

Use one adaptive material layer, a concise state sentence, one dominant progress value, compact TX01/TX02 rows, recent durable events, Settings, `지금 백업`, destination, log, and Quit. Rename every visible “삭제” action to “휴지통으로 이동.” Distinguish copy, source verification, conversion, artifact verification, refusal, partial Trash, and complete states.

- [ ] **Step 5: Implement the settings groups**

Build Backup, Source Safety, and General sections. Show the fixed M4A profile as explanatory copy. Keep automatic Trash off by default and require a modal acknowledgement on enable. Disable only the control with an in-flight command, preserve responsive access to other non-conflicting controls, and display persisted values returned by Rust.

- [ ] **Step 6: Apply the Apple interaction and accessibility contract**

Use system font and adaptive semantic colors, immediate pressed states, standard focus rings, minimum 44-point primary targets, clear labels, and no decorative gradients. Use critically damped short transforms only for state continuity. Under `prefers-reduced-motion`, use a short opacity change; under `prefers-reduced-transparency`, replace material with opaque adaptive surfaces. Verify light and dark contrast.

- [ ] **Step 7: Run frontend tests and build**

```bash
pnpm test -- --run
pnpm typecheck
pnpm build
```

- [ ] **Step 8: Commit and push the interface checkpoint**

Explicitly stage the listed UI, contract, and fixture files. Commit `feat(ui): refine backup status and settings`, push, and prove live parity `0 0`.

---

### Task 8: Update independent verification, packaging, and local installation

**Files:**
- Create: `scripts/install-local.sh`
- Modify: `scripts/check.sh`
- Modify: `scripts/package-local.sh`
- Modify: `scripts/verify-backup.sh`
- Modify: `scripts/accept-deletion-fixture.sh`
- Modify: `README.md`
- Modify: `docs/superpowers/specs/2026-08-09-dji-mic-m4a-trash-settings-design.md`
- Modify: `.agent/audit.md`
- Modify: `.agent/implement.md`
- Modify: `.agent/progress.md`

- [ ] **Step 1: Make independent verification artifact-aware**

Update `verify-backup.sh` to prove live source WAV SHA-256 against ledger source evidence, final artifact SHA-256 against ledger artifact evidence, and M4A shape through `/usr/bin/afinfo -x`. It must not compare a lossy M4A hash with a WAV hash. Preserve WAV-mode equality checks. Redact paths and hashes from ordinary output unless an explicit diagnostic flag is used.

- [ ] **Step 2: Make the safety fixture prove Trash semantics**

Retain the existing script name for compatibility, but update its language and assertions to require `/Volumes/DJI-DELTEST`, a fixture sentinel, recoverable Trash movement, whole-session disappearance from the source location, and absence of production `remove_file`/`remove_dir` disposal calls.

- [ ] **Step 3: Add a recoverable local installer**

`install-local.sh` must accept the verified release app, stage it in `mktemp -d`, verify deep strict code signing and bundle identity before touching the current installation, request the running app to quit, move the existing exact app bundle into the private temporary rollback directory, atomically install the staged bundle at `/Users/channprj/Applications/DJI Mic Backup.app`, reverify it, and restore the previous bundle on failure. It must never target `~`, `$HOME`, a workspace root, or a glob for recursive cleanup.

- [ ] **Step 4: Expand the repository check gate**

Add source scans preventing permanent production deletion, shell-based audio invocation, arbitrary settings IPC, and direct `.Trashes` manipulation. Keep formatting, all Rust tests, Clippy warnings-as-errors, frontend tests, typecheck, and production build.

- [ ] **Step 5: Document the new operational contract**

Update README instructions for defaults, daily log path, mounted-device 15-second detection, M4A verification, manual and automatic Trash behavior, automatic Trash confirmation, empty legacy-session reconciliation, disposable-volume acceptance, and local install/rollback.

- [ ] **Step 6: Run the full source and disposable acceptance gates**

```bash
./scripts/check.sh
./scripts/accept-deletion-fixture.sh
```

Expected: source gates pass. The disposable test passes only when the named fixture is mounted; otherwise it exits with the documented skip status and no production volume mutation.

- [ ] **Step 7: Verify the connected production devices read-only**

Run the independent verifier against TX01 and TX02 without retirement enabled. Confirm any live source has durable source evidence and a valid final artifact. Confirm no app-owned `.part` or staging file remains on the transmitters. Verify the known empty TX02 legacy session is eligible only through the ledger-backed empty-folder rule before allowing the app to move it to Trash.

- [ ] **Step 8: Build release artifacts and verify them**

```bash
./scripts/package-local.sh
```

Verify the `.app` and DMG, architecture, minimum macOS version, bundle identifier, and `codesign --verify --deep --strict` before installation.

- [ ] **Step 9: Install, relaunch, and prove installed parity**

```bash
./scripts/install-local.sh "src-tauri/target/release/bundle/macos/DJI Mic Backup.app"
open -a "/Users/channprj/Applications/DJI Mic Backup.app"
```

Compare SHA-256 of the release and installed executables and require equality. Confirm the installed process path, menu-bar status item, persisted settings, current paired devices, and creation/append of the current daily log under the configured destination.

- [ ] **Step 10: Capture final evidence and commit documentation/scripts**

Record exact commands and outcomes in `.agent/audit.md`, implementation notes in `.agent/implement.md`, and checkpoint hashes in `.agent/progress.md`. Update the approved design status to `Implemented and locally installed` only after the installed runtime proof succeeds. Explicitly stage only the files listed for this task, commit `build: verify and install the m4a backup release`, push, fetch, and prove both parity commands print `0 0`.

---

## Final Review Gate

- [ ] Every acceptance criterion in the approved design maps to a passing test or named runtime proof above.
- [ ] `rg -n "TODO|TBD|todo!|unimplemented!" src src-tauri scripts README.md` reports no unresolved implementation marker.
- [ ] `cargo fmt --all -- --check`, all Rust tests, Clippy, frontend tests, TypeScript, and production build pass from a clean checkout state.
- [ ] Production retirement contains no permanent unlink and every manual/automatic action uses the shared revalidation path.
- [ ] The installed executable is byte-identical to the verified release executable and deep strict code signing passes.
- [ ] `git status --short` is empty and local/tracking/live-remote parity is `0 0`.
