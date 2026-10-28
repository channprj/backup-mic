#!/bin/bash
# No arguments build only the frontend. Tauri's beforeBuildCommand and
# scripts/check.sh call `pnpm build` that way, so those calls must not package.
# `pnpm build install [open]` packages, installs, and optionally launches.

# Prints one action per line: frontend, or package + install, plus launch.
# Returns 2 for any argument other than install or open.
build_actions() {
  if [[ $# -eq 0 ]]; then
    printf '%s\n' frontend
    return 0
  fi

  local arg do_launch=0
  for arg in "$@"; do
    case "$arg" in
      install) ;;
      open) do_launch=1 ;;
      *)
        echo "error: unknown argument: $arg" >&2
        echo "Usage: pnpm build | pnpm build install [open]" >&2
        return 2
        ;;
    esac
  done

  printf '%s\n' package
  printf '%s\n' install
  if [[ "$do_launch" -eq 1 ]]; then
    printf '%s\n' launch
  fi
}

run_build() {
  local root actions status line
  local do_frontend=0 do_package=0 do_install=0 do_launch=0
  root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
  cd "$root"

  set +e
  actions="$(build_actions "$@")"
  status=$?
  set -e
  if [[ "$status" -ne 0 ]]; then
    exit "$status"
  fi

  while IFS= read -r line; do
    [[ -n "$line" ]] || continue
    case "$line" in
      frontend) do_frontend=1 ;;
      package) do_package=1 ;;
      install) do_install=1 ;;
      launch) do_launch=1 ;;
      *)
        echo "error: unknown action: $line" >&2
        exit 1
        ;;
    esac
  done <<<"$actions"

  if [[ "$do_frontend" -eq 1 ]]; then
    tsc -b
    vite build
  fi
  if [[ "$do_package" -eq 1 ]]; then
    bash scripts/package-local.sh
  fi
  if [[ "$do_install" -eq 1 ]]; then
    bash scripts/install-local.sh "src-tauri/target/release/bundle/macos/Backup Mic.app"
  fi
  if [[ "$do_launch" -eq 1 ]]; then
    open -a "$HOME/Applications/Backup Mic.app"
  fi
}

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
  set -euo pipefail
  run_build "$@"
fi
