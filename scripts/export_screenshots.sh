#!/usr/bin/env bash
# Renders the GUI screenshots (sample data) for the README under Xvfb.
#   scripts/export_screenshots.sh [extra dest dir]
# Output: docs/screenshots/*.png, copied to /workspace/windowkeeper-screens/
# (and to the optional first argument).
set -euo pipefail

cd "$(dirname "$0")/.."
DISPLAY_NUM="${SCREENSHOT_DISPLAY:-:99}"
OUT_DIR="docs/screenshots"
COPY_DIRS=("/workspace/windowkeeper-screens")
[ $# -ge 1 ] && COPY_DIRS+=("$1")

XVFB_PID=""
cleanup() { [ -n "$XVFB_PID" ] && kill "$XVFB_PID" 2>/dev/null || true; }
trap cleanup EXIT

# Reuse a running X server on that display, otherwise start Xvfb.
if ! xdpyinfo -display "$DISPLAY_NUM" >/dev/null 2>&1; then
  Xvfb "$DISPLAY_NUM" -screen 0 1920x1200x24 +extension GLX +render -nolisten tcp &
  XVFB_PID=$!
  for _ in $(seq 1 50); do
    xdpyinfo -display "$DISPLAY_NUM" >/dev/null 2>&1 && break
    sleep 0.2
  done
fi

export DISPLAY="$DISPLAY_NUM"
export LIBGL_ALWAYS_SOFTWARE=1
export WINIT_UNIX_BACKEND=x11
unset WAYLAND_DISPLAY

cargo run --example export_screenshots --features screenshots --release -- "$OUT_DIR"

for dir in "${COPY_DIRS[@]}"; do
  mkdir -p "$dir"
  cp -f "$OUT_DIR"/*.png "$dir"/
  echo "kopiert nach $dir"
done
