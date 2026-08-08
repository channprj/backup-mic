# DJI Mic Mini 2S Tauri/Rust Automatic Backup Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:executing-plans` sequentially in the main thread. The active workspace instructions prohibit subagent execution. Use test-driven development and create only verified outcome-based commits.

**Goal:** Ship a macOS 13+ menu-bar app that automatically backs up stable WAV recordings from paired DJI Mic Mini 2S transmitters, proves each backup by byte count and SHA-256, shows clear progress and history, and offers only explicit two-phase verified source deletion.

**Architecture:** `backup-core` is a Tauri-independent Rust library that owns every filesystem and deletion decision. The Tauri adapter owns Disk Arbitration, lifecycle, native plugins, blocking task scheduling, redacted DTOs, and nine narrow commands. React/Vite/shadcn owns presentation only and treats Rust snapshots as canonical.

**Pinned baseline:** Rust 1.97.1; Tauri Rust 2.11.5; Tauri CLI 2.11.4; Tauri JS API 2.11.1; React 19.2.8; Vite 8.2.1; TypeScript 7.0.2; Tailwind CSS 4.3.3; shadcn CLI 4.16.2; Vitest 4.1.10; `rusqlite` 0.40.2 with bundled SQLite; pnpm 10.33.4; Node 26.7.0.

## Global Constraints

- Approved design: `docs/superpowers/specs/2026-08-09-dji-mic-mini-tauri-rust-backup-design.md`.
- Destination: `/Users/channprj/Documents/DJI-Mic-Mini-2S`; retain 10 GiB after each run.
- Current transmitter recordings are production data. They may be inventoried, read, copied, and independently hashed, but never renamed, modified, ejected, or deleted during implementation or acceptance.
- Volume label, mount path, BSD disk number, and filename prefix are never device authority.
- Automatic deletion, recursive delete, frontend paths, frontend filesystem/shell/network access, remote content, telemetry, and analytics are forbidden.
- Direct Disk Arbitration C API from Rust is the initial platform adapter. A Swift bridge requires a separate design amendment backed by two reproducible failures.
- Every new behavior follows red-green-refactor. Every checkpoint passes relevant tests and `git diff --check`, then is committed, pushed, and proven at `0 0` upstream parity.

## File Layout

```text
src/                              React application and shadcn sources
src/features/backup/              Typed IPC, snapshot hook, popover states
contracts/fixtures/               Rust/TypeScript DTO contract fixtures
src-tauri/src/                    Tauri adapter, commands, tray, services
src-tauri/src/platform/macos/     Isolated Disk Arbitration FFI
src-tauri/crates/backup-core/     Safety domain, SQLite, copy, deletion
scripts/                          Quality, inventory, and package checks
docs/superpowers/                 Approved design and execution plan
```

## Checkpoint 1: Reproducible Workspace and Contracts

**Files:** root Node/Vite/Tailwind/shadcn files; `src-tauri/Cargo.toml`, `build.rs`, `tauri.conf.json`, `capabilities/main.json`; core crate manifest; initial React/Rust smoke tests; `state.rs`, `events.rs`, `error.rs`, `dto.rs`, `contracts/fixtures`.

- [ ] Initialize Git on `main`, create `.gitignore` for build output, `.agent/`, and `.superpowers/`, and create a private `channprj/backup-mic` remote because no repository currently exists.
- [ ] Pin the versions in the baseline and commit both lockfiles. Configure `com.channprj.DJIMicBackup`, macOS 13, a hidden 380px frameless window, strict bundled-content CSP, and a capability containing only `core:default`.
- [ ] Write failing Rust and frontend smoke tests, then implement the minimum bootable shell.
- [ ] Define `Transmitter`, `BackupPhase`, `DeletionPhase`, per-device state, activity, and monotonic progress. Each copied byte contributes copy plus destination-verification work; each reused destination contributes verification work; file completion occurs after ledger commit.
- [ ] Define redacted `AppSnapshotDto`, public errors, opaque pairing candidates, and deletion proposal summaries. Zod objects are strict. Fixtures must be accepted by Rust and TypeScript and must reject paths, UUIDs, hashes, recording names, and ledger IDs.

Verify:

