# Backup Mic Rule Engine and Product Rename Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Rename the macOS app to Backup Mic and replace the fixed DJI-only source model with editable glob rules that automatically detect, verify, convert, organize, and safely retire recordings from multiple removable recorders.

**Architecture:** Keep rule validation, matching, safe traversal, evidence, and retirement authority in `backup-core`; keep Disk Arbitration, Apple audio tools, app-data migration, and Foundation Trash adapters in the Tauri crate; keep React limited to strict DTOs and fixed commands. Introduce the rule engine beside the working DJI path, migrate durable identities to dynamic sources, then switch runtime/UI consumers and finally remove the obsolete fixed-source surface.

**Tech Stack:** Rust 2024, Tauri 2, SQLite/rusqlite, globset, Disk Arbitration, Foundation `NSFileManager`, macOS `/usr/bin/afconvert` and `/usr/bin/afinfo`, React 19, TypeScript 7, Zod 4, Vitest, Testing Library, shell acceptance fixtures, Headatever.

## Global Constraints

- macOS 13 or newer; the packaged executable must include arm64.
- Public product name, app bundle, executable, npm package, and active bundle identifier become `Backup Mic`, `Backup Mic.app`, `backup-mic`, `backup-mic`, and `com.channprj.BackupMic`.
- New installations default to `~/Documents/Backup Mic`; upgrades preserve the persisted destination.
- Rule syntax supports ASCII-case-insensitive `*`, `?`, and `**` globs with `/` separators; it never accepts regex, shell, templates, environment expansion, or executable expressions.
- Rule matching must stay below a canonical removable writable source root, never follow symlinks, skip `.Trashes` and system metadata, stop after 32 directory levels or 100,000 visited entries, and fail closed on ambiguous rules.
- Names, prefixes, and suffixes are at most 64 Unicode scalar values; patterns are at most 256; each pattern group contains at most 32 entries.
- The DJI Mic Mini 2S is one editable built-in rule using the generic engine. Existing TX01/TX02 identities and evidence migrate without loss.
- Recording artifacts use `<destination>/<archive directory>/YYYY/MM/YYMMDD-<prefix><source stem><suffix>.<wav|m4a>`; non-recording files remain in the reserved `source-extras` hierarchy.
- Automatic backup, AAC-LC 128 kbps conversion, and automatic Trash settings apply to every rule; rules and preferences are frozen for a run.
- Every source gets an independent copy/conversion barrier. A failed source remains intact and does not prevent another source from completing.
- No source becomes Trash-eligible until its current rule, mount authority, complete source inventory, hashes, final artifacts, batch barrier, and durable audit-log boundary are revalidated.
- Source retirement uses Foundation Trash only. Never use permanent unlink, Finder, AppleScript, shell disposal, or direct `.Trashes` manipulation.
- The webview never receives UUIDs, hashes, absolute source paths, arbitrary filesystem access, arbitrary setting keys, or arbitrary process execution.
- Application-data and artifact migration are atomic and idempotent. Legacy app data, logs, destination files, and the prior app bundle are never permanently deleted.
- Do not edit `VERSION` by hand. Use Headatever only after source, migration, fixture, and package gates pass.
- Follow `$gcpr`: explicit-path staging, Conventional Commit checkpoints, immediate ordinary push, and `0 0` upstream proof after every checkpoint. Never rewrite or force-push history.
- The repository instructions require inline sequential execution; do not dispatch subagents.
- Before every production-code step, write and run the named failing test; after implementation, rerun it and the affected suite.

---

### Task 1: Define and validate recorder backup rules

**Files:**
- Create: `src-tauri/crates/backup-core/src/rule.rs`
- Create: `src-tauri/crates/backup-core/tests/rule_validation.rs`
- Modify: `src-tauri/crates/backup-core/src/lib.rs`
- Modify: `src-tauri/crates/backup-core/src/error.rs`
- Modify: `src-tauri/crates/backup-core/Cargo.toml`
- Modify: `src-tauri/Cargo.lock`

**Interfaces:**
- Consumes: `CoreError`, safe relative-path helpers, serde conventions.
- Produces:

```rust
pub const MAX_RULE_TEXT_CHARS: usize = 64;
pub const MAX_GLOB_CHARS: usize = 256;
pub const MAX_PATTERNS_PER_GROUP: usize = 32;

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RuleId(String);

impl RuleId {
    pub fn new() -> Self;
    pub fn parse(value: &str) -> Result<Self, CoreError>;
    pub fn as_str(&self) -> &str;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FilenameProfile { Preserve, DjiTxShort }

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceConstraintProfile { GenericExternal, DjiMicMini2s }

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BackupRuleDraft {
    pub id: Option<RuleId>,
    pub name: String,
    pub archive_directory_name: String,
    pub enabled: bool,
    pub volume_name_glob: String,
    pub required_path_globs: Vec<String>,
    pub backup_file_globs: Vec<String>,
    pub session_directory_globs: Vec<String>,
    pub filename_prefix: String,
    pub filename_suffix: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BackupRule {
    pub id: RuleId,
    pub name: String,
    pub archive_directory_name: String,
    pub enabled: bool,
    pub volume_name_glob: String,
    pub required_path_globs: Vec<String>,
    pub backup_file_globs: Vec<String>,
    pub session_directory_globs: Vec<String>,
    pub filename_prefix: String,
    pub filename_suffix: String,
    pub filename_profile: FilenameProfile,
    pub device_constraint_profile: DeviceConstraintProfile,
    pub preset_kind: Option<String>,
    pub preset_revision: Option<u32>,
    pub archive_directory_locked: bool,
    pub archived_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

pub struct CompiledBackupRule {
    pub rule: BackupRule,
    volume_matcher: globset::GlobMatcher,
    required_matchers: Vec<globset::GlobMatcher>,
    backup_matchers: Vec<globset::GlobMatcher>,
    session_matchers: Vec<globset::GlobMatcher>,
}

pub fn validate_rule(draft: BackupRuleDraft) -> Result<BackupRuleDraft, CoreError>;
pub fn compile_rule(rule: BackupRule) -> Result<CompiledBackupRule, CoreError>;
pub fn apply_filename_profile(profile: FilenameProfile, stem: &str) -> String;
pub fn destination_stem(rule: &BackupRule, source_stem: &str) -> Result<String, CoreError>;
```

- [ ] **Step 1: Write failing validation and naming tests**

In `rule_validation.rs`, create a minimal draft with `volume_name_glob = "ZOOM_*"`, `backup_file_globs = ["RECORD/**/*.WAV"]`, prefix `zoom-`, and suffix `-field`. Assert validation accepts it and `destination_stem` returns `zoom-ZOOM0001-field`. Add table cases that reject an empty backup pattern list, malformed `[abc`, `/` in a suffix, `..` as a name, 65-character names, 257-character patterns, and 33 entries. Assert `DjiTxShort` maps only leading ASCII-case-insensitive `TX01_`/`TX02_` to `T01_`/`T02_`.

- [ ] **Step 2: Run the RED test**

```bash
cargo test --manifest-path src-tauri/Cargo.toml -p backup-core --test rule_validation
```

Expected: compilation fails because `backup_core::rule` and its types do not exist.

- [ ] **Step 3: Add the pinned glob compiler and rule module**

