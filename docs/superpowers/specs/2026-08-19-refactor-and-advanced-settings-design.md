# Refactor and Advanced Settings — Design

## Problem Statement

`backup-mic` passes its full gate and its safety behaviour is well covered, but the code has
outgrown its files and the settings surface has stopped growing with the app.

Two concrete problems:

1. **Vestigial structure.** The app began as a DJI-specific two-transmitter tool and became a
   general rule engine. The old transmitter-pairing machinery was never removed, so the runtime
   still maintains a second, parallel copy of every source's state that nothing reads. Several
   files also carry duplicated helpers and command boilerplate, and four files hold more than
   1,500 lines each, which makes them hard to read and hard to change safely.
2. **Frozen policy.** Two safety-relevant numbers — the destination free-space reserve and the
   mounted-device rescan interval — are compile-time constants even though both are already
   threaded through the code as ordinary parameters. Users cannot adjust either one, and the
   settings window exposes only three booleans.

## Desired Outcome

The same verified behaviour, with dead structure gone, large files split along their real seams,
and the two frozen policy numbers exposed as first-class persisted preferences with UI controls.

Every checkpoint keeps `./scripts/check.sh` green.

## Scope

### Phase A — Remove the vestigial transmitter-pairing subsystem

Evidence that the whole chain is unreachable:

| Item | Location | Why it is dead |
|---|---|---|
| `AppState::pair_devices` | `app_state.rs:977` | No production caller. `pair_devices` is an explicitly forbidden command name (`tests/commands.rs:40`). |
| `PairingManager::observe` | `pairing.rs:40` | No caller anywhere, so `candidates` is always empty. |
| `PairingManager::summaries` | `pairing.rs:67` | Always returns `[]` because of the above. |
| `snapshot.pairing_candidates` | `dto.rs:218` | `#[serde(skip)]`; only writer is `sync_pairing_snapshot`. |
| `snapshot.transmitters` | `dto.rs:202` | `#[serde(skip)]`; maintained in parallel with `snapshot.sources` from 11 call sites. |
| `AppState::awaiting_deletion_transmitter` | `app_state.rs:1192` | No caller; sole reader of `snapshot.transmitters`. |
| `AppState::mounted_roots` | `app_state.rs:481` | No caller. |
| `RuntimeState.{pairing, paired, mounted}` | `app_state.rs:53-57` | Write-only once the above are gone. |

Two existing tests already assert that `transmitters` and `pairing_candidates` never reach the
webview (`tests/ipc_contract.rs:48`, `tests/rule_commands.rs:52`). Deleting the fields keeps
those assertions true, so they stay as regression cover.

`backup_core::device::{match_volume, DeviceMatch}` and `Ledger::paired_devices` stay. They back
the documented connected-hardware acceptance test in `platform/macos/mod.rs`.

### Phase B — De-duplicate shared helpers

- `fn local_now()` exists three times (`app_state.rs:1510`, `commands.rs:708`, `lib.rs:192`),
  `fn now_string()` twice (`orchestrator.rs:2238`, `recovery_tool.rs:246`), and the expression
  `UtcOffset::current_local_offset().unwrap_or(UtcOffset::UTC)` appears eight times. Collapse
  them into one `src-tauri/src/clock.rs`.
- Each of the 19 Tauri commands repeats the same two-step shape: call one `AppState` method,
  wrap its error with `reported_core_error(state, command, stage, error, None)`, then return
  `snapshot_with_activity()` wrapped the same way. Introduce one small helper so a command reads
  as its intent instead of twelve lines of `map_err`.

### Phase C — Split oversized files along existing seams

Pure moves with `pub use` re-exports, so no caller changes and no test changes:

| File | Lines | Split into |
|---|---|---|
| `backup-core/src/ledger.rs` | 3161 | `ledger/` — schema, settings, rules, sources, records, runs, retirement, activity |
| `backup-core/src/deletion.rs` | 2331 | `deletion/` — proposal, preflight, retirement |
| `src-tauri/src/orchestrator.rs` | 2335 | `orchestrator/` — run pipeline, device monitor, trash |
| `src-tauri/src/app_state.rs` | 1991 | after Phase A, split remaining state concerns |

Frontend: `BackupPopover.tsx` (659), `SettingsApp.tsx` (564) and `RuleEditor.tsx` (538) each mix
data flow, layout and dialog state. Extract the cohesive fragments into their own components.

### Phase D — Advanced settings

Both new preferences are numbers the code already accepts as parameters, so no safety logic
changes — only where the number comes from.

| Setting | Today | Becomes | Default |
|---|---|---|---|
| Destination free-space reserve | `DEFAULT_CAPACITY_RESERVE_BYTES = 10 GiB` (`destination.rs:24`) | persisted preference, chosen from a fixed set | 10 GiB |
| Mounted-device rescan interval | `RESCAN_INTERVAL = 15 s` (`orchestrator.rs:72`) | persisted preference, chosen from a fixed set | 15 s |

Storage needs **no migration**. Preferences live in the key–value `settings` table
(`ledger.rs:625`, `ledger.rs:694`) and an absent row already falls back to the Rust default.

The IPC boundary stays narrow and typed. `check.sh` forbids arbitrary settings IPC
(`set_setting(key`, `setting_key`), so the new commands take a named, range-checked value —
never a caller-supplied key. Out-of-range values are refused in Rust, not clamped, so the UI can
never silently disagree with what was stored.

### Non-Goals

Unchanged from `.agent/goal.md`: no audio playback, transcription, tagging, editing, cloud sync,
analytics, automatic trash emptying, automatic eject, App Store distribution or notarization.

Additionally out of scope here:

- **Configurable M4A bitrate.** The profile id `aac_lc_128k_v1` (`batch.rs:5`) is durable ledger
  evidence and the gate asserts the exact `afconvert` argument list. Making the bitrate variable
  would change the meaning of every stored cohort barrier. Not worth it.
- **Log retention/pruning.** Would add a delete path to a codebase whose central safety rule is
  that it never deletes without reverification.
- **CSS restructuring.** `index.css` is 1342 lines of working hand-written component styles with
  no visual regression cover. Splitting it risks appearance changes for no functional gain.

## Verification

`./scripts/check.sh` must pass at every checkpoint. Baseline for comparison, recorded before any
change: 232 aggregate Rust test passes, 11 frontend files / 75 tests, clean fmt, clippy
`-D warnings`, typecheck and production build.

New behaviour gets tests first, following the repo's red-green rule:

- Reserve and interval defaults when no row exists.
- Round-trip through the ledger.
- Refusal of out-of-range values.
- Capacity preflight honouring a non-default reserve.
- Rescan scheduler honouring a non-default interval, including a live change.
- Zod contract and settings UI cover for the new controls.
