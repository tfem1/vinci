#!/usr/bin/env bash
# End-to-end test on a virtual X11 desktop (used by CI, runnable locally):
#   real x11grab capture + PulseAudio + global hotkey (Alt+F10 via xdotool)
#   + command save, then validation of the clips with ffprobe.
set -euo pipefail
cd "$(dirname "$0")/.."

cargo build -q -p vinci-cli
BIN="$PWD/target/debug/vinci-cli"
scripts/virtual-desktop.sh start
export DISPLAY="${VINCI_TEST_DISPLAY:-:99}"
WORK="$(mktemp -d)"
export VINCI_HOME="$WORK/home"
cleanup() { [ -n "${REC_PID:-}" ] && kill "$REC_PID" 2>/dev/null; exec 3>&- 2>/dev/null || true; scripts/virtual-desktop.sh stop >/dev/null 2>&1 || true; rm -rf "$WORK"; }
trap cleanup EXIT

mkfifo "$WORK/cmd"
"$BIN" --log info record --backend x11 --buffer 30 --hotkeys <"$WORK/cmd" >"$WORK/rec.log" 2>"$WORK/rec.err" &
REC_PID=$!
exec 3>"$WORK/cmd"   # keep the fifo open

fail() { echo "E2E FAILED: $*"; echo "--- stdout"; cat "$WORK/rec.log"; echo "--- stderr (tail)"; tail -30 "$WORK/rec.err"; exit 1; }

sleep 14
grep -q "save_default.*registered" "$WORK/rec.log" || fail "Alt+F10 was not registered"
xdotool key --clearmodifiers alt+F10
sleep 4
echo "save 6" >&3
sleep 4
echo "quit" >&3
wait "$REC_PID" || true
REC_PID=""

mapfile -t clips < <(find "$VINCI_HOME/Clips" -name '*.mp4' | sort)
[ "${#clips[@]}" -eq 2 ] || fail "expected 2 clips, found ${#clips[@]}"
grep -q "hotkey: SaveClip" "$WORK/rec.log" || fail "hotkey press not received"
for c in "${clips[@]}"; do
  streams="$(ffprobe -v error -show_entries stream=codec_type -of csv=p=0 "$c" | sort | tr '\n' ' ')"
  [ "$streams" = "audio video " ] || fail "$c: unexpected streams '$streams'"
  dur="$(ffprobe -v error -show_entries format=duration -of csv=p=0 "$c")"
  mean="$(ffmpeg -hide_banner -nostats -i "$c" -map 0:a -af volumedetect -f null - 2>&1 | sed -n 's/.*mean_volume: \(-\?[0-9.]*\) dB.*/\1/p')"
  awk -v m="$mean" 'BEGIN { exit !(m > -45) }' || fail "$c: audio is silent (mean $mean dB)"
  echo "OK  $(basename "$c")  ${dur}s  audio ${mean} dB"
done
echo "E2E PASSED"
