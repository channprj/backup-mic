# Recorder Layout, Live Preview, Safe Cancellation, and UI Scale Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add exactly three per-recorder date folder layouts, make path previews match real output, allow active scans/backups to be cancelled without retiring source files, and normalize compact macOS control typography and spacing.

**Architecture:** Extend the existing rule DTO and SQLite record with a closed `DateFolderLayout` enum that defaults existing data and the DJI preset to `year_month`. Keep actual path generation in `backup-core`, while a small frontend formatter mirrors the same public naming contract for immediate representative previews. Expose the existing process-wide cancellation token through one narrow Tauri command, make stable scanning cancellation-aware, and settle cancellation as idle rather than error before any automatic Trash retirement.

**Tech Stack:** Tauri 2, Rust, rusqlite, React, TypeScript, Zod, Vitest, Testing Library, Tailwind/CVA, pnpm, macOS packaging scripts.

## Global constraints

- Supported layout values are only `year_month_day`, `year_month`, and `compact_date`.
- Existing rules, migrations, and DJI Mic Mini 2S preset default to `year_month` (`YYYY/MM/`).
- Preserve the filename contract `YYMMDD-{prefix}{profiled source stem}{suffix}.{extension}`.
- Preserve existing artifact evidence and `source-extras`; do not move old archive files.
- Cancellation must be checked before automatic source retirement and must not create an error snapshot.
- Use red-green TDD and push only meaningful green checkpoints.
- Do not trigger a real backup on the connected DJI volumes during installed-app verification.
- Preserve unrelated user-owned files and finish with local/tracking/live-remote parity.

---

## Task 1: Add the closed date-folder model and destination behavior

**Files:**

- Modify: `src-tauri/crates/backup-core/src/rule.rs`
- Modify: `src-tauri/crates/backup-core/src/destination.rs`
- Modify: `src-tauri/crates/backup-core/src/preset.rs`
- Modify: `src-tauri/crates/backup-core/tests/rule_destinations.rs`
- Modify: `src-tauri/crates/backup-core/tests/rule_validation.rs`
- Modify rule fixtures in other focused tests only as required by the new field

- [ ] Add failing destination tests for the exact paths:

```text
year_month_day -> <archive>/2026/08/12/260812-file.wav
year_month     -> <archive>/2026/08/260812-file.wav
compact_date   -> <archive>/260812/260812-file.wav
```

- [ ] Add a failing test that prefix, suffix and DJI `TX01_` to `T01_` profiling stay identical across all three layouts.
- [ ] Add `DateFolderLayout` with snake-case serde values and a `Default` implementation returning `YearMonth`.
- [ ] Add `date_folder_layout` to `BackupRuleDraft` and `BackupRule`; thread it through validation and compilation.
- [ ] Refactor `recording_rule_destination` to append the selected folder components before the unchanged final filename.
- [ ] Set the DJI preset explicitly to `DateFolderLayout::YearMonth`.
- [ ] Update compile-only test fixtures with the compatibility default; do not mechanically change expected paths outside focused layout assertions.
- [ ] Run focused tests and formatting:

```bash
cargo test --manifest-path src-tauri/Cargo.toml -p backup-core --test rule_destinations
cargo test --manifest-path src-tauri/Cargo.toml -p backup-core --test rule_validation
cargo test --manifest-path src-tauri/Cargo.toml -p backup-core --test rule_scanner
cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check
git diff --check
```

---

## Task 2: Persist and migrate the selected layout

**Files:**

- Create: `src-tauri/crates/backup-core/migrations/0007_rule_date_folder_layout.sql`
- Modify: `src-tauri/crates/backup-core/src/ledger.rs`
- Modify: `src-tauri/crates/backup-core/tests/rule_ledger.rs`
- Modify: `src-tauri/crates/backup-core/tests/dynamic_sources.rs`

- [ ] Add failing ledger tests proving create, update, list, duplicate-style draft construction and DJI restore preserve each selected layout.
- [ ] Add a migration regression test that opens a schema at version 6 and proves its existing rules hydrate as `YearMonth` after migration.
- [ ] Add `date_folder_layout TEXT NOT NULL DEFAULT 'year_month'` with a CHECK limited to the three approved values.
- [ ] Apply migration 7 in `migrate()` and extend `RULE_COLUMNS`, `StoredRuleRow`, hydration, insert, update, archived restore and DJI restore SQL consistently.
- [ ] Reject unknown persisted values as invalid data rather than silently falling back.
- [ ] Run ledger and migration regression tests:

```bash
cargo test --manifest-path src-tauri/Cargo.toml -p backup-core --test rule_ledger
cargo test --manifest-path src-tauri/Cargo.toml -p backup-core --test dynamic_sources
cargo test --manifest-path src-tauri/Cargo.toml -p backup-core --test ledger_recovery
cargo test --manifest-path src-tauri/Cargo.toml -p backup-core --test rule_layout_migration
cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check
git diff --check
```