```bash
pnpm typecheck
pnpm test
pnpm build
cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check
cargo test --manifest-path src-tauri/Cargo.toml --workspace
git diff --check
```

Commit: `chore: scaffold tauri backup workspace`

## Checkpoint 2: Trusted Recording Model and Ledger

**Files:** `backup-core/src/{device,recording,scanner,filesystem,clock,ledger,recovery}.rs`, initial migration, property/integration tests.

- [ ] Test and implement paired device matching by UUID plus USB, external, removable, writable, accepted media name, and capacity within the greater of one percent or 256 MiB. Never commit production UUIDs.
- [ ] Test and implement anchored `TXdd_MICdigits_yyyyMMdd_HHmmss[_suffix].wav` parsing, encoded-date destination grouping, modification-date fallback, and transmitter-prefix mismatch reporting.
- [ ] Scan visible directories with `symlink_metadata`, skip hidden paths/symlinks/non-regular files, canonicalize every candidate, require it to stay under the mounted root, and require identical path/size/mtime observations two seconds apart through an injected clock.
- [ ] Add property tests for Unicode components, traversal, hidden segments, and symlink escape.
- [ ] Add bundled SQLite schema for paired devices, settings, backup runs, recordings, deletion runs/items, and activity. Use foreign keys, transactions, WAL, busy timeout, relative paths, 50-entry pruning, interrupted-run recovery, and corruption quarantine that disables deletion.

Verify:

```bash
cargo test --manifest-path src-tauri/Cargo.toml -p backup-core
cargo clippy --manifest-path src-tauri/Cargo.toml -p backup-core --all-targets -- -D warnings
git diff --check
```

Commit: `feat(core): add trusted recording model and ledger`

## Checkpoint 3: Verified Backup and Deletion Safety

**Files:** `backup-core/src/{destination,hash,copy,backup,deletion}.rs`; backup, failure, recovery, deletion, and operation-sequence tests.

- [ ] Preflight `available >= copy bytes + 10 GiB` with checked arithmetic before creating partial files.
- [ ] Plan `YYYY/YYYY-MM-DD/TX01|TX02/original.wav`; reuse only equal content; preserve different same-name content with a deterministic SHA-256 suffix; never overwrite.
- [ ] Copy through an exclusive UUID-named hidden partial with 1 MiB buffers, source hash, flush, file sync, source metadata reread, independent destination hash, size/hash equality, same-directory rename, directory sync, and transactional ledger commit.
- [ ] Reconcile interruption before/after rename conservatively and delete only current-run app-owned partials. Missing/changed existing backups are never silently trusted.
- [ ] Prepare a private five-minute deletion proposal only after a complete live scan and first destination verification. Confirm only by opaque ID; consume it; repeat identity/generation/candidate/metadata/source hash/destination hash preflight for the entire vector before the first non-recursive unlink.
- [ ] Invalidate proposals on unmount, remount, new scan, backup start, destination change, competing deletion, and expiry. Stop on the first unlink error and record exact removed/remaining aggregates.
- [ ] Property-test that arbitrary operation sequences cannot reach unlink without current authority.

Verify:

```bash
cargo test --manifest-path src-tauri/Cargo.toml -p backup-core
cargo clippy --manifest-path src-tauri/Cargo.toml -p backup-core --all-targets -- -D warnings
git diff --check
```

Commit: `feat(core): add verified backup and deletion safety`

## Checkpoint 4: macOS Device and Tauri Lifecycle

**Files:** `src-tauri/src/platform/macos`, `app_state.rs`, `orchestrator.rs`, `pairing.rs`, `commands.rs`, tray/window/native-service adapters, inventory script.

