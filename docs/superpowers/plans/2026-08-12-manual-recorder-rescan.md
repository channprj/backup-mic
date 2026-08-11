# Manual Recorder Rescan and Detection Recovery Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Stop mounted recorders from remaining in a false detecting state when automatic backup is disabled, and provide a refresh button plus Command-R that rescans and immediately backs up newly found recordings.

**Architecture:** Keep recorder discovery and backup behavior in the existing Rust lifecycle and `backup_now` command. The backend will expose an idle mounted state whenever no automatic job is scheduled. The React popover will funnel the header button, keyboard shortcut, and footer action through one guarded `backupNow` handler, preserving the current scan, copy, verification, conversion, and Trash policy.

**Tech Stack:** Tauri 2, Rust, React, TypeScript, Vitest, Testing Library, pnpm, macOS app bundling scripts.

## Global Constraints

- Preserve the existing DJI recorder rule, source identity, archive naming, hashing, conversion, and optional verified-session Trash behavior.
- Do not add a new Tauri command, DTO, or capability; manual refresh reuses `backup_now`.
- Do not trigger a real manual backup during installed-app verification because the current profile has automatic Trash enabled.
- Use red-green TDD for each behavior change and commit/push only green checkpoints.
- Preserve unrelated user-owned changes and finish with local, tracking, and live remote parity.

---

## Task 1: Settle matched mounts when no automatic backup is scheduled

**Files:**

- Modify: `src-tauri/src/app_state.rs`

- [ ] Add the focused unit test below in the existing `app_state` test module before changing production code:

```rust
#[test]
fn matched_mount_phase_tracks_whether_backup_is_really_scheduled() {
    assert_eq!(mounted_source_phase(false), BackupPhase::Idle);
    assert_eq!(mounted_source_phase(true), BackupPhase::Detecting);
}
```

- [ ] Run the exact test and confirm it fails because `mounted_source_phase` does not exist:

```bash
cargo test --manifest-path src-tauri/Cargo.toml --lib app_state::tests::matched_mount_phase_tracks_whether_backup_is_really_scheduled -- --exact
```

- [ ] Add this private helper near `backup_requirements_met`:

```rust
fn mounted_source_phase(should_schedule: bool) -> BackupPhase {
    if should_schedule {
        BackupPhase::Detecting
    } else {
        BackupPhase::Idle
    }
}
```

- [ ] Restructure `handle_mounted` to compute `should_schedule` once after inserting the matched source. Use `mounted_source_phase(should_schedule)` for the dynamic source, legacy transmitter, and overall snapshot. Keep the matched source in runtime state and keep its activity log even when automatic backup is off, but return its ID only when a job should actually be scheduled.

```rust
let should_schedule = settings.automatic_backup
    && backup_requirements_met(&settings, runtime.destination.as_ref());
let source_phase = mounted_source_phase(should_schedule);
// ...update source, transmitter, and overall phases with source_phase...
if should_schedule { Some(source_id) } else { None }
```

- [ ] Run focused and regression checks:

```bash
cargo test --manifest-path src-tauri/Cargo.toml --lib app_state::tests::matched_mount_phase_tracks_whether_backup_is_really_scheduled -- --exact
cargo test --manifest-path src-tauri/Cargo.toml --lib app_state::tests
cargo test --manifest-path src-tauri/Cargo.toml --test rescan_scheduler
cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check
git diff --check
```

- [ ] Commit, push, and prove parity:

```bash
git add src-tauri/src/app_state.rs
git commit -m "fix(backup): settle manual recorder detection"
git push
git status --short --branch
git rev-list --left-right --count HEAD...@{upstream}
git fetch origin
git rev-list --left-right --count HEAD...origin/main
```

Expected: both rev-list commands report `0 0`.

---

## Task 2: Add one guarded manual refresh path to the backup popover

**Files:**

- Modify: `src/features/backup/BackupPopover.tsx`
- Modify: `src/features/backup/__tests__/BackupPopover.test.tsx`
- Modify: `src/index.css`

- [ ] Add a manual-idle fixture and failing tests that prove:

  - a mounted idle recorder renders `녹음기 연결됨` and `다시 확인 및 백업을 눌러 새 녹음을 확인하세요`;
  - the header button invokes `actions.backupNow` once;
  - Command-R on a non-editable target prevents browser reload and invokes once;
  - a pending backup ignores duplicate button, footer, and shortcut requests;
  - an active backup prevents Command-R reload but does not invoke another backup;
  - an editable input target is left untouched;
  - incomplete setup prevents reload but does not invoke backup.