- [ ] Explicitly stage the core model, migration, destination, ledger and affected tests; commit and push:

```bash
git commit -m "feat(rules): support date folder layouts"
git push
```

- [ ] Prove tracking and live remote both report `0 0`.

---

## Task 3: Extend frontend contracts and implement the live output preview

**Files:**

- Modify: `src/features/backup/contracts.ts`
- Modify: `src/features/backup/RuleEditor.tsx`
- Modify: `src/features/backup/RuleList.tsx`
- Modify: `src/features/backup/SettingsApp.tsx`
- Modify: `src/features/backup/__tests__/contracts.test.ts`
- Modify: `src/features/backup/__tests__/RuleEditor.test.tsx`
- Modify: `src/features/backup/__tests__/RuleList.test.tsx`
- Modify affected snapshot/fixture builders under `src/features/backup/__tests__` and `src/preview.tsx`

- [ ] Extend the Zod schemas with `z.enum(["year_month_day", "year_month", "compact_date"])` and default new drafts to `year_month`.
- [ ] Add failing RuleEditor tests for the exact three select labels and saved values.
- [ ] Add failing real-time tests that change archive, layout, prefix and suffix and observe the output path immediately.
- [ ] Add failing tests for DJI representative input/profile (`TX01_` to `T01_`) and current artifact extension (`m4a`/`wav`).
- [ ] Implement a pure preview formatter near RuleEditor with explicit layout mapping and profile-aware representative filename; pass `snapshot.artifact_format` from SettingsApp.
- [ ] Render two compact rows, `원본 예시` and `백업 결과`, in a wrapping monospace signal-path card.
- [ ] Update RuleList to render the selected label instead of fixed `YYYY/MM` and preserve the field when duplicating a rule.
- [ ] Run the focused frontend suites and type/build checks:

```bash
pnpm exec vitest run src/features/backup/__tests__/contracts.test.ts src/features/backup/__tests__/RuleEditor.test.tsx src/features/backup/__tests__/RuleList.test.tsx src/features/backup/__tests__/SettingsApp.test.tsx
pnpm typecheck
pnpm build
git diff --check
```

- [ ] Explicitly stage the contract, preview UI and tests; commit and push:

```bash
git commit -m "feat(ui): preview recorder output paths"
git push
```

- [ ] Prove tracking and live remote both report `0 0`.

---

## Task 4: Add cancellable stable scanning and a safe backend command

**Files:**

- Modify: `src-tauri/crates/backup-core/src/rule_scanner.rs`
- Modify: `src-tauri/crates/backup-core/tests/rule_scanner.rs`
- Modify: `src-tauri/src/app_state.rs`
- Modify: `src-tauri/src/orchestrator.rs`
- Modify: `src-tauri/src/commands.rs`
- Modify: `src-tauri/src/lib.rs`
- Modify related Rust integration tests where the orchestrator seam already exists

- [ ] Add failing scanner tests for cancellation before the first pass and across the stability wait.
- [ ] Thread `&CancellationToken` into `scan_rule_stable`; check at scan entry, during directory iteration, around sleep and before returning the second result. Keep scheduler one-pass scanning on a non-operation internal helper.
- [ ] Add failing AppState tests for cancellation settlement: mounted sources return to idle, progress/stages/error/retirement reset, message is `operation_cancelled`, and an audit activity is recorded once.
- [ ] Add `settle_cancelled_operation` and make `cancel_active_operation` report whether a live operation token was requested.
- [ ] Add an argument-free `cancel_backup` command, register it in the exact command list/count, and expose it through the Tauri invoke handler.
- [ ] Handle `CoreError::Cancelled` separately in `start_backup`; do not route it through `set_error`.
- [ ] Check the operation token between recording completion and automatic retirement so cancellation cannot move source files to Trash.
- [ ] Add a regression test proving retirement is not called after cancellation at that boundary.
- [ ] Run focused and workspace Rust checks:

```bash
cargo test --manifest-path src-tauri/Cargo.toml -p backup-core --test rule_scanner
cargo test --manifest-path src-tauri/Cargo.toml --lib app_state::tests
cargo test --manifest-path src-tauri/Cargo.toml --lib commands::tests
cargo test --manifest-path src-tauri/Cargo.toml --lib orchestrator::tests
cargo test --manifest-path src-tauri/Cargo.toml --workspace
cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check
git diff --check
```

---

## Task 5: Expose cancellation in the popover

**Files:**

- Modify: `src/features/backup/client.ts`
- Modify: `src/features/backup/BackupPopover.tsx`
- Modify: `src/features/backup/__tests__/client.test.ts`
- Modify: `src/features/backup/__tests__/BackupPopover.test.tsx`
- Modify: `src/preview.tsx`

