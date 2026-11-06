#!/usr/bin/env bash
# Runs VINCI's Wayland capture pipeline against a real PipeWire video stream, without
# a desktop: a private D-Bus session + PipeWire + WirePlumber, a green test-pattern
# node published with pipewiresink, then the `real_pipewire_stream_to_fmp4` test
# (pipewiresrc → x264 → Matroska → FFmpeg remux → fragmented MP4 → replay demuxer).
#
#   scripts/test-wayland-pipeline.sh
#
# Needs: pipewire, wireplumber, dbus-run-session, GStreamer (pipewire, base, good,
# bad, ugly plugins), FFmpeg. The xdg-desktop-portal handshake itself is not covered.
set -euo pipefail
cd "$(dirname "$0")/.."

if [ -z "${VINCI_PW_INNER:-}" ]; then
  for c in pipewire wireplumber dbus-run-session pw-dump gst-launch-1.0 python3; do
    command -v "$c" >/dev/null || { echo "missing: $c" >&2; exit 1; }
  done
  exec env VINCI_PW_INNER=1 dbus-run-session -- "$0" "$@"
fi

RUN="$(mktemp -d)"; chmod 700 "$RUN"
export XDG_RUNTIME_DIR="$RUN"
cleanup() { local p; for p in $(jobs -p); do kill "$p" 2>/dev/null || true; done; wait 2>/dev/null || true; rm -rf "$RUN"; }
trap cleanup EXIT

pipewire >"$RUN/pipewire.log" 2>&1 &
sleep 1
wireplumber >"$RUN/wireplumber.log" 2>&1 &
sleep 1
gst-launch-1.0 -q videotestsrc is-live=true pattern=solid-color foreground-color=0xff00ff00 \
  ! video/x-raw,format=BGRx,width=1280,height=720,framerate=30/1 \
  ! pipewiresink mode=provide stream-properties="properties,media.class=Video/Source,node.name=vinci-test-source" \
  >"$RUN/source.log" 2>&1 &

node=""
for _ in $(seq 1 50); do
  node="$(pw-dump 2>/dev/null | python3 -c '
import json, sys
for o in json.load(sys.stdin):
    p = (o.get("info") or {}).get("props") or {}
    if p.get("node.name") == "vinci-test-source":
        print(o["id"]); break
' || true)"
  [ -n "$node" ] && break
  sleep 0.2
done
[ -n "$node" ] || { echo "the test PipeWire node did not appear" >&2; cat "$RUN/source.log" >&2; exit 1; }
echo "PipeWire test node: $node"

VINCI_TEST_PW_NODE="$node" cargo test -q -p vinci-capture real_pipewire_stream_to_fmp4 -- --nocapture