- [ ] Run the focused suite and confirm the new expectations fail:

```bash
pnpm exec vitest run src/features/backup/__tests__/BackupPopover.test.tsx
```

- [ ] Import `useCallback`, make the existing `run` helper stable, and add a shared handler used by the header button, footer button, and shortcut:

```tsx
const refreshAndBackup = useCallback(async () => {
  try {
    await run("backup", actions.backupNow);
  } catch {
    // run already exposes the failure through the inline error state.
  }
}, [actions.backupNow, run]);
```

- [ ] Add an editable-target helper and a window keydown effect. Command-R on a non-editable target must always call `preventDefault`; it invokes `refreshAndBackup` only when setup is ready, no backup phase is active, and `pendingRef.current` is clear.

```tsx
function isEditableTarget(target: EventTarget | null) {
  return target instanceof HTMLElement
    && (target.matches("input, textarea, select") || target.isContentEditable);
}
```

- [ ] Add an icon-only header button beside the connection summary with:

  - class `header-refresh`;
  - `aria-label="녹음기 다시 확인 및 백업"`;
  - `title="녹음기 다시 확인 및 백업 (⌘R)"`;
  - disabled state while active or pending;
  - the existing spinner while the backup action is pending.

- [ ] Pass mounted recorder count into the settled status component and render the mounted-idle title and explanation. Keep the current disconnected copy for zero mounted sources.

- [ ] Keep the header button square without introducing a new visual system:

```css
.header-actions .header-refresh {
  width: 32px;
  padding-inline: 0;
}
```

- [ ] Run focused tests, type checking, formatting, and diff checks:

```bash
pnpm exec vitest run src/features/backup/__tests__/BackupPopover.test.tsx
pnpm typecheck
pnpm exec prettier --check src/features/backup/BackupPopover.tsx src/features/backup/__tests__/BackupPopover.test.tsx src/index.css
git diff --check
```

- [ ] Commit, push, and prove parity:

```bash
git add src/features/backup/BackupPopover.tsx src/features/backup/__tests__/BackupPopover.test.tsx src/index.css
git commit -m "feat(ui): add manual recorder refresh"
git push
git status --short --branch
git rev-list --left-right --count HEAD...@{upstream}
git fetch origin
git rev-list --left-right --count HEAD...origin/main
```

Expected: both rev-list commands report `0 0`.

---

## Task 3: Prove, package, reinstall, and launch the completed app

**Files:**

- Verify: entire repository
- Produce: `src-tauri/target/release/bundle/macos/Backup Mic.app`
- Produce: `src-tauri/target/release/bundle/dmg/*.dmg`
- Install: `/Applications/Backup Mic.app`
- Copy: `~/Downloads/<built dmg>`

- [ ] Run the complete source validation matrix:

```bash
pnpm test
pnpm build
cargo test --manifest-path src-tauri/Cargo.toml --workspace
cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check
git diff --check
```

- [ ] Build the signed local package with the repository script:

```bash
./scripts/package-local.sh "$(tr -d '\r\n' < VERSION)"
```

- [ ] Reinstall the exact built app and launch it:

```bash
./scripts/install-local.sh 'src-tauri/target/release/bundle/macos/Backup Mic.app' "$(tr -d '\r\n' < VERSION)"
open -a '/Applications/Backup Mic.app'
```

- [ ] Verify the running process, bundle version, `arm64` architecture, code signature, and equality of built/installed executable SHA-256 hashes.

- [ ] Verify the two mounted DJI volumes still contain their non-Trash WAV files and confirm backup destination and automatic-backup/Trash settings are unchanged. Do not press the refresh action against these real recordings.

- [ ] Capture installed-app evidence that the mounted recorder is idle and the manual refresh control is present. If macOS WebView accessibility or screen capture is unavailable, report that limitation separately from source, package, install, and process proof.

- [ ] Move the newly built DMG to the resolved `~/Downloads` directory. If an exact-name destination exists, move it to a proven recoverable Trash location first; stop instead of overwriting when recovery cannot be proven. Verify the final DMG with `hdiutil verify` and record its SHA-256.

- [ ] If packaging or verification generates tracked changes, commit and push them as one meaningful green checkpoint. Otherwise do not create an empty commit.

- [ ] Finish with a clean-tree and remote-parity audit:

```bash
git status --short --branch
git log --oneline --decorate -8
git rev-list --left-right --count HEAD...@{upstream}
git fetch origin
git rev-list --left-right --count HEAD...origin/main
```

Expected: clean status and both rev-list commands report `0 0`.
