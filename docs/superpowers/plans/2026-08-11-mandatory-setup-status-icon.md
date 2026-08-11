# Mandatory Setup, Destination Feedback, and Status Icon Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Require every fresh installation to choose a backup destination and review real settings before backup, show the persisted destination immediately in Settings, and use the Backup Mic face as an automatically inverted macOS status-bar template icon.

**Architecture:** Store a typed initial-setup marker beside the destination in the existing SQLite settings table, derive a three-state setup DTO in Rust, and keep every manual or automatic backup path fail-closed until it is `ready`. Return one revisioned snapshot containing a display-only destination, let React render a blocking two-stage flow, and load a dedicated embedded template PNG for the tray instead of the opaque bundle icon.

**Tech Stack:** Rust 2024, Tauri 2.11, SQLite/rusqlite, React 19, TypeScript 7, Zod 4, Vitest, Testing Library, macOS AppKit template images, shell acceptance fixtures, Headatever.

## Global Constraints

- Apply mandatory onboarding only to fresh ledgers without a persisted destination; existing and migrated destinations with no marker are `ready`.
- Setup states are exactly `needs_destination`, `needs_settings_review`, and `ready`.
- A fresh destination and `settings_review_pending` marker commit in one SQLite transaction; runtime state changes only after commit.
- Cancel remains allowed, but backup UI, Settings entry, destination/log actions, scanning, backup, and Trash authority remain unavailable until setup is `ready`; quit remains available.
- Existing `ready` users stay ready when changing destination, and their settings, rules, ledger evidence, and migration state remain unchanged.
- The settings review exposes automatic backup, AAC-LC 128 kbps M4A conversion, automatic macOS Trash, login start, the DJI Mic Mini 2S preset, and the persisted destination.
- Automatic Trash remains off by default and requires the existing confirmation; all copy, conversion, source-revalidation, and complete-session barriers remain unchanged.
- `destination_display` is display-only: use `~/...` below the home directory, allow canonical `/Volumes/...` locations, strip control characters, and never treat it as filesystem authority.
- The status item uses the central Backup Mic face as a 36x36 RGBA template mask; AppKit performs light/dark recoloring without theme listeners or asset swapping.
- macOS 13 is the minimum version; the packaged executable must include arm64 and a deep strict ad-hoc signature.
- Follow `$gcpr`: stage explicit paths, use Conventional Commit checkpoints, push immediately, fetch, and require `0 0` upstream parity. Never rewrite or force-push history.
- Repository instructions require inline sequential execution; do not dispatch subagents.
- Before production code in every task, write and run the named failing test; after implementation, rerun focused and affected suites.

---

### Task 1: Persist a typed initial-setup marker atomically

**Files:**
- Create: `src-tauri/crates/backup-core/src/initial_setup.rs`
- Create: `src-tauri/crates/backup-core/tests/initial_setup.rs`
- Modify: `src-tauri/crates/backup-core/src/lib.rs`
- Modify: `src-tauri/crates/backup-core/src/ledger.rs`

**Interfaces:**
- Consumes: existing `settings(key, value_json, updated_at)` table and `CoreError::LedgerCorrupt` / `CoreError::InvalidRequest`.
- Produces:

```rust
pub const DESTINATION_SETTING: &str = "destination_path";
pub const INITIAL_SETUP_SETTING: &str = "initial_setup_state";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InitialSetupMarker {
    SettingsReviewPending,
    Complete,
}

impl InitialSetupMarker {
    pub fn encode(self) -> &'static str;
    pub fn decode(value: Option<String>) -> Result<Option<Self>, CoreError>;
}

impl Ledger {
    pub fn initial_setup_marker(&self) -> Result<Option<InitialSetupMarker>, CoreError>;
    pub fn persist_destination(
        &mut self,
        destination_json: &str,
        require_settings_review: bool,
        updated_at: &str,
    ) -> Result<(), CoreError>;
    pub fn complete_initial_setup(&mut self, updated_at: &str) -> Result<(), CoreError>;
}
```

- [ ] **Step 1: Write failing fresh, legacy, invalid-marker, and rollback tests**

In `initial_setup.rs`, assert `None` on a new ledger; `SettingsReviewPending` after `persist_destination(..., true, ...)`; `Complete` only after `complete_initial_setup`; and unchanged `None` after `persist_destination(..., false, ...)` for a compatible legacy-ready ledger. Write an invalid raw value with `set_setting(INITIAL_SETUP_SETTING, "\"unknown\"", ...)` and assert `LedgerCorrupt`.