Add `globset = "=0.4.19"` to `backup-core`. Implement `CompiledBackupRule` with precompiled `GlobMatcher` values using `GlobBuilder::case_insensitive(true).literal_separator(true)`. Normalize `\` to `/` before matching. Map invalid input to new privacy-safe `CoreError::InvalidRule` and `PublicErrorCode::InvalidRule` values without returning raw source paths.

- [ ] **Step 4: Implement exact text and filename validation**

Reject NUL/control characters, separators, leading `.`, exact `.`/`..`, whitespace-only non-empty prefix/suffix, and normalized-empty names. Preserve Unicode content otherwise. `destination_stem` must apply the profile first, wrap it with prefix/suffix, and re-run safe-component validation before returning.

- [ ] **Step 5: Run focused and core suites**

```bash
cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check
cargo test --manifest-path src-tauri/Cargo.toml -p backup-core --test rule_validation
cargo test --manifest-path src-tauri/Cargo.toml -p backup-core --lib
cargo clippy --manifest-path src-tauri/Cargo.toml -p backup-core --all-targets -- -D warnings
git diff --check
```

- [ ] **Step 6: Commit and push the rule-domain checkpoint**

Explicitly stage the Task 1 files, commit `feat(rules): add safe recorder rule validation`, push, fetch, and require `git rev-list --left-right --count HEAD...@{u}` to print `0 0`.

---

### Task 2: Persist editable rules and seed the DJI preset

**Files:**
- Create: `src-tauri/crates/backup-core/migrations/0005_backup_rules.sql`
- Create: `src-tauri/crates/backup-core/src/preset.rs`
- Create: `src-tauri/crates/backup-core/tests/rule_ledger.rs`
- Modify: `src-tauri/crates/backup-core/src/lib.rs`
- Modify: `src-tauri/crates/backup-core/src/ledger.rs`
- Modify: `src-tauri/crates/backup-core/tests/ledger_recovery.rs`

**Interfaces:**
- Consumes: `BackupRule`, `BackupRuleDraft`, `FilenameProfile`, `Ledger`, schema migrations 1-4.
- Produces:

```rust
pub const DJI_PRESET_KIND: &str = "dji_mic_mini_2s";
pub const DJI_PRESET_REVISION: u32 = 1;

pub fn dji_mic_mini_2s_preset() -> BackupRuleDraft;

impl Ledger {
    pub fn backup_rules(&self, include_archived: bool) -> Result<Vec<BackupRule>, CoreError>;
    pub fn save_backup_rule(
        &mut self,
        draft: BackupRuleDraft,
        updated_at: &str,
    ) -> Result<BackupRule, CoreError>;
    pub fn archive_backup_rule(&mut self, id: &RuleId, archived_at: &str) -> Result<(), CoreError>;
    pub fn restore_dji_preset(&mut self, updated_at: &str) -> Result<BackupRule, CoreError>;
    pub fn dji_rule(&self) -> Result<BackupRule, CoreError>;
}
```

- [ ] **Step 1: Write a failing new-ledger preset round-trip test**

Open a new ledger and assert it contains exactly one enabled rule named `DJI Mic Mini 2S`, with `volume_name_glob = "*"`, `filename_profile = DjiTxShort`, `device_constraint_profile = DjiMicMini2s`, root/session WAV globs, session-directory glob `TX_MIC*`, preset revision 1, and no archive lock. Save a `Zoom H1n` rule, reopen the ledger, and compare every field and pattern order.

- [ ] **Step 2: Write failing edit, archive, and restore tests**

Edit the DJI prefix, archive it, call `restore_dji_preset`, and assert the rule reuses its original ID, is enabled, has revision 1 defaults, and retains a synthetic device-binding row. Assert an unused user rule is physically deleted by the archive operation while a rule referenced by one synthetic artifact becomes archived.

- [ ] **Step 3: Run the RED tests**

```bash
cargo test --manifest-path src-tauri/Cargo.toml -p backup-core --test rule_ledger
cargo test --manifest-path src-tauri/Cargo.toml -p backup-core --test ledger_recovery
```

Expected: failures identify migration 5, the preset module, and missing ledger methods.

- [ ] **Step 4: Add forward-only schema version 5**

Create normalized `backup_rules`, `backup_rule_patterns`, and `rule_device_bindings` tables. Persist the filename and device-constraint profiles as checked enum text. Enforce unique normalized active names, unique `(rule_id, kind, ordinal)` patterns, allowed pattern kinds, preset uniqueness, and foreign keys. Seed the DJI preset in the same migration transaction using a stable UUID literal so every fresh and upgraded ledger receives the same logical preset.

- [ ] **Step 5: Implement transactional rule persistence**

Validate before opening a transaction; upsert the rule row, replace its ordered pattern rows, commit, and read it back. Refuse changes to `archive_directory_name` after evidence locks it. `restore_dji_preset` updates only editable preset fields, clears `archived_at`, preserves ID/bindings/evidence, and never touches global preferences.

- [ ] **Step 6: Run ledger and migration suites**

```bash
cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check
cargo test --manifest-path src-tauri/Cargo.toml -p backup-core --test rule_ledger
cargo test --manifest-path src-tauri/Cargo.toml -p backup-core --test ledger_recovery
cargo test --manifest-path src-tauri/Cargo.toml -p backup-core --lib
git diff --check
```

- [ ] **Step 7: Commit and push the persisted-rule checkpoint**

Explicitly stage the Task 2 files, commit `feat(rules): persist editable backup rules`, push, fetch, and prove `0 0` parity.

---

### Task 3: Migrate durable evidence to dynamic source identities

**Files:**
- Create: `src-tauri/crates/backup-core/migrations/0006_dynamic_sources.sql`
- Create: `src-tauri/crates/backup-core/src/source.rs`
- Create: `src-tauri/crates/backup-core/tests/dynamic_sources.rs`
- Modify: `src-tauri/crates/backup-core/src/lib.rs`
- Modify: `src-tauri/crates/backup-core/src/state.rs`
- Modify: `src-tauri/crates/backup-core/src/device.rs`
- Modify: `src-tauri/crates/backup-core/src/recording.rs`
- Modify: `src-tauri/crates/backup-core/src/additional_file.rs`
- Modify: `src-tauri/crates/backup-core/src/events.rs`
- Modify: `src-tauri/crates/backup-core/src/batch.rs`
- Modify: `src-tauri/crates/backup-core/src/ledger.rs`
- Modify: `src-tauri/crates/backup-core/tests/ledger_recovery.rs`
- Modify: `scripts/verify-backup.sh`

**Interfaces:**
- Consumes: `RuleId`, legacy `Transmitter`, `PairedDevice`, all current evidence tables.
- Produces:

```rust
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SourceId(String);