- [ ] Bind the minimal Disk Arbitration and Core Foundation C APIs in one unsafe module. Document create/copy versus borrowed ownership and release every owned reference.
- [ ] Run callbacks on a dedicated CFRunLoop thread and send owned events. Treat an appearance without UUID plus mount URL as pending; use description changes for mount completion; coalesce by UUID/mount generation; cancel and invalidate on disappearance.
- [ ] Use opaque mount-generation pairing candidates. Store TX01/TX02 assignments transactionally and reject duplicates or stale candidates.
- [ ] Run core filesystem work in blocking tasks outside state locks. Allow one backup/deletion operation while preserving independent transmitter outcomes.
- [ ] Register exactly `get_app_snapshot`, `backup_now`, `choose_destination`, `pair_devices`, `prepare_deletion`, `confirm_deletion`, `set_autostart`, `open_destination`, and `quit_app`.
- [ ] Initialize dialog, opener, notification, autostart, and positioner in Rust. React gets no plugin permissions. Set accessory activation policy, toggle/position the hidden tray popover, hide on focus loss unless setup/confirmation is active, and keep the process alive while hidden.
- [ ] Emit revisioned safe snapshots; events accelerate presentation and a later `get_app_snapshot` repairs event loss.
- [ ] Add a read-only inventory script that reports aggregate properties/counts without UUIDs, names, hashes, or destructive commands.

Verify:

```bash
cargo test --manifest-path src-tauri/Cargo.toml --workspace
cargo clippy --manifest-path src-tauri/Cargo.toml --workspace --all-targets -- -D warnings
pnpm tauri build --debug --no-bundle
zsh -n scripts/hardware-inventory.sh
git diff --check
```

Commit: `feat(macos): connect devices and tauri lifecycle`

## Checkpoint 5: Approved Popover UI

**Files:** `src/features/backup`, `src/components/ui`, app CSS, frontend tests.

- [ ] Implement a typed command client and snapshot hook that validates all input, fetches on mount/focus/visibility, listens for changes, keeps the greatest revision, and unregisters listeners.
- [ ] Render the accepted 380px dark native-adjacent surface up to 640px: identity/connection, current summary, active overall and TX progress, newest eight activity entries, contextual action, and utilities. Only activity scrolls.
- [ ] During backup show `백업 중`, keep-connected/originals-unchanged text, percent, verified files, copied bytes, stage, accessible progress bar, and TX01/TX02 pills.
- [ ] Render exact waiting/checking/complete/nothing-new/partial/error/partial-deletion states. Partial states never use success treatment or deletion authority.
- [ ] Implement destination/pairing/notification/autostart setup. The deletion dialog shows only safe proposal aggregates and sends only the proposal ID; no optimistic success.
- [ ] Cover keyboard navigation, focus trap, visible focus, text alternatives, sufficient contrast, and reduced motion.

Verify:

```bash
pnpm test
pnpm typecheck
pnpm build
pnpm tauri build --debug --no-bundle
git diff --check
```

Commit: `feat(ui): add backup status popover`

## Checkpoint 6: Package and Verify the Full Local Chain

**Files:** failure/privacy tests, `scripts/check.sh`, `scripts/package-local.sh`, README, icons, bundle config, long-task audit.

- [ ] Add deterministic failure injection for open/read/write/flush/sync/rename/hash/metadata/capacity/unlink/cancellation/ledger failures. Every failure proves source preservation and honest state.
- [ ] Scan DTOs, activity, notifications, and release logs for forbidden private values.
- [ ] Document setup, automatic backup, progress, SHA-256, deduplication, deletion safety, recovery, capacity, privacy, build/install, and uninstall.
- [ ] Generate icons, build app and DMG, verify plist/signature, install the exact app locally, and verify tray, notifications, login start, focus behavior, sleep/wake, and reconnect.
- [ ] Take a fresh read-only production inventory. Run automatic backup without approving deletion. Independently match every source and destination size/hash. Reconnect and prove no duplicate.
- [ ] Exercise real FAT32 copy/hash/unlink only in a disposable disk-image fixture injected into core tests. Never pair it in the installed app and never delete production recordings.
- [ ] Complete `.agent/audit.md`, run the full gate, commit/push, and prove local/tracking/live-remote parity.

Verify:

```bash
zsh scripts/check.sh
zsh scripts/package-local.sh
git status --short --branch
git rev-list --left-right --count HEAD...@{u}
```

Commit: `docs: package and verify dji mic backup`

## Completion Gate

Implementation is complete only when the full automated gate is green, the installed artifact is the runtime-tested artifact, every production source present at acceptance has an independently equal destination hash, unchanged reconnect creates no duplicate, production deletion has never been invoked, the disposable FAT32 deletion fixture passes, all unavailable runtime proofs are explicitly recorded, and Git local/tracking/live-remote SHA evidence is equal.