Use a second `rusqlite::Connection` to install a trigger that raises on the marker insert, then assert the destination insert from the same transaction rolls back:

```rust
connection.execute_batch(
    "CREATE TRIGGER fail_setup_marker BEFORE INSERT ON settings
     WHEN NEW.key = 'initial_setup_state'
     BEGIN SELECT RAISE(ABORT, 'fixture failure'); END;",
)?;
let error = ledger.persist_destination("\"/tmp/Backup Mic\"", true, now).unwrap_err();
assert!(matches!(error, CoreError::Ledger(_)));
assert_eq!(ledger.setting(DESTINATION_SETTING).unwrap(), None);
```

- [ ] **Step 2: Run the RED test**

```bash
cargo test --manifest-path src-tauri/Cargo.toml -p backup-core --test initial_setup
```

Expected: compilation fails because `backup_core::initial_setup` and the three ledger methods do not exist.

- [ ] **Step 3: Implement strict marker encoding and decoding**

Encode markers as JSON strings `"settings_review_pending"` and `"complete"`. Decode `None` as no marker and reject malformed JSON, JSON non-strings, or any other string with `CoreError::LedgerCorrupt`.

```rust
match serde_json::from_str::<String>(&value)?.as_str() {
    "settings_review_pending" => Ok(Some(Self::SettingsReviewPending)),
    "complete" => Ok(Some(Self::Complete)),
    _ => Err(CoreError::LedgerCorrupt),
}
```

- [ ] **Step 4: Implement destination and completion transactions**

`persist_destination` starts one transaction, upserts `destination_path`, conditionally upserts the pending marker, and commits. `complete_initial_setup` updates only a row currently equal to the encoded pending value; require exactly one changed row before commit.

```rust
let changed = transaction.execute(
    "UPDATE settings SET value_json = ?1, updated_at = ?2
     WHERE key = ?3 AND value_json = ?4",
    params![complete, updated_at, INITIAL_SETUP_SETTING, pending],
)?;
if changed != 1 { return Err(CoreError::InvalidRequest); }
```

- [ ] **Step 5: Run core persistence gates**

```bash
cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check
cargo test --manifest-path src-tauri/Cargo.toml -p backup-core --test initial_setup
cargo test --manifest-path src-tauri/Cargo.toml -p backup-core --test ledger_recovery
cargo test --manifest-path src-tauri/Cargo.toml -p backup-core --lib
git diff --check
```

- [ ] **Step 6: Commit and push the persistence checkpoint**

Explicitly stage the four Task 1 files, commit `feat(setup): persist mandatory setup state`, push `main`, fetch, and require `git rev-list --left-right --count HEAD...@{u}` to print `0 0`.

---

### Task 2: Enforce setup state and destination feedback in Rust

**Files:**
- Modify: `src-tauri/src/dto.rs`
- Modify: `src-tauri/src/app_state.rs`
- Modify: `src-tauri/src/commands.rs`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/tests/commands.rs`
- Modify: `src-tauri/tests/ipc_contract.rs`
- Modify: `src-tauri/tests/rule_device_lifecycle.rs`

**Interfaces:**
- Consumes: `InitialSetupMarker`, `Ledger::persist_destination`, `Ledger::complete_initial_setup`, current operation reservation and preference-save serialization.
- Produces:

```rust
#[serde(rename_all = "snake_case")]
pub enum SetupStateDto {
    NeedsDestination,
    NeedsSettingsReview,
    Ready,
}

pub struct AppSnapshotDto {
    // existing fields remain strict and unchanged
    pub destination_display: Option<String>,
    pub setup_state: SetupStateDto,
}

impl AppState {
    pub fn persist_destination_for_state(
        &self,
        app: &AppHandle,
        destination: PathBuf,
        occurred_at: &str,
    ) -> Result<AppSnapshotDto, CoreError>;
    pub fn complete_initial_setup_for_state(
        &self,
        app: &AppHandle,
        occurred_at: &str,
    ) -> Result<AppSnapshotDto, CoreError>;
}

