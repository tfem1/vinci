#!/usr/bin/env bash
# Start a headless test desktop for VINCI end-to-end tests (Linux/CI):
#   Xvfb :99 (1920x1080) + PulseAudio null sink playing a 440 Hz tone
#   + an animated test pattern window.
# Usage: scripts/virtual-desktop.sh start|stop
set -euo pipefail
DISPLAY_NUM="${VINCI_TEST_DISPLAY:-:99}"
LOGDIR="${TMPDIR:-/tmp}/vinci-virtual-desktop"
mkdir -p "$LOGDIR"

case "${1:-start}" in
  start)
    if ! xdpyinfo -display "$DISPLAY_NUM" >/dev/null 2>&1; then
      Xvfb "$DISPLAY_NUM" -screen 0 1920x1080x24 -nolisten tcp >"$LOGDIR/xvfb.log" 2>&1 &
      for _ in $(seq 1 50); do xdpyinfo -display "$DISPLAY_NUM" >/dev/null 2>&1 && break; sleep 0.1; done
    fi
    export DISPLAY="$DISPLAY_NUM"
    pulseaudio --check 2>/dev/null || pulseaudio -D --exit-idle-time=-1 --log-target=file:"$LOGDIR/pulse.log"
    for _ in $(seq 1 50); do pactl info >/dev/null 2>&1 && break; sleep 0.1; done
    pactl list short sinks | grep -q vinci_test || pactl load-module module-null-sink sink_name=vinci_test >/dev/null
    pactl set-default-sink vinci_test
    pgrep -f "sine=frequency=440" >/dev/null || \
      (ffmpeg -hide_banner -loglevel error -re -f lavfi -i "sine=frequency=440:sample_rate=48000" -f pulse -device vinci_test tone >"$LOGDIR/tone.log" 2>&1 &)
    pgrep -f "VINCI-test-scene" >/dev/null || \
      (ffplay -hide_banner -loglevel error -f lavfi -i "testsrc2=size=1600x900:rate=60" -window_title VINCI-test-scene -an >"$LOGDIR/scene.log" 2>&1 &)
    sleep 1
    echo "virtual desktop ready on DISPLAY=$DISPLAY_NUM (audio: vinci_test.monitor)"
    ;;
  stop)
    pkill -f "VINCI-test-scene" || true
    pkill -f "sine=frequency=440" || true
    pulseaudio -k 2>/dev/null || true
    pkill -f "Xvfb $DISPLAY_NUM" || true
    ;;
esac