- [ ] Add a failing client test for the exact argument-free `cancel_backup` invocation.
- [ ] Add `cancelBackup` to `backupClient`, `BackupActions`, every mock and the local preview action set.
- [ ] Add failing UI tests proving an active snapshot shows `백업 취소`, one click calls the command once, the label becomes `취소 중…`, repeated clicks are blocked, and header refresh remains disabled.
- [ ] Reuse the existing `run`/single-flight action guard with a distinct `cancel` pending key; do not optimistically invent an idle snapshot.
- [ ] Keep Command-R reload prevention but do not start backup while active or cancellation is pending.
- [ ] Verify inline errors and eventual backend snapshots still render correctly.
- [ ] Run focused/frontend checks:

```bash
pnpm exec vitest run src/features/backup/__tests__/client.test.ts src/features/backup/__tests__/BackupPopover.test.tsx
pnpm typecheck
pnpm build
git diff --check
```

- [ ] Explicitly stage backend and frontend cancellation files and tests; commit and push:

```bash
git commit -m "feat(backup): support safe cancellation"
git push
```

- [ ] Prove tracking and live remote both report `0 0`.

---

## Task 6: Normalize compact control typography and spacing

**Files:**

- Modify: `src/components/ui/button.tsx`
- Modify: `src/index.css`
- Modify: `src/features/backup/RuleEditor.tsx` only if semantic classes are needed
- Modify frontend tests only for stable semantic/accessibility assertions, not pixel snapshots

- [ ] Audit rendered control classes against the approved scale: 22px title, 13px section, 11px body/button/control, 10px caption, 32px controls, 4/8/12/16 spacing.
- [ ] Change the shared Button base from oversized `text-sm` to the compact product scale and align default/small/icon heights without shrinking hit clarity.
- [ ] Align settings inputs/selects/actions, rule rows and the new preview card to the same baseline, radius and spacing tokens.
- [ ] Preserve current palette, SF font stack, focus-visible treatment, disabled contrast and dark/light behavior.
- [ ] Run the frontend suite and build:

```bash
pnpm test
pnpm build
pnpm exec prettier --check src/components/ui/button.tsx src/features/backup/RuleEditor.tsx src/index.css
git diff --check
```

- [ ] Build/run the local preview mode or test harness and inspect settings and popover at the app viewport; fix overflow, clipped labels and disproportionate controls.
- [ ] Explicitly stage only the visual consistency changes; commit and push:

```bash
git commit -m "fix(ui): normalize control type and spacing"
git push
```

- [ ] Prove tracking and live remote both report `0 0`.

---

## Task 7: Full verification, package, reinstall, launch, and DMG delivery

**Files and artifacts:**

- Verify: entire repository
- Build: `src-tauri/target/release/bundle/macos/Backup Mic.app`
- Build/move: `src-tauri/target/release/bundle/dmg/*.dmg` to resolved `~/Downloads`
- Install: `/Users/channprj/Applications/Backup Mic.app`

- [ ] Run the complete source matrix from a clean intended diff:

```bash
pnpm test
pnpm build
cargo test --manifest-path src-tauri/Cargo.toml --workspace
cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check
git diff --check
```

- [ ] Confirm the app is stopped, record both mounted DJI volumes' non-Trash WAV counts, and record backup destination plus automatic backup/Trash settings before packaging.
- [ ] Build the local package with the repository script and current `VERSION`:

```bash
./scripts/package-local.sh "$(tr -d '\r\n' < VERSION)"
```

- [ ] Verify app architecture, deep/strict ad-hoc signature, bundle version and built executable SHA-256.
- [ ] Reinstall the exact built app with `scripts/install-local.sh`, compare installed executable SHA-256, and launch it.
- [ ] Open the tray UI at most once without invoking refresh/backup; verify the connected idle state, cancel-ready active UI from automated tests, three settings options and the live preview through non-destructive evidence.
- [ ] Confirm source WAV counts and persisted settings are unchanged after runtime inspection.
- [ ] Move the new DMG to resolved `~/Downloads`. If the exact destination exists, first use a recoverable Finder/Foundation Trash operation and verify disappearance; never overwrite an unproven target.
- [ ] Run `hdiutil verify` and SHA-256 on the final Downloads DMG.
- [ ] If packaging creates no tracked changes, do not fabricate an empty commit.
- [ ] Finish with clean tree and remote parity:

```bash
git status --short --branch
git log --oneline --decorate -10
git rev-list --left-right --count HEAD...@{upstream}
git fetch origin
git rev-list --left-right --count HEAD...origin/main
```

Expected: installed app is running, source files/settings are unchanged, the DMG is verified in Downloads, the tree is clean, and both rev-list commands report `0 0`.