#[tauri::command]
pub async fn complete_initial_setup(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<AppSnapshotDto, PublicError>;
```

- [ ] **Step 1: Write failing setup-state and display-path tests**

Add AppState tests for the exact compatibility matrix:

```rust
assert_state(false, None, SetupStateDto::NeedsDestination);
assert_state(true, None, SetupStateDto::Ready);
assert_state(true, Some(SettingsReviewPending), SetupStateDto::NeedsSettingsReview);
assert_state(true, Some(Complete), SetupStateDto::Ready);
```

Assert `destination_display_for(Path::new("/Users/example/Documents/Backup Mic"), Some(Path::new("/Users/example"))) == "~/Documents/Backup Mic"`, an external destination remains `/Volumes/Recorder Backups/Backup Mic`, and an unconfigured internal fallback yields `None`.

Add tests that pending setup makes `backup_is_ready()` false even with a matched source; device arrival does not schedule automatic backup; completing setup without a pending marker fails; and persistence updates the displayed path before returning and survives reopening the ledger.

- [ ] **Step 2: Run the RED Rust tests**

```bash
cargo test --manifest-path src-tauri/Cargo.toml -p backup-mic --lib app_state::tests::pending_setup_blocks_every_backup_path
cargo test --manifest-path src-tauri/Cargo.toml -p backup-mic --lib dto::tests::destination_display_is_home_relative_or_external_absolute
cargo test --manifest-path src-tauri/Cargo.toml -p backup-mic --test rule_device_lifecycle
```

Expected: compilation or assertions fail because the third setup state, destination display, and completion command are absent.

- [ ] **Step 3: Derive the startup state and safe display value**

Read `ledger.initial_setup_marker()` inside `AppState::new_with_failure_root`. Derive setup state from destination configuration first, then marker compatibility. Populate `destination_display` only for configured destinations. Remove setup-state mutation from `sync_pairing_snapshot`; pairing must never complete onboarding.

The display helper filters control characters and takes at most 2,048 Unicode scalar values. macOS canonical paths fit below this bound, so a valid final component is not truncated in practice.

- [ ] **Step 4: Make destination persistence commit before runtime publication**

Replace the command's direct `set_setting` plus `set_destination` pair with `persist_destination_for_state`. Determine `require_settings_review` from the current state, serialize the canonical path, commit through the ledger, then invalidate deletion proposals and update destination, generation, display, and setup state under the runtime lock.

```rust
let next_setup = match current_setup {
    SetupStateDto::NeedsDestination => SetupStateDto::NeedsSettingsReview,
    SetupStateDto::NeedsSettingsReview => SetupStateDto::NeedsSettingsReview,
    SetupStateDto::Ready => SetupStateDto::Ready,
};
```

Only `Ready` may retain the existing post-change automatic-backup behavior. Dialog cancellation returns the unchanged snapshot without a success signal.

- [ ] **Step 5: Add completion and fail-closed backup gates**

Register `complete_initial_setup` as command 18. Serialize it with `preference_save`, require `NeedsSettingsReview`, require the destination still be a directory, persist `Complete`, then publish `Ready`. After persistence and publication, schedule existing automatic backup only if `automatic_backup_enabled()` and `backup_is_ready()` are both true.

Include `setup_state == Ready` in `backup_requirements_met` and use that helper in both `backup_is_ready` and `handle_mounted` scheduling. `orchestrator::start_backup` remains the final manual/automatic gate.

- [ ] **Step 6: Update the narrow IPC contract**

Keep all existing commands and add only `complete_initial_setup`. In fixture privacy checks, remove and validate `destination_display` separately before scanning the remaining snapshot for source paths, UUIDs, and hashes. Accept `~/...` and `/Volumes/...` only as display data; never add a path argument to a command.

- [ ] **Step 7: Run focused and crate gates**

```bash
cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check
cargo test --manifest-path src-tauri/Cargo.toml -p backup-mic --lib
cargo test --manifest-path src-tauri/Cargo.toml -p backup-mic --test commands
cargo test --manifest-path src-tauri/Cargo.toml -p backup-mic --test ipc_contract
cargo test --manifest-path src-tauri/Cargo.toml -p backup-mic --test rule_device_lifecycle
cargo clippy --manifest-path src-tauri/Cargo.toml -p backup-mic --all-targets -- -D warnings
git diff --check
```

- [ ] **Step 8: Commit and push the Rust enforcement checkpoint**

Explicitly stage the seven Task 2 files, commit `feat(setup): enforce reviewed backup destination`, push, fetch, and prove `0 0` parity.

---

### Task 3: Render blocking onboarding and immediate destination feedback

**Files:**
- Modify: `contracts/fixtures/backup-complete.json`
- Modify: `contracts/fixtures/backup-copying.json`
- Modify: `contracts/fixtures/error-destination-full.json`
- Modify: `contracts/fixtures/partial-trash.json`
- Modify: `src/features/backup/contracts.ts`
- Modify: `src/features/backup/client.ts`
- Modify: `src/features/backup/SetupFlow.tsx`
- Modify: `src/features/backup/BackupPopover.tsx`
- Modify: `src/features/backup/SettingsApp.tsx`
- Modify: `src/features/backup/__tests__/contracts.test.ts`
- Modify: `src/features/backup/__tests__/client.test.ts`
- Modify: `src/features/backup/__tests__/SetupFlow.test.tsx`
- Modify: `src/features/backup/__tests__/BackupPopover.test.tsx`
- Modify: `src/features/backup/__tests__/SettingsApp.test.tsx`
- Modify: `src/index.css`

**Interfaces:**
- Consumes: strict `AppSnapshotDto`, existing narrow setting actions, existing automatic-Trash confirmation copy.
- Produces:

```ts
export const appSnapshotSchema = z.object({
  // existing fields
  destination_display: z.string().min(1).max(2048).nullable(),
  setup_state: z.enum(["needs_destination", "needs_settings_review", "ready"]),
}).strict();

export function completeInitialSetup(): Promise<AppSnapshot>;
```

- [ ] **Step 1: Update fixtures and write failing strict-contract tests**

Add `"destination_display": "~/Documents/Backup Mic"` to the four ready fixtures. Assert `needs_settings_review` parses, missing/extra/overlong destination fields fail, and `completeInitialSetup` invokes exactly `complete_initial_setup` with no arguments.

- [ ] **Step 2: Write failing onboarding behavior tests**

Replace the old “requires only a destination” test with three-state coverage:

```tsx
render(<SetupFlow snapshot={needsDestination} actions={actions} />);
expect(screen.getByRole("button", { name: "백업 폴더 선택" })).toBeEnabled();

rerender(<SetupFlow snapshot={needsReview} actions={actions} />);
expect(screen.getByText("~/Documents/Backup Mic")).toBeInTheDocument();
expect(screen.getByRole("switch", { name: "자동으로 백업" })).toBeChecked();
expect(screen.getByRole("button", { name: "설정 확인 완료" })).toBeEnabled();
```

Assert setting commands update from persisted snapshots; enabling Trash first opens the existing confirmation; a failed setting rolls back; completion double-click invokes once; completion failure remains blocked; and successful completion returns `ready`.

- [ ] **Step 3: Write failing popover access-boundary tests**

For both non-ready states, assert the recorder count, Settings button, backup-now, destination, logs, activity, and Trash controls are absent while `Backup Mic` and `앱 종료` remain. For `ready`, assert the current surface is unchanged.

- [ ] **Step 4: Write failing Settings destination tests**

Assert the initial `destination_display` is visible. Make `chooseDestination` resolve a higher-revision snapshot with `~/Backups/New Backup Mic`, then assert that exact path and an `aria-live` `백업 폴더가 변경되었습니다` status appear immediately. Make the command reject and assert the old path remains; model cancel as an unchanged snapshot and assert no false success message.

- [ ] **Step 5: Implement strict contracts and the completion client**

Add the nullable display field and third setup state to Zod, update fixtures, export `completeInitialSetup`, and add it to `backupClient`. Keep `invokeSnapshot` as the only response parser.

- [ ] **Step 6: Expand SetupFlow without creating a generic wizard**

Keep `SetupFlow.tsx` as the one feature component. Maintain a local revisioned `viewSnapshot`, a pending-key set, a completion ref, action error, and automatic-Trash dialog state. Render only the destination card for `needs_destination`; render destination, four switches, DJI preset summary, and `설정 확인 완료` for `needs_settings_review`.

Do not optimistically claim persisted path or completion. Setting switches may use the same narrow optimistic/rollback pattern as Settings, but completion is enabled only when no setting or destination command is pending.

- [ ] **Step 7: Hide every non-quit popover surface until ready**

In `BackupPopover`, branch immediately after the product header. A non-ready snapshot renders `SetupFlow` plus a footer containing only `앱 종료`. Render connection count, Settings, normal status, sources, activity, backup, destination, and logs only in the ready branch.

- [ ] **Step 8: Show persisted destination and success feedback in Settings**

Replace the fixed `선택한 백업 폴더` span with a read-only path element using `overflow-wrap: anywhere`. Compare the returned display value with the previous value: show the live success status only when it changed; an unchanged snapshot represents dialog cancellation and keeps the previous status.

- [ ] **Step 9: Run frontend contract and UI gates**

```bash
pnpm vitest run \
  src/features/backup/__tests__/contracts.test.ts \
  src/features/backup/__tests__/client.test.ts \
  src/features/backup/__tests__/SetupFlow.test.tsx \
  src/features/backup/__tests__/BackupPopover.test.tsx \
  src/features/backup/__tests__/SettingsApp.test.tsx
pnpm typecheck
pnpm build
git diff --check
```

- [ ] **Step 10: Commit and push the onboarding UI checkpoint**

Explicitly stage the fifteen Task 3 files, commit `feat(ui): require first-run settings review`, push, fetch, and prove `0 0` parity.

---

### Task 4: Install the Backup Mic template status icon

**Files:**
- Create: `src-tauri/icons/tray-template.png`
- Modify: `src-tauri/src/tray.rs`

**Interfaces:**
- Consumes: the existing app icon face, Tauri `Image::from_bytes`, and `TrayIconBuilder::icon_as_template(true)`.
- Produces:

```rust
const TRAY_ICON_BYTES: &[u8] = include_bytes!("../icons/tray-template.png");

fn tray_icon() -> tauri::Result<tauri::image::Image<'static>> {
    tauri::image::Image::from_bytes(TRAY_ICON_BYTES)
}
```

- [ ] **Step 1: Read and use the imagegen skill for the visual asset**

Create a transparent monochrome mask derived from the existing center face: opaque rounded face body, transparent background, and transparent eye/mouth holes. Preserve the app icon's expression and remove the outer dark tile. Produce a square source, then ensure the repository asset is exactly 36x36 RGBA without introducing another logo design.

- [ ] **Step 2: Write the failing asset contract test**

In `tray.rs`, decode the included bytes and assert width and height are 36. Inspect `rgba()` and assert at least one alpha value is 0, at least one is 255, the four corners are transparent, both eye centers are transparent, the mouth center is transparent, and a face-body sample is opaque.

```rust
let image = tray_icon().unwrap();
assert_eq!((image.width(), image.height()), (36, 36));
assert_eq!(alpha_at(&image, 0, 0), 0);
assert_eq!(alpha_at(&image, 18, 18), 255);
```

- [ ] **Step 3: Run the RED tray test**

```bash
cargo test --manifest-path src-tauri/Cargo.toml -p backup-mic --lib tray::tests::template_icon_preserves_the_backup_mic_face_mask
```

Expected: failure because `tray-template.png` and `tray_icon` do not exist.

- [ ] **Step 4: Load the embedded icon instead of the bundle icon**

Decode `TRAY_ICON_BYTES` before building the tray, pass the image with `.icon(icon)`, and retain `.icon_as_template(true)`. Remove the `default_window_icon()` fallback. Keep tooltip, left-click behavior, positioner, toggle, and setup error propagation unchanged.

- [ ] **Step 5: Run tray and source checks**

```bash
cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check
cargo test --manifest-path src-tauri/Cargo.toml -p backup-mic --lib tray::tests
cargo clippy --manifest-path src-tauri/Cargo.toml -p backup-mic --all-targets -- -D warnings
git diff --check
```

- [ ] **Step 6: Commit and push the status-icon checkpoint**

Explicitly stage the PNG and `tray.rs`, commit `feat(tray): use adaptive Backup Mic icon`, push, fetch, and prove `0 0` parity.

---

### Task 5: Finish disposable recorder acceptance and full regression

**Files:**
- Modify: `README.md`
- Modify: `scripts/check.sh`
- Modify: `scripts/install-local.sh`
- Modify: `scripts/package-local.sh`
- Create: `scripts/accept-rule-fixtures.sh`
- Create: `src-tauri/tests/rule_volume_acceptance.rs`

**Interfaces:**
- Consumes: two isolated 64 MiB FAT32 disk images, production device registry/rule matcher/audio pipeline, existing package and rollback installer.
- Produces: repeatable two-recorder acceptance proof and stricter package/install verification.

- [ ] **Step 1: Re-run the missing-fixture guard and source suite**

```bash
env -u BACKUP_MIC_ZOOM_RULE_FIXTURE -u BACKUP_MIC_SONY_RULE_FIXTURE \
  cargo test --manifest-path src-tauri/Cargo.toml -p backup-mic \
  --test rule_volume_acceptance -- --ignored --exact \
  backs_up_two_isolated_fat32_rule_volumes_through_the_production_pipeline
