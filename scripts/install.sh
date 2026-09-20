#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CARGO_BIN="${CARGO_HOME:-$HOME/.cargo}/bin"
TARGET_LINK="$CARGO_BIN/tutor"
DEBUG_BIN="$ROOT/target/debug/tutor"
RELEASE_BIN="$ROOT/target/release/tutor"

mkdir -p "$CARGO_BIN"

usage() {
    cat <<EOS
Usage: scripts/install.sh [release|dev|status]

  release (default) Build optimized release binary and install to $CARGO_BIN/tutor
  dev               Build debug binary and symlink $CARGO_BIN/tutor -> target/debug/tutor
  status            Show currently active tutor binary location and version
EOS
}

mode="${1:-release}"

case "$mode" in
    release)
        echo "==> Building and installing release binary to $TARGET_LINK..."
        cargo install --path "$ROOT" --locked
        echo "==> Installed:"
        ls -l "$TARGET_LINK"
        "$TARGET_LINK" --version
        ;;

    dev)
        echo "==> Building debug binary..."
        (cd "$ROOT" && cargo build)
        echo "==> Symlinking $TARGET_LINK -> $DEBUG_BIN..."
        ln -snfv "$DEBUG_BIN" "$TARGET_LINK"
        echo "==> Dev link established:"
        ls -l "$TARGET_LINK"
        "$TARGET_LINK" --version
        echo
        echo "Note: Any code changes recompiled with 'cargo build' will immediately take effect."
        ;;

    status)
        echo "==> Which tutor is on PATH:"
        which tutor 2>/dev/null || echo "(not found in PATH)"
        echo
        echo "==> Target link in cargo bin:"
        if [ -e "$TARGET_LINK" ] || [ -L "$TARGET_LINK" ]; then
            ls -l "$TARGET_LINK"
            "$TARGET_LINK" --version 2>/dev/null || true
        else
            echo "(not installed in $CARGO_BIN)"
        fi
        ;;

    -h|--help|help)
        usage
        exit 0
        ;;

    *)
        echo "Error: unknown mode '$mode'" >&2
        usage >&2
        exit 1
        ;;
esac