impl SourceId {
    pub fn new() -> Self;
    pub fn parse(value: &str) -> Result<Self, CoreError>;
    pub fn as_str(&self) -> &str;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceRecord {
    pub id: SourceId,
    pub rule_id: RuleId,
    pub volume_uuid: String,
    pub legacy_slot: Option<String>,
    pub display_name: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MountedSourceAuthority {
    pub source: SourceRecord,
    pub descriptor: VolumeDescriptor,
}

impl Ledger {
    pub fn upsert_source(&mut self, source: &SourceRecord, seen_at: &str) -> Result<(), CoreError>;
    pub fn source(&self, id: &SourceId) -> Result<SourceRecord, CoreError>;
    pub fn sources_for_rule(&self, rule: &RuleId) -> Result<Vec<SourceRecord>, CoreError>;
}
```

- [ ] **Step 1: Write a failing v4-to-v6 migration test**

Create a schema-v4 fixture containing paired TX01/TX02 devices, one recording per transmitter, an additional file, a completed batch, an activity, and a deletion run. Open it through `Ledger::open` and assert two source rows reference the DJI preset, preserve the legacy slots, and every evidence row resolves to the correct `SourceId` without changing hashes, byte counts, phases, timestamps, or retirement outcomes.

- [ ] **Step 2: Write failing dynamic-source round-trip tests**

Insert two sources for one `Zoom H1n` rule with different UUIDs and identical display names. Commit same-named recordings for each source and assert ledger uniqueness uses `(source_id, source_relative_path)` rather than transmitter text. Serialize `SourceId` and assert it is opaque and contains neither rule name nor UUID.

- [ ] **Step 3: Run the RED tests**

```bash
cargo test --manifest-path src-tauri/Cargo.toml -p backup-core --test dynamic_sources
cargo test --manifest-path src-tauri/Cargo.toml -p backup-core --test ledger_recovery
```

Expected: compilation fails for `source`, and the upgrade fixture has no source columns.

- [ ] **Step 4: Add schema version 6 and backfill legacy evidence**

Create `sources` with unique `(rule_id, lower(volume_uuid))`. Add nullable `source_id` columns to recordings, additional files, backup runs/items, conversion items, activities, deletion runs/items, and superseded-WAV evidence; backfill through TX01/TX02 DJI source rows; add indexes and triggers that require `source_id` for new writes. Retain legacy transmitter columns read-only so old evidence remains independently inspectable; no new API may key on them.

- [ ] **Step 5: Replace core evidence fields with `SourceId`**

Change `VerifiedRecording`, `VerifiedAdditionalFile`, `BatchItemKey`, `ActivityEntry`, deletion evidence, and ledger query parameters from `Transmitter` to `SourceId`. Keep `Transmitter` only inside legacy-migration and DJI preset display compatibility. Update row decoders, equality checks, verifier SQL, and privacy assertions.

- [ ] **Step 6: Run all core evidence suites**

```bash
cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check
cargo test --manifest-path src-tauri/Cargo.toml -p backup-core --test dynamic_sources
cargo test --manifest-path src-tauri/Cargo.toml -p backup-core --test ledger_recovery
cargo test --manifest-path src-tauri/Cargo.toml -p backup-core --test batch_ledger
cargo test --manifest-path src-tauri/Cargo.toml -p backup-core --test backup_flow
cargo test --manifest-path src-tauri/Cargo.toml -p backup-core --test session_retirement
cargo clippy --manifest-path src-tauri/Cargo.toml -p backup-core --all-targets -- -D warnings
git diff --check
```

- [ ] **Step 7: Commit and push the dynamic-evidence checkpoint**

Explicitly stage the Task 3 files, commit `refactor(core): generalize backup source identity`, push, fetch, and prove `0 0` parity.

---

### Task 4: Scan rule-selected files and plan consistent destinations

**Files:**
- Create: `src-tauri/crates/backup-core/src/rule_scanner.rs`
- Create: `src-tauri/crates/backup-core/tests/rule_scanner.rs`
- Create: `src-tauri/crates/backup-core/tests/rule_destinations.rs`
- Modify: `src-tauri/crates/backup-core/src/lib.rs`
- Modify: `src-tauri/crates/backup-core/src/filesystem.rs`
- Modify: `src-tauri/crates/backup-core/src/recording.rs`
- Modify: `src-tauri/crates/backup-core/src/destination.rs`
- Modify: `src-tauri/crates/backup-core/src/backup.rs`

**Interfaces:**
- Consumes: `CompiledBackupRule`, `BackupRule`, `SourceId`, `Clock`, current hash/no-clobber destination helpers.
- Produces:

```rust
pub const MAX_SCAN_DEPTH: usize = 32;
pub const MAX_VISITED_ENTRIES: usize = 100_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SelectedFileKind { RecordingWav, Companion }

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuleFileObservation {
    pub relative_path: PathBuf,
    pub file_name: String,
    pub kind: SelectedFileKind,
    pub session_relative_path: Option<PathBuf>,
    pub size: u64,
    pub modified_nanos: i128,
    pub archive_date: time::Date,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuleScanResult {
    pub files: Vec<RuleFileObservation>,
    pub fingerprint: RuleScanFingerprint,
    pub unsafe_session_count: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuleScanFingerprint(Vec<(PathBuf, u64, i128)>);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuleDestinationPlan {
    pub source: RuleFileObservation,
    pub relative_destination: PathBuf,
    pub disposition: DestinationDisposition,
}

pub fn scan_rule_once(
    root: &Path,
    rule: &CompiledBackupRule,
    local_offset: UtcOffset,
) -> Result<RuleScanResult, CoreError>;

pub fn scan_rule_stable(
    root: &Path,
    rule: &CompiledBackupRule,
    local_offset: UtcOffset,
    clock: &dyn Clock,
) -> Result<RuleScanResult, CoreError>;

pub fn plan_rule_file(
    rule: &BackupRule,
    source_id: &SourceId,
    source: &RuleFileObservation,
    existing: Option<&VerifiedArtifact>,
) -> Result<RuleDestinationPlan, CoreError>;
```

- [ ] **Step 1: Write failing matcher and traversal tests**

Create a `ZOOM_H1N` fixture with `RECORD/FOLDER01/ZOOM0001.WAV`, a selected text companion, an unrelated file, `.Trashes`, a 33-level directory, and (on Unix) an out-of-root symlink. Assert the valid rule selects only the two intended regular files, marks WAV versus companion, assigns the WAV's modification-date calendar date, never follows unsafe entries, and returns `CoreError::RuleScanLimit` when entry/depth bounds are exceeded.

- [ ] **Step 2: Write failing stability and session tests**

Use a mutating clock to change one WAV between scans and require its exclusion. For `session_directory_globs = ["RECORD/FOLDER*"]`, assert every selected member carries the same session relative path and that an unselected regular entry or nested directory increments `unsafe_session_count`, preventing session retirement while still allowing verified file backup.

- [ ] **Step 3: Write failing destination tests**

For source `ZOOM0001.WAV`, date 2026-08-10, prefix `zoom-`, suffix `-field`, assert WAV and M4A plans are `Zoom H1n/2026/08/260810-zoom-ZOOM0001-field.wav` and `.m4a`. Assert a companion uses `Zoom H1n/source-extras/<opaque-key>/RECORD/FOLDER01/notes.txt`, equal content reuses its target, and different content receives an 8-character hash suffix without clobbering.

- [ ] **Step 4: Run the RED tests**

```bash
cargo test --manifest-path src-tauri/Cargo.toml -p backup-core --test rule_scanner
cargo test --manifest-path src-tauri/Cargo.toml -p backup-core --test rule_destinations
```

Expected: compilation fails for `rule_scanner`, rule destination types, and scan-limit errors.

- [ ] **Step 5: Implement the bounded canonical walker**

Walk once with `symlink_metadata`, count every visited entry, normalize safe relative paths, evaluate all required patterns, then select any backup-file match. Treat a `.wav` extension ASCII-case-insensitively as `RecordingWav`; everything else is `Companion`. Use stable modification time for generic archive dates and keep the DJI filename parser behind `DjiTxShort` so its encoded date remains authoritative.

- [ ] **Step 6: Implement rule-aware destination planning**

Reuse the current temporary-copy, canonical destination, content-reuse, and hash-suffix primitives. Prefix every playable artifact with the locked archive directory and established `YYYY/MM/YYMMDD-` calendar path. Derive the companion evidence key from the source ID through a one-way digest truncated for collision resistance; never serialize the UUID itself.

- [ ] **Step 7: Run scanner, destination, and property suites**

```bash
cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check
cargo test --manifest-path src-tauri/Cargo.toml -p backup-core --test rule_scanner
cargo test --manifest-path src-tauri/Cargo.toml -p backup-core --test rule_destinations
cargo test --manifest-path src-tauri/Cargo.toml -p backup-core --test scanner_safety
cargo test --manifest-path src-tauri/Cargo.toml -p backup-core --test path_properties
cargo test --manifest-path src-tauri/Cargo.toml -p backup-core --test additional_files
cargo clippy --manifest-path src-tauri/Cargo.toml -p backup-core --all-targets -- -D warnings
git diff --check
```

- [ ] **Step 8: Commit and push the scanner/planner checkpoint**

Explicitly stage the Task 4 files, commit `feat(backup): scan rule-matched recorder files`, push, fetch, and prove `0 0` parity.

---

### Task 5: Match mounted volumes and rescan dynamic sources

**Files:**
- Create: `src-tauri/src/rule_runtime.rs`
- Create: `src-tauri/tests/rule_device_lifecycle.rs`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/src/platform/device_registry.rs`
- Modify: `src-tauri/src/rescan.rs`
- Modify: `src-tauri/src/app_state.rs`
- Modify: `src-tauri/src/orchestrator.rs`
- Modify: `src-tauri/src/failure_reporter.rs`
- Modify: `src-tauri/tests/rescan_scheduler.rs`

**Interfaces:**
- Consumes: `MountedVolume`, `BackupRule`, `scan_rule_once`, `RuleScanFingerprint`, `SourceRecord`, `MountedSourceAuthority`.
- Produces:

```rust
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RuleVolumeMatch {
    Unrelated,
    Matched(MatchedSource),
    Conflict { rule_ids: Vec<RuleId>, display_name: String },
    Rejected(PublicError),
}

#[derive(Clone, Debug)]
pub struct MatchedSource {
    pub authority: MountedSourceAuthority,
    pub rule: BackupRule,
    pub mounted: MountedVolume,
    pub initial_fingerprint: RuleScanFingerprint,
}

pub fn match_mounted_volume(
    ledger: &mut Ledger,
    mounted: &MountedVolume,
    rules: &[BackupRule],
    local_offset: UtcOffset,
    seen_at: &str,
) -> Result<RuleVolumeMatch, CoreError>;

pub struct RescanScheduler {
    // keyed by SourceId; no fixed two-source loop
}
```

- [ ] **Step 1: Write failing volume-match tests**

Construct removable/writable USB fixtures for `ZOOM_H1N` and `SONY_REC`, each with its own rule and directory layout. Assert one rule yields `Matched`, no rule yields `Unrelated`, two matching rules yield a sorted `Conflict`, and internal, read-only, non-removable, and non-USB candidates yield `Rejected` without creating a source row.

- [ ] **Step 2: Write failing identity and rescan tests**

Match the same rule/UUID at mount generation 1 and 2 and assert the durable `SourceId` stays equal while `MountedSourceAuthority` changes. Match two UUIDs with the same label and assert distinct sources. Update `rescan_scheduler.rs` so arbitrary `SourceId` values mount, defer, become pending, and unmount independently.

- [ ] **Step 3: Run the RED tests**

```bash
cargo test --manifest-path src-tauri/Cargo.toml -p dji-mic-backup --test rule_device_lifecycle
cargo test --manifest-path src-tauri/Cargo.toml -p dji-mic-backup --test rescan_scheduler
```

Expected: failures identify `rule_runtime` and the transmitter-keyed scheduler.

- [ ] **Step 4: Implement fail-closed mount matching**

Filter physical properties before filesystem work. Evaluate all enabled non-archived rules, including DJI preset constraints, with the same bounded matcher. Create/update a `SourceRecord` only after exactly one rule matches. Report conflicts with rule IDs internally and sanitized rule names/display name publicly; do not publish UUIDs or paths.

- [ ] **Step 5: Generalize observed and mounted state maps**

Replace `HashMap<Transmitter, MountedVolume>` with `HashMap<SourceId, MatchedSource>`. Preserve an observed-volume map for read-only rule testing. Unmount only when UUID and mount generation both match. Invalidate deletion proposals for that source and cancel its pending rescan; do not clear unrelated sources.

- [ ] **Step 6: Replace the fixed scheduler loop**

Drive initial fingerprints and the 15-second rescan from current matched sources. `RescanScheduler::due_sources` returns `Vec<SourceId>`. A changed fingerprint queues that source; when a process-wide operation is active, retain the per-source pending bit so it runs after the guard releases.

- [ ] **Step 7: Run runtime lifecycle suites**

```bash
cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check
cargo test --manifest-path src-tauri/Cargo.toml -p dji-mic-backup --test rule_device_lifecycle
cargo test --manifest-path src-tauri/Cargo.toml -p dji-mic-backup --test rescan_scheduler
cargo test --manifest-path src-tauri/Cargo.toml -p dji-mic-backup --test failure_reporting
cargo clippy --manifest-path src-tauri/Cargo.toml --workspace --all-targets -- -D warnings
git diff --check
```

- [ ] **Step 8: Commit and push the mount-runtime checkpoint**

Explicitly stage the Task 5 files, commit `feat(runtime): detect rule-matched recorder volumes`, push, fetch, and prove `0 0` parity.

---

### Task 6: Run independent verified backup cohorts for dynamic sources

**Files:**
- Create: `src-tauri/tests/multi_rule_backup.rs`
- Modify: `src-tauri/crates/backup-core/src/backup.rs`
- Modify: `src-tauri/crates/backup-core/src/batch.rs`
- Modify: `src-tauri/crates/backup-core/src/ledger.rs`
- Modify: `src-tauri/src/app_state.rs`
- Modify: `src-tauri/src/orchestrator.rs`
- Modify: `src-tauri/src/artifact_pipeline.rs`
- Modify: `src-tauri/tests/artifact_pipeline.rs`
- Modify: `src-tauri/tests/conversion_cohort.rs`
- Modify: `src-tauri/tests/concurrent_settings.rs`

**Interfaces:**
- Consumes: `MatchedSource`, `RuleScanResult`, `RuleDestinationPlan`, `SourceId`, current copy/conversion/audio adapters.
- Produces:

```rust
struct PreparedSource {
    source_id: SourceId,
    authority: MountedSourceAuthority,
    rule: BackupRule,
    source_root: PathBuf,
    scan: RuleScanResult,
    recording_plans: Vec<DestinationPlan>,
    companion_plans: Vec<AdditionalFilePlan>,
    copy_barrier: CopyBarrier,
    required_copy_bytes: u64,
}

pub struct SourceRunOutcome {
    pub source_id: SourceId,
    pub phase: BackupPhase,
    pub verified_files: u64,
    pub error: Option<PublicError>,
}

pub fn run_backup(
    app: &AppHandle,
    state: &AppState,
    cancellation: &CancellationToken,
) -> Result<Vec<SourceRunOutcome>, CoreError>;
```

- [ ] **Step 1: Write a failing two-rule success test**

Create Zoom and Sony source roots with the same `REC0001.WAV` name, distinct rule folders/prefixes, and PCM fixture bytes. Invoke the production backup path with fake audio tools. Assert both sources reach `Completed`, their copied WAV hashes equal their sources, their final M4As pass the existing audio-property checks, paths are isolated by archive directory, and both batch barriers are complete.

- [ ] **Step 2: Write a failing independent-failure regression test**

Inject a copy fault for only the Sony source. Assert Zoom still completes conversion and becomes deletion-ready, Sony records `PartialFailure`, no Sony conversion starts, every Sony source file remains, and the overall snapshot reports partial failure without erasing Zoom's success.

- [ ] **Step 3: Write a failing frozen-rule test**

Begin a run with prefix `zoom-`, persist prefix `field-` while the operation guard is held, and assert current artifact naming uses `zoom-`, the snapshot marks `setting_applies_next_run`, and the next rescan plans `field-` without duplicating the already verified source artifact.

- [ ] **Step 4: Run the RED tests**

```bash
cargo test --manifest-path src-tauri/Cargo.toml -p dji-mic-backup --test multi_rule_backup
cargo test --manifest-path src-tauri/Cargo.toml -p dji-mic-backup --test concurrent_settings
```

Expected: fixed `PreparedTransmitter` and global barrier behavior cannot satisfy the tests.

- [ ] **Step 5: Replace transmitter preparation with source preparation**

Snapshot every matched source and its persisted rule before scanning. Build a separate `CopyBarrier` and batch row per source. Calculate capacity across serialized source work while retaining per-source expected bytes. Use `SourceId` in every recording, companion, conversion, progress, and activity record.

- [ ] **Step 6: Isolate source failure boundaries**

Process each prepared source through copy, barrier, conversion, artifact verification, and source revalidation. Record a failed source outcome and continue with the next prepared source unless the failure is process-wide (`DestinationUnavailable`, ledger corruption, cancellation, or destination-generation change). Compute overall phase from all outcomes: all complete, nothing new, partial failure, or fatal error.

- [ ] **Step 7: Preserve no-clobber and reuse guarantees**

Before reusing evidence, rehash current source and destination according to the existing ledger contract. Lock `archive_directory_name` on the first committed artifact. A later display-name edit cannot relocate or duplicate artifacts. A prefix/suffix change affects only new filenames while source evidence prevents a duplicate of an already verified item.

- [ ] **Step 8: Run affected core and runtime suites**

```bash
cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check
cargo test --manifest-path src-tauri/Cargo.toml -p dji-mic-backup --test multi_rule_backup
cargo test --manifest-path src-tauri/Cargo.toml -p dji-mic-backup --test artifact_pipeline
cargo test --manifest-path src-tauri/Cargo.toml -p dji-mic-backup --test conversion_cohort
cargo test --manifest-path src-tauri/Cargo.toml -p dji-mic-backup --test concurrent_settings
cargo test --manifest-path src-tauri/Cargo.toml -p backup-core --test batch_barrier
cargo test --manifest-path src-tauri/Cargo.toml -p backup-core --test backup_flow
cargo clippy --manifest-path src-tauri/Cargo.toml --workspace --all-targets -- -D warnings
git diff --check
```

- [ ] **Step 9: Commit and push the independent-backup checkpoint**

Explicitly stage the Task 6 files, commit `feat(backup): process matched recorders independently`, push, fetch, and prove `0 0` parity.

---

### Task 7: Revalidate dynamic rule sources before recoverable Trash

**Files:**
- Create: `src-tauri/crates/backup-core/tests/rule_retirement.rs`
- Create: `src-tauri/tests/rule_trash_flow.rs`
- Modify: `src-tauri/crates/backup-core/src/deletion.rs`
- Modify: `src-tauri/crates/backup-core/src/ledger.rs`
- Modify: `src-tauri/src/app_state.rs`
- Modify: `src-tauri/src/orchestrator.rs`
- Modify: `src-tauri/src/commands.rs`
- Modify: `src-tauri/tests/fat32_trash_acceptance.rs`
- Modify: `scripts/accept-deletion-fixture.sh`

**Interfaces:**
- Consumes: `SourceId`, `MountedSourceAuthority`, `BackupRule`, `RuleScanResult`, existing opaque proposals and `TrashAdapter`.
- Produces:

```rust
pub struct RuleDeletionContext<'a> {
    pub source_id: &'a SourceId,
    pub rule_id: &'a RuleId,
    pub rule_updated_at: &'a str,
    pub authority: &'a MountedSourceAuthority,
    pub destination_generation: u64,
    pub scan_generation: u64,
}

pub struct CompleteRuleDeletionSnapshot {
    pub files: Vec<DeletionCandidate>,
    pub sessions: Vec<RuleSessionDeletionCandidate>,
}

pub fn propose_rule_deletion(
    context: &RuleDeletionContext<'_>,
    snapshot: CompleteRuleDeletionSnapshot,
    expires_at: OffsetDateTime,
) -> Result<DeletionProposal, CoreError>;
```

- [ ] **Step 1: Write failing per-file and session retirement tests**

For a generic rule without session globs, verify only ledger-complete files become candidates and are moved individually. For a rule with `RECORD/FOLDER*`, verify the folder moves as one item only when every regular entry has copy/artifact evidence. Add an unselected `notes.tmp`, nested directory, and symlink case; each must preserve the entire session and return `SourceChanged` or `UnsafeSessionEntry`.

- [ ] **Step 2: Write failing invalidation tests**

Create proposals and mutate, one at a time, rule revision, rule enabled state, volume UUID, mount generation, source file bytes, destination generation, scan generation, batch barrier, and final M4A bytes. Assert confirmation refuses before calling the fake Trash adapter. Advance time beyond five minutes and assert expiry.

- [ ] **Step 3: Run the RED tests**

```bash
cargo test --manifest-path src-tauri/Cargo.toml -p backup-core --test rule_retirement
cargo test --manifest-path src-tauri/Cargo.toml -p dji-mic-backup --test rule_trash_flow
```

Expected: deletion APIs still require `Transmitter` and DJI session recognition.

- [ ] **Step 4: Generalize deletion evidence and proposal authority**

Key pending proposals, deletion runs, candidates, and failure outcomes by `SourceId`. Freeze the exact rule ID and update timestamp. Re-run the generic scanner during confirmation, compare the complete selected inventory, and separately require complete regular-entry coverage for any session-directory candidate.

- [ ] **Step 5: Preserve WAV-retention and source independence**

When M4A conversion is disabled, reject WAV source retirement exactly as today. Automatic Trash evaluates completed sources independently; one refused source does not reset another source's verified outcome. Manual commands accept only an opaque `SourceId` already present in the current snapshot.

- [ ] **Step 6: Expand the disposable FAT32 fixture**

Add one DJI session-shaped source and one generic `ZOOM_TEST/RECORD/FOLDER01` source inside the disposable `DJI-DELTEST` image. Require complete verified copy/conversion evidence, then prove only the requested generic session moves to recoverable Trash; the other source and destination remain intact. Keep the exact marker, mount-name, capacity, and real-recorder refusal checks.

- [ ] **Step 7: Run deletion and safety suites**

```bash
cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check
cargo test --manifest-path src-tauri/Cargo.toml -p backup-core --test rule_retirement
cargo test --manifest-path src-tauri/Cargo.toml -p backup-core --test session_retirement
cargo test --manifest-path src-tauri/Cargo.toml -p backup-core --test run_retirement
cargo test --manifest-path src-tauri/Cargo.toml -p dji-mic-backup --test rule_trash_flow
cargo test --manifest-path src-tauri/Cargo.toml -p dji-mic-backup --test fat32_trash_acceptance
cargo clippy --manifest-path src-tauri/Cargo.toml --workspace --all-targets -- -D warnings
git diff --check
```

- [ ] **Step 8: Commit and push the dynamic-retirement checkpoint**

Explicitly stage the Task 7 files, commit `feat(safety): revalidate rule sources before trash`, push, fetch, and prove `0 0` parity.

---

### Task 8: Expose a strict rule and dynamic-source IPC contract

**Files:**
- Create: `src-tauri/tests/rule_commands.rs`
- Modify: `src-tauri/src/dto.rs`
- Modify: `src-tauri/src/commands.rs`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/src/app_state.rs`
- Modify: `src-tauri/tests/commands.rs`
- Modify: `src-tauri/tests/ipc_contract.rs`
- Modify: `contracts/fixtures/backup-complete.json`
- Modify: `contracts/fixtures/backup-copying.json`
- Modify: `contracts/fixtures/backup-error.json`
- Modify: `contracts/fixtures/setup-pairing.json`
- Modify: `contracts/fixtures/trash-proposal.json`
- Modify: `src/features/backup/contracts.ts`
- Modify: `src/features/backup/client.ts`
- Modify: `src/features/backup/__tests__/contracts.test.ts`
- Modify: `src/features/backup/__tests__/client.test.ts`

**Interfaces:**
- Consumes: `BackupRuleDraft`, `BackupRule`, `RuleVolumeMatch`, `SourceId`, app snapshots.
- Produces:

```rust
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BackupRuleDto {
    pub id: String,
    pub name: String,
    pub archive_directory_name: String,
    pub archive_directory_locked: bool,
    pub enabled: bool,
    pub volume_name_glob: String,
    pub required_path_globs: Vec<String>,
    pub backup_file_globs: Vec<String>,
    pub session_directory_globs: Vec<String>,
    pub filename_prefix: String,
    pub filename_suffix: String,
    pub is_dji_preset: bool,
    pub archived: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceSnapshotDto {
    pub source_id: String,
    pub rule_name: String,
    pub volume_name: String,
    pub legacy_slot: Option<String>,
    pub mounted: bool,
    pub phase: BackupPhase,
    pub progress: ProgressDto,
    pub retirement_outcome: DeletionPhase,
    pub deletion_ready: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RuleTestResultDto {
    pub matched_volumes: Vec<String>,
    pub matched_file_count: u64,
    pub conflict_rule_names: Vec<String>,
}
```

Commands become the existing non-pairing commands plus exact fixed commands `save_backup_rule`, `archive_backup_rule`, `restore_dji_rule`, and `test_backup_rule`. `prepare_trash` accepts `source_id`; arbitrary setting keys and paths remain impossible.

- [ ] **Step 1: Write failing Rust DTO and command-surface tests**

Assert snapshot JSON contains `sources` and `backup_rules`, not fixed `transmitters` or pairing candidates. Assert source IDs are UUIDs and forbidden fields remain absent. Update the exact command list: remove `pair_devices`; add the four rule commands; reject any command named `set_setting`, `scan_path`, or `open_path`.

- [ ] **Step 2: Write failing TypeScript schema and client tests**

Define wished-for strict Zod schemas for rules, dynamic sources, rule test results, and trash proposals. Parse updated fixtures and reject an extra `volume_uuid`, absolute path, hash, or arbitrary pattern field. Assert the client sends only validated rule DTOs and opaque IDs to the exact command names.

- [ ] **Step 3: Run the RED tests**

```bash
cargo test --manifest-path src-tauri/Cargo.toml -p dji-mic-backup --test rule_commands
cargo test --manifest-path src-tauri/Cargo.toml -p dji-mic-backup --test commands
pnpm test -- src/features/backup/__tests__/contracts.test.ts src/features/backup/__tests__/client.test.ts
```

Expected: fixed transmitter/pairing contracts and missing rule commands fail.

- [ ] **Step 4: Implement narrow backend commands**

`save_backup_rule` validates and persists one draft, invalidates proposals for changed rules, refreshes the snapshot, and queues no backup until the next lifecycle/rescan. `archive_backup_rule` uses a UUID-only ID. `restore_dji_rule` takes no user configuration. `test_backup_rule` compiles an unsaved draft and evaluates observed volumes read-only, returning display names and counts only.

- [ ] **Step 5: Replace fixed frontend contracts**

Use strict Zod objects, bounded arrays matching backend limits, UUID source/rule IDs, and a dynamic `sources` array. Preserve current progress, error, activity, settings, notification, and artifact-format fields. Replace `activity.transmitter` and `publicError.transmitter` with nullable `source_id` plus nullable safe `source_label`. Reduce setup state to `needs_destination` or `ready`; the built-in rule eliminates mandatory two-device pairing.

- [ ] **Step 6: Run contract, command, and privacy suites**

```bash
cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check
cargo test --manifest-path src-tauri/Cargo.toml -p dji-mic-backup --test rule_commands
cargo test --manifest-path src-tauri/Cargo.toml -p dji-mic-backup --test commands
cargo test --manifest-path src-tauri/Cargo.toml -p dji-mic-backup --test ipc_contract
pnpm test -- src/features/backup/__tests__/contracts.test.ts src/features/backup/__tests__/client.test.ts
pnpm typecheck
git diff --check
```

- [ ] **Step 7: Commit and push the IPC checkpoint**

Explicitly stage the Task 8 files, commit `feat(api): expose validated backup rule commands`, push, fetch, and prove `0 0` parity.

---

### Task 9: Build the rule editor and dynamic source UI

**Files:**
- Create: `src/features/backup/RuleEditor.tsx`
- Create: `src/features/backup/RuleList.tsx`
- Create: `src/features/backup/__tests__/RuleEditor.test.tsx`
- Create: `src/features/backup/__tests__/RuleList.test.tsx`
- Modify: `src/features/backup/SettingsApp.tsx`
- Modify: `src/features/backup/BackupPopover.tsx`
- Modify: `src/features/backup/SetupFlow.tsx`
- Modify: `src/features/backup/TrashDialog.tsx`
- Modify: `src/features/backup/format.ts`
- Modify: `src/features/backup/__tests__/SettingsApp.test.tsx`
- Modify: `src/features/backup/__tests__/BackupPopover.test.tsx`
- Modify: `src/features/backup/__tests__/SetupFlow.test.tsx`
- Modify: `src/features/backup/__tests__/TrashDialog.test.tsx`
- Modify: `src/index.css`
- Modify: `src/preview.tsx`

**Interfaces:**
- Consumes: `BackupRuleDto`, `RuleTestResultDto`, dynamic `SourceSnapshot`, and Task 8 client actions.
- Produces:

```tsx
export interface RuleEditorProps {
  rule: BackupRule | null;
  connectedVolumeNames: string[];
  busy: boolean;
  onCancel: () => void;
  onSave: (draft: BackupRuleDraft) => Promise<void>;
  onTest: (draft: BackupRuleDraft) => Promise<RuleTestResult>;
}

export interface RuleListProps {
  rules: BackupRule[];
  busyRuleId: string | null;
  onAdd: () => void;
  onEdit: (rule: BackupRule) => void;
  onDuplicate: (rule: BackupRule) => void;
  onArchive: (ruleId: string) => Promise<void>;
  onRestoreDji: () => Promise<void>;
}
```

- [ ] **Step 1: Write failing editor validation and preview tests**

Render a new rule, fill `Zoom H1n`, `ZOOM_*`, required `RECORD/**`, backup `RECORD/**/*.WAV`, prefix `zoom-`, and suffix `-field`. Assert the preview shows `Zoom H1n/YYYY/MM/YYMMDD-zoom-ZOOM0001-field.m4a`, the save payload preserves ordered pattern arrays, and adding/removing repeatable fields is keyboard reachable. Assert malformed `[abc`, empty backup patterns, `/` in suffix, and 65-character names display field errors and never call `onSave` or `onTest`.

- [ ] **Step 2: Write failing rule-list behavior tests**

Assert the DJI preset appears first with `기본 규칙`, can be disabled or edited, and exposes `기본값 복원`. Assert duplicate creates an enabled draft with no ID, unlocked archive directory, and `복사본` name. Assert an evidenced rule uses an archive confirmation and an archived preset can be restored without deleting evidence.

- [ ] **Step 3: Write failing dynamic popover/setup/trash tests**

Render zero, one, and three dynamic sources. Require `0개 녹음기 연결됨`, rule and volume labels, independent phases, and no `/2` copy. Setup must require only destination selection, then explain that the DJI preset is ready and other recorders are added in settings; it must not show TX01/TX02 pairing controls. Trash dialog titles use the safe source label, never `transmitter`.

- [ ] **Step 4: Run the RED tests**

```bash
pnpm test -- src/features/backup/__tests__/RuleEditor.test.tsx src/features/backup/__tests__/RuleList.test.tsx
pnpm test -- src/features/backup/__tests__/SettingsApp.test.tsx src/features/backup/__tests__/BackupPopover.test.tsx src/features/backup/__tests__/SetupFlow.test.tsx src/features/backup/__tests__/TrashDialog.test.tsx
```

Expected: new components are absent and fixed transmitter copy fails.

- [ ] **Step 5: Implement the settings rule list and editor**

Keep local drafts separate from persisted snapshots. Use repeatable labeled inputs, inline validation mirroring backend limits, pending state per action, and persisted response replacement after saves. `Test against connected disks` displays matched volume names/counts or conflict names and never triggers backup. Lock the archive-directory input when `archive_directory_locked` is true.

- [ ] **Step 6: Replace fixed pairing and transmitter presentation**

Render snapshot sources keyed by opaque source ID. Derive mounted counts dynamically. Replace setup pairing with destination-only onboarding plus the built-in DJI rule explanation. Keep per-source progress, deletion readiness, failure/log actions, and activity labels. Preserve responsive scroll behavior for the taller settings window.

- [ ] **Step 7: Run frontend suite, type checking, and production build**

```bash
pnpm test
pnpm typecheck
pnpm build
git diff --check
```

- [ ] **Step 8: Run browser preview smoke checks**

Start `pnpm dev`, open `preview.html?state=complete`, `copying`, `error`, and `rules` in the configured browser harness, and verify no console errors, horizontal overflow, inaccessible unlabeled form controls, fixed two-source copy, or hidden save/conflict messages. Capture screenshots for the rule list and editor review, then stop the server.

- [ ] **Step 9: Commit and push the rule-UI checkpoint**

Explicitly stage the Task 9 files, commit `feat(ui): add recorder backup rule editor`, push, fetch, and prove `0 0` parity.

---

### Task 10: Rename the product and migrate legacy app data atomically

**Files:**
- Create: `src-tauri/src/legacy_app_data.rs`
- Create: `src-tauri/tests/legacy_app_data.rs`
- Modify: `package.json`
- Modify: `pnpm-lock.yaml`
- Modify: `src-tauri/Cargo.toml`
- Modify: `src-tauri/Cargo.lock`
- Modify: `src-tauri/tauri.conf.json`
- Modify: `src-tauri/src/main.rs`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/src/tray.rs`
- Modify: `src-tauri/src/commands.rs`
- Modify: `src-tauri/src/orchestrator.rs`
- Modify: `src-tauri/src/failure_reporter.rs`
- Modify: `src-tauri/crates/backup-core/Cargo.toml`
- Modify: `src-tauri/crates/backup-core/src/lib.rs`
- Modify: `src-tauri/crates/backup-core/src/ledger.rs`
- Modify: `src-tauri/crates/backup-core/tests/fat32_deletion_acceptance.rs`
- Modify: `src-tauri/crates/backup-core/tests/ledger_recovery.rs`
- Modify: `src-tauri/tests/apple_audio_tools.rs`
- Modify: `src-tauri/tests/artifact_pipeline.rs`
- Modify: `src-tauri/tests/commands.rs`
- Modify: `src-tauri/tests/concurrent_settings.rs`
- Modify: `src-tauri/tests/conversion_cohort.rs`
- Modify: `src-tauri/tests/failure_reporting.rs`
- Modify: `src-tauri/tests/fat32_trash_acceptance.rs`
- Modify: `src-tauri/tests/ipc_contract.rs`
- Modify: `src-tauri/tests/macos_trash.rs`
- Modify: `src-tauri/tests/rescan_scheduler.rs`
- Modify: `src-tauri/tests/verify_backup_script.rs`
- Modify: `src-tauri/tests/rule_device_lifecycle.rs`
- Modify: `src-tauri/tests/multi_rule_backup.rs`
- Modify: `src-tauri/tests/rule_trash_flow.rs`
- Modify: `src-tauri/tests/rule_commands.rs`
- Modify: `src/features/backup/BackupApp.tsx`
- Modify: `src/features/backup/BackupPopover.tsx`
- Modify: `src/features/backup/SettingsApp.tsx`
- Modify: `src/features/backup/__tests__/BackupApp.test.tsx`
- Modify: `index.html`
- Modify: `preview.html`
- Modify: `README.md`
- Modify: `scripts/accept-deletion-fixture.sh`
- Modify: `scripts/package-local.sh`
- Modify: `scripts/install-local.sh`
- Modify: `scripts/verify-backup.sh`
- Modify: `scripts/check.sh`

**Interfaces:**
- Consumes: legacy path `/Users/channprj/Library/Application Support/com.channprj.DJIMicBackup/ledger.sqlite3`, new app-data/log directories from Tauri, `Ledger::open` migration validation.
- Produces:

```rust
pub const PRODUCT_NAME: &str = "Backup Mic";
pub const LEGACY_BUNDLE_IDENTIFIER: &str = "com.channprj.DJIMicBackup";
pub const CURRENT_BUNDLE_IDENTIFIER: &str = "com.channprj.BackupMic";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LegacyMigrationOutcome { NotNeeded, Migrated }

pub fn migrate_legacy_ledger(
    legacy_ledger: &Path,
    current_ledger: &Path,
) -> Result<LegacyMigrationOutcome, CoreError>;

pub fn prepare_app_data(
    current_app_data: &Path,
    legacy_app_data: &Path,
) -> Result<LegacyMigrationOutcome, CoreError>;
```

- [ ] **Step 1: Write failing closed/WAL/corrupt migration tests**

Create a legacy ledger with persisted destination and rule/evidence fixtures. Test a clean closed database and a database with committed WAL content. Require an owner-only temporary snapshot, `quick_check = ok`, all schema migrations, atomic target installation, preserved destination/evidence, and byte-for-byte unchanged legacy files. For corrupt input or an occupied non-SQLite target, assert no authoritative new ledger is installed and the legacy source remains unchanged.

- [ ] **Step 2: Write failing product identity tests**

Update runtime/config/package tests to require `Backup Mic`, `Backup Mic.app`, `backup-mic`, `backup_mic_lib`, `com.channprj.BackupMic`, `~/Documents/Backup Mic`, and the new fallback log root. Assert legacy identifiers appear only in migration and exact old-bundle installer handling. Require package scripts to verify the renamed executable and bundle identifier.

- [ ] **Step 3: Run the RED tests**

```bash
cargo test --manifest-path src-tauri/Cargo.toml -p dji-mic-backup --test legacy_app_data
cargo test --manifest-path src-tauri/Cargo.toml -p dji-mic-backup --test runtime_shell
pnpm test
```

Expected: migration helper is absent and old identities remain public.

- [ ] **Step 4: Implement SQLite online-backup migration**

Enable rusqlite's pinned `backup` feature. Open the legacy source through SQLite, copy committed state into a `0600` temporary database in the new app-data directory, synchronize it and its parent, open it through `Ledger::open` to run migrations and integrity checks, close it, and use no-clobber atomic rename to `ledger.sqlite3`. Never rename, checkpoint, quarantine, or mutate the legacy source.

- [ ] **Step 5: Run app-data preparation before opening the new ledger**

Resolve the exact legacy app-data path through the user's Library/Application Support directory. Call `prepare_app_data` only when the current ledger is absent. Route failures to `~/Library/Logs/com.channprj.BackupMic` with sanitized codes. Preserve `destination_path`; use `documents.join("Backup Mic")` only when no configured destination exists.

- [ ] **Step 6: Rename code, metadata, scripts, and public copy**

Rename the root Rust package/lib imports and executable, npm package, Tauri product/window titles, bundle identifier, thread labels, tray copy, notification copy, UI headings, active docs references, and package/install expectations. Keep `backup-core` unchanged. Update `check.sh` package selectors from `dji-mic-backup` to `backup-mic`.

- [ ] **Step 7: Make the installer transition exact and recoverable**

Stage `Backup Mic.app` under an owner-only temp directory. Verify it before touching installed apps. Ask both exact bundle IDs to quit. Move existing exact `Backup Mic.app` and legacy `DJI Mic Backup.app` into rollback slots, install and verify the new app and executable hash, then move prior bundles to macOS Trash. On any failure, remove the failed new bundle from the target and restore both prior exact bundles.

- [ ] **Step 8: Run renamed package and migration suites**

```bash
cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check
cargo test --manifest-path src-tauri/Cargo.toml -p backup-core --test ledger_recovery
cargo test --manifest-path src-tauri/Cargo.toml -p backup-mic --test legacy_app_data
cargo test --manifest-path src-tauri/Cargo.toml -p backup-mic --test runtime_shell
cargo test --manifest-path src-tauri/Cargo.toml --workspace --all-targets
cargo clippy --manifest-path src-tauri/Cargo.toml --workspace --all-targets -- -D warnings
pnpm test
pnpm typecheck
pnpm build
git diff --check
```

- [ ] **Step 9: Audit allowed legacy and DJI strings**

Run targeted `rg` searches for `DJI Mic Backup`, `DJIMicBackup`, `dji-mic-backup`, and `dji_mic_backup_lib`. Require every remaining current-code hit to be one of: legacy migration identifier/path, old installed bundle handling, DJI preset/device help, fixture name, or historical spec/plan. Fix every other active product-surface hit.

- [ ] **Step 10: Commit and push the breaking rename checkpoint**

Explicitly stage the Task 10 files, commit `feat(app)!: rename product to Backup Mic`, push, fetch, and prove `0 0` parity.

---

### Task 11: Relocate legacy DJI archives under the preset directory

**Files:**
- Create: `src-tauri/crates/backup-core/tests/rule_layout_migration.rs`
- Modify: `src-tauri/crates/backup-core/src/layout.rs`
- Modify: `src-tauri/crates/backup-core/src/ledger.rs`
- Modify: `src-tauri/src/app_state.rs`
- Modify: `src-tauri/src/orchestrator.rs`
- Modify: `src-tauri/tests/verify_backup_script.rs`
- Modify: `scripts/verify-backup.sh`
- Modify: `README.md`

**Interfaces:**
- Consumes: DJI preset rule ID/archive directory, verified recording and additional-file evidence, existing verified relocation and `TrashAdapter`.
- Produces:

```rust
pub fn migrate_legacy_rule_layout(
    destination_root: &Path,
    ledger: &mut Ledger,
    rule: &BackupRule,
    trash: &dyn TrashAdapter,
    cancellation: &CancellationToken,
) -> Result<Vec<LayoutMigration>, CoreError>;
```

- [ ] **Step 1: Write failing calendar-artifact migration tests**

Seed verified WAV and M4A artifacts at `2026/08/260810-T01_...` and `2026/08/260810-T02_...`. Assert they migrate to `DJI Mic Mini 2S/2026/08/...`, preserve bytes/hash/audio evidence, move old files through the fake Trash adapter, update ledger paths only after target verification, and remove only empty legacy month/year directories.

- [ ] **Step 2: Write failing extras, collision, repair, and idempotency tests**

Seed `source-extras` evidence, a same-content target, a different-content collision, a missing old file with verified target, and a mismatched old artifact. Require extras to move beneath the preset, same content to reuse, collision to gain a hash suffix, missing-old evidence to repair its path, mismatched old content to remain untouched, and a second migration call to return no work.

- [ ] **Step 3: Run the RED tests**

```bash
cargo test --manifest-path src-tauri/Cargo.toml -p backup-core --test rule_layout_migration
cargo test --manifest-path src-tauri/Cargo.toml -p backup-mic --test verify_backup_script
```

Expected: current layout migration knows calendar flattening but not rule directories.

- [ ] **Step 4: Implement verified rule-directory relocation**

Reuse canonical-root checks, synchronized temporary copy, size/SHA verification, no-clobber finalization, Foundation Trash adapter, monotonic ledger relocation, and empty-directory cleanup. Process one artifact independently so interruption leaves either old verified evidence or a new verified target; never delete an unverified old artifact.

- [ ] **Step 5: Make startup migration ordered and retryable**

After ledger schema/app-data migration and before new backup planning, run current legacy-calendar normalization if needed, then rule-directory relocation. Report per-artifact sanitized failures, continue safe independent items, and retain skipped evidence for the next launch. Do not mark the migration complete while eligible unmigrated verified artifacts remain.

- [ ] **Step 6: Generalize the independent verifier and README**

Change verifier source arguments from `TX01=/Volumes/...` to `<rule-name>=<mounted-root>` while retaining explicit `--ledger` and `--diagnostic`. Verify every live selected rule file against source evidence and every durable artifact beneath its locked archive directory. Document Backup Mic setup, rule fields/examples, DJI preset, conversion/Trash behavior, migration, build/package/install paths, and allowed hardware-proof limitations.

- [ ] **Step 7: Run layout, verifier, docs, and full source gates**

```bash
cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check
cargo test --manifest-path src-tauri/Cargo.toml -p backup-core --test rule_layout_migration
cargo test --manifest-path src-tauri/Cargo.toml -p backup-core --test layout_migration
cargo test --manifest-path src-tauri/Cargo.toml -p backup-mic --test verify_backup_script
./scripts/check.sh
git diff --check
```

- [ ] **Step 8: Commit and push the archive-migration checkpoint**

Explicitly stage the Task 11 files, commit `feat(migration): preserve DJI backups under rule archive`, push, fetch, and prove `0 0` parity.

---

### Task 12: Prove disk-image automation, package, install, and release

**Files:**
- Create: `scripts/accept-rule-fixtures.sh`
- Create: `src-tauri/tests/rule_volume_acceptance.rs`
- Modify: `scripts/check.sh`
- Modify: `scripts/package-local.sh`
- Modify: `scripts/install-local.sh`
- Modify: `README.md`
- Generated by Headatever only: `VERSION`
- Modified by Headatever only: `package.json`
- Modified by Headatever only: `src-tauri/Cargo.toml`
- Modified by Headatever only: `src-tauri/crates/backup-core/Cargo.toml`
- Modified by Headatever only: `src-tauri/tauri.conf.json`
- Modified by Headatever only: `src-tauri/Cargo.lock`

**Interfaces:**
- Consumes: production rule matcher, Disk Arbitration monitor, verified backup pipeline, disposable disk images, packaging/install scripts, Headatever.
- Produces: reproducible fixture evidence, one verified release commit/tag, packaged app/DMG checksums, installed `/Users/channprj/Applications/Backup Mic.app`, and final Git parity.

- [ ] **Step 1: Write a failing two-volume acceptance test**

In `rule_volume_acceptance.rs`, require environment variables pointing to two mounted fixture roots named `ZOOM_RULETEST` and `SONY_RULETEST`. Insert rules `ZOOM_* + RECORD/**/*.WAV` and `SONY_* + REC_FILE/**/*.WAV`, feed their real mounted descriptors through `DeviceRegistry`/rule matching, run production stable scan and backup, and assert isolated rule folders, prefix/suffix names, equal source-copy hashes, verified M4A evidence, and independent completed source batches.

- [ ] **Step 2: Run the RED acceptance test without fixtures**

```bash
cargo test --manifest-path src-tauri/Cargo.toml -p backup-mic --test rule_volume_acceptance -- --ignored
```

Expected: the test refuses with the exact missing fixture marker/environment message; it must never fall back to a real mounted recorder.

- [ ] **Step 3: Implement the disposable disk-image harness**

`accept-rule-fixtures.sh` creates two fresh 64 MiB MS-DOS FAT32 images in a `mktemp -d` directory, labels them exactly `ZOOM_RULETEST` and `SONY_RULETEST`, writes sentinel markers plus valid PCM WAVs under the two distinct layouts, records their device nodes, and exports canonical mount roots to the ignored test. It refuses existing same-name mounts, internal disks, connected DJI volumes, absent sentinels, or unexpected capacity. Its trap detaches only recorded device nodes and moves temporary images to Trash or removes only its exact `mktemp` directory after detach.

- [ ] **Step 4: Run disposable mount and deletion acceptance**

```bash
./scripts/accept-rule-fixtures.sh
./scripts/accept-deletion-fixture.sh
```

Expected: both scripts report verified counts and hashes without printing source UUIDs/full hashes; source retirement occurs only inside the exact disposable deletion image.

- [ ] **Step 5: Run the complete fresh source gate**

```bash
./scripts/check.sh
git diff --check
git status --short --branch
```

Expected: all Rust tests, Clippy, frontend tests, type checking, production frontend build, privacy scans, rule tests, migration tests, verifier tests, and runtime-shell guards pass; only intentional Task 12 changes remain.

- [ ] **Step 6: Commit and push fixture/release-readiness code**

Explicitly stage only `scripts/accept-rule-fixtures.sh`, `src-tauri/tests/rule_volume_acceptance.rs`, `scripts/check.sh`, package/install script adjustments, and README adjustments. Commit `test(release): verify rule-based recorder backups`, push, fetch, and prove `0 0` parity.

- [ ] **Step 7: Create the versioned release checkpoint through Headatever**

Invoke the `headatever` skill. Run `headatever init 0 --dry-run`, inspect the exact version files, then run `headatever init 0 --push`. Do not hand-edit `VERSION`, do not restage generated version files into another commit, and do not create or move tags manually. Record the generated release commit and annotated tag, fetch, and prove branch parity `0 0` plus tag presence on `origin`.

- [ ] **Step 8: Package and verify the renamed release**

```bash
./scripts/package-local.sh "$(tr -d '\r\n' < VERSION)"
```

Require exactly one `Backup Mic.app` and one DMG, `com.channprj.BackupMic`, executable `backup-mic`, matching version, macOS 13 minimum, arm64, deep strict ad-hoc signature, a valid DMG, and recorded SHA-256 values.

- [ ] **Step 9: Install atomically and verify installed parity**

```bash
./scripts/install-local.sh "src-tauri/target/release/bundle/macos/Backup Mic.app" "$(tr -d '\r\n' < VERSION)"
open -a "/Users/channprj/Applications/Backup Mic.app"
```

Require the installed executable hash to equal the packaged executable hash, the old exact bundle to be absent from Applications after recoverable Trash migration, and the new app to launch. Verify the settings snapshot contains the DJI preset and the migrated persisted destination. If native GUI automation or physical recorders are unavailable, report those proof gaps explicitly rather than treating fixture evidence as hardware evidence.

- [ ] **Step 10: Perform the requirement-by-requirement completion audit**

Re-read the approved design and this plan. For rename, rule CRUD/test, glob matching, dynamic sources, automatic rescan, prefix/suffix naming, DJI preset/migration, verified copy/conversion, recoverable Trash, independent failure, installed artifact, and `$gcpr`, point to the exact test/output/artifact that proves it. Run:

```bash
git log --oneline 3fa889d..HEAD
git status --short --branch
git rev-list --left-right --count HEAD...@{u}
git ls-remote --tags origin
```

Expected: intended checkpoint history, clean tree, `0 0`, and the Headatever tag on the live remote. Any fix discovered here becomes a new verified Conventional Commit and ordinary push before completion.