```

Expected: fail with `BACKUP_MIC_RULE_FIXTURES_REQUIRED: run scripts/accept-rule-fixtures.sh`.

- [ ] **Step 2: Run full automated checks**

```bash
./scripts/check.sh
```

Expected: Rust workspace tests, clippy, frontend tests, typecheck, build, and the missing-fixture guard all pass.

- [ ] **Step 3: Run destructive-fixture safety preflight**

Run `./scripts/accept-rule-fixtures.sh`. If exact physical DJI mounts are connected, require the script to refuse before creating or attaching fixtures and do not eject or mutate those recorders. Once the user-controlled physical mounts are absent, rerun and require two disposable FAT32 sources, two verified distinct WAV hashes, two AAC-LC M4A artifacts, separate batch barriers, and safe image detachment.

- [ ] **Step 4: Run existing deletion and verifier acceptance**

```bash
./scripts/accept-deletion-fixture.sh
cargo test --manifest-path src-tauri/Cargo.toml -p backup-mic --test verify_backup_script
```

Require Foundation Trash-only behavior, complete-session barriers, and no physical recorder mutation.

- [ ] **Step 5: Commit and push the release-acceptance checkpoint**

Explicitly stage only the six Task 5 files, commit `test(release): verify rule-based recorder backups`, push, fetch, and prove `0 0` parity.

---

### Task 6: Release, reinstall, and verify the installed app

**Files:**
- Headatever-managed version files and annotated tag only.
- Build outputs under `src-tauri/target/release/bundle/` are verification artifacts and are not committed.

**Interfaces:**
- Consumes: clean source checkpoints, `headatever`, `scripts/package-local.sh`, and `scripts/install-local.sh`.
- Produces: one verified `Backup Mic.app`, one verified DMG, a release tag, and an atomically replaced local installation.

- [ ] **Step 1: Prove source and Git readiness**

```bash
./scripts/check.sh
git status --short --branch
git fetch origin
git rev-list --left-right --count HEAD...origin/main
```

Require only expected generated/build artifacts, no unstaged source, and `0 0` before release.

- [ ] **Step 2: Invoke the headatever skill and create the release**

Read the `headatever` skill. Run `headatever init 0 --dry-run`, inspect the exact proposed version files, then run `headatever init 0 --push`. Do not edit `VERSION`, package metadata, commits, or tags by hand. Fetch and verify the release commit, annotated tag, and `0 0` branch parity.

- [ ] **Step 3: Build and verify one app and one DMG**

```bash
./scripts/package-local.sh "$(tr -d '\r\n' < VERSION)"
```

Require exact bundle/DMG names, identifier `com.channprj.BackupMic`, expected short version, macOS `13.0`, arm64, deep strict ad-hoc signing, DMG verification, and printed SHA-256 values.

- [ ] **Step 4: Atomically reinstall the verified bundle**

```bash
./scripts/install-local.sh \
  "src-tauri/target/release/bundle/macos/Backup Mic.app" \
  "$(tr -d '\r\n' < VERSION)"
