# Refactor and Advanced Settings — Plan and Outcome

Spec: `docs/superpowers/specs/2026-08-19-refactor-and-advanced-settings-design.md`

Every checkpoint below was verified with `./scripts/check.sh` and pushed with `HEAD...@{u}`
parity `0 0` before the next one started.

## Baseline

`./scripts/check.sh` green before any change: 261 aggregate Rust test passes, 83 frontend tests,
clean `cargo fmt`, `clippy -D warnings`, TypeScript, and production Vite build.

(The first baseline was measured against a tree eleven commits behind `origin/main`. After
rebasing onto `e453959` the numbers above are the ones every checkpoint is compared against.)

## Checkpoints

| Commit | Outcome |
|---|---|
| `e78f77c` | Spec for the work below |
| `4d118a7` + `eae33a1` | Removed the vestigial transmitter-pairing subsystem |
| `19c19a8` | One `clock` module instead of three copies of `local_now` |
| `f0946e8` | `Reported` replaces the per-command `map_err` boilerplate |
| `22a6895` | Free-space reserve and rescan interval became settings |
| `4bb05b6` | `ledger.rs` split into per-table modules |
| `f5ba6b1` | `orchestrator.rs` split by backup stage |
| `fdf3bd5` | `deletion.rs` split into authority, proof, and execution |
| `4361779` | `app_state.rs` grouped by what each part owns |
| `9ee8035` | Popover and settings behaviour separated from layout |

`4d118a7` was pushed before its callers were removed, so that commit alone does not compile.
`eae33a1` completes it. Both are left in history rather than rewritten, because the branch was
already published.

## What the file sizes became

| Before | After |
|---|---|
| `ledger.rs` 3158 | `ledger/` — largest module 866 |
| `deletion.rs` 2331 | `deletion/` — largest module 842 |
| `orchestrator.rs` 2258 | `orchestrator/` — largest module 519 |
| `app_state.rs` 1825 | `app_state/` — largest module 634 |
| `BackupPopover.tsx` 670 | 133, plus three focused files |
| `SettingsApp.tsx` 662 | 413, plus two focused files |

## How the moves were verified

Each split compared every non-import, non-brace line before and after and required an exact
match, so a move could not quietly become an edit:

| Split | Logic lines, both sides |
|---|---|
| ledger | 2741 |
| orchestrator | 1893 |
| deletion | 1972 |
| app_state | 1425 |

The frontend split is verified by its tests instead — 407 lines of DOM-level popover tests and the
settings tests exercise the components directly.

## New behaviour, and the tests that cover it

- Free-space reserve (1–512 GiB, default 10) and rescan interval (5–3600 s, default 15) persist in
  the settings table, which needs no migration because an absent row already falls back to the
  Rust default.
- Out-of-range values are refused in Rust rather than clamped, at both ends of both ranges.
- A stored value the settings window does not offer as a preset is displayed as itself.
- The rescan interval applies without a restart, and `set_interval` rebases pending deadlines in
  both directions.
- The capacity preflight reads the configured reserve: a run refuses with `InsufficientCapacity`
  and preserves every source, and the same run succeeds once the reserve fits.
- `setting.saved` records numeric limits. The audit log's approved-field list needed `value`;
  without it the write failed closed and logged `audit_event_invalid`, which a test caught.

## Fixes found along the way

- `AppState::set_preference` had no production caller and was a second, divergent copy of the
  snapshot mapping in `apply_persisted_preferences` — it never set `setting_applies_next_run`.
- The settings header carried a hand-typed `v0.260811.4` while the app was `0.260815.0`. Vite now
  injects the version from the manifest, with a test asserting the rendered string.
- `deletion_guard` used `include_str!` on one file, so anything added beside it would have escaped
  the forbidden-token scan. It now walks the policy directory and refuses to run if the file count
  drops.

## Deliberately not done

- **Configurable M4A bitrate.** `aac_lc_128k_v1` is durable ledger evidence and the gate asserts
  the exact `afconvert` argument list; a variable bitrate would change the meaning of every stored
  cohort barrier.
- **Log retention.** Would add a delete path to a codebase whose central rule is that it never
  deletes without reverification.
- **`index.css`.** 1368 lines of working hand-written component styles with no visual regression
  cover; splitting it risks appearance changes for no functional gain.