```

Require the prior exact app to move to macOS Trash only after staging verification, `/Users/channprj/Applications/Backup Mic.app` to verify again, the legacy exact bundle to be absent, and packaged/installed executable SHA-256 values to match.

- [ ] **Step 5: Launch and verify the installed app**

Launch only the installed bundle, not the build-tree executable:

```bash
open -a "/Users/channprj/Applications/Backup Mic.app"
```

Verify the installed process path is `/Users/channprj/Applications/Backup Mic.app/Contents/MacOS/backup-mic`. Confirm an existing ledger opens as `ready`, shows the current destination, and retains the DJI preset and preferences. With an isolated fresh app-data fixture, verify destination cancel remains blocked, selection transitions to settings review, settings persist, completion transitions to ready, and restart restores the destination.

Inspect the native status item in light and dark appearance: it must show the Backup Mic face, automatically invert, survive an in-process theme switch, and open/toggle the popover at the tray position. Record any unavailable GUI/TCC evidence separately rather than replacing it with source-test claims.

- [ ] **Step 6: Prove final remote parity and clean source tree**

```bash
git fetch origin
git status --short --branch
git rev-list --left-right --count HEAD...origin/main
git rev-parse HEAD
git rev-parse origin/main
```

Require `0 0`, matching SHAs, the release tag on origin, and no uncommitted source files. Report the release version, source/release commits, package hashes, installed hash, onboarding proof, destination proof, status-item proof, and any environment-limited evidence.
