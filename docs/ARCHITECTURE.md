# VINCI — Technical architecture

## 1. Feasibility summary

| Requirement | How VINCI does it | Status |
|---|---|---|
| Instant replay 30 s – 5 min (up to 10 min) | Encoder streams **fragmented MP4**; VINCI keeps a GOP-aligned ring of fragments (RAM or a fixed-size disk file) | Implemented, tested end-to-end |
| Save without re-encoding | Clip = init segment + selected fragments → `ffmpeg -c copy +faststart` remux → atomic rename | Implemented, tested |
| Windows capture | Windows.Graphics.Capture (`gfxcapture`, FFmpeg ≥ 8.1) and DXGI Desktop Duplication (`ddagrab`), frames stay in D3D11 textures | Implemented; **compile-checked only** (no Windows machine in the dev sandbox) |
| Windows audio | Native WASAPI loopback + microphone mixer in Rust, PCM piped to the encoder | Implemented; compile-checked only |
| Linux X11 | `x11grab` (XShm) + PulseAudio/PipeWire-Pulse monitor | Implemented, tested (Xvfb + PulseAudio) |
| Linux Wayland | xdg-desktop-portal ScreenCast → PipeWire fd → GStreamer `pipewiresrc` → encoder → copy-remux | Implemented; unit-tested (no compositor in sandbox) |
| Hardware encoders | NVENC, AMF, Quick Sync, VA-API, validated by a **real test encode** at startup (cached) | Implemented; probe logic tested (no GPU in sandbox → correctly reported unavailable) |
| Codecs / profiles | H.264, HEVC, AV1 × Quality / Balanced / Compact; quality-targeted rate control with peak cap | Implemented, unit-tested |
| Compression | Re-encode to a profile or to a target size (e.g. Discord 10 MB) in place | Implemented, tested |
| Library | SQLite metadata, folder is the source of truth, one physical file per clip | Implemented, tested |
| Hotkeys | `global-hotkey` (RegisterHotKey / XGrabKey) on its own thread; XDG GlobalShortcuts portal on Wayland | X11 tested with xdotool; Windows/Wayland compile-checked |
| Game detection | Process scan + foreground window + database (≈ 85 games) + Steam/Epic/Riot/GOG library heuristics, Proton aware | Implemented, unit-tested |
| Overlay | Transparent click-through always-on-top window, created on demand; desktop notification fallback when no compositor / Wayland | Implemented, tested on X11 |
| Updates | Ed25519-signed manifest + SHA-256 verified downloads, stable/beta channels | Implemented, tested against a local HTTP server |
| Installers | `install.sh` (deb/rpm/AppImage), `install.ps1` (NSIS per-user / MSI), CI-built packages | deb/rpm built and installed locally; AppImage/NSIS/MSI built in CI |

**Non-goals (by design):** no DLL injection / game hooks (anti-cheat safety), no mandatory
account or cloud sync, no telemetry.

## 2. Process model

```
┌──────────────────────────── VINCI process (Rust) ────────────────────────────┐
│ Tauri shell (src-tauri)  ──commands/events──►  vinci-engine::Engine          │
│   React UI (webview,          ▲                   │                          │
│   destroyed when hidden)      │                   ├─ capture supervisor ─────┼──► helper process(es)
│   tray · overlay window       │                   │   watchdog, restart,     │    FFmpeg (WGC/DDA/X11)
│                               │                   │   encoder fallback       │    or GStreamer → FFmpeg (Wayland)
│ vinci-cli (headless) ─────────┘                   │        ▲ fMP4 on stdout  │
│                                                   ├─ replay buffer (RAM/disk ring)
│                                                   ├─ save worker → export (copy remux)
│                                                   ├─ job worker (compress / trim)
│                                                   ├─ library (SQLite) · thumbnails
│                                                   ├─ hotkey service (own thread)
│                                                   ├─ game detector (4–10 s polling)
│                                                   └─ updater (signed manifest)
└───────────────────────────────────────────────────────────────────────────────┘
```

Why an out-of-process media engine? FFmpeg/GStreamer already implement the native capture
APIs and every hardware encoder with zero-copy GPU paths. Running them as helpers keeps
the VINCI process tiny (measured ≈ 12–15 MB RSS, < 1 % CPU for the engine itself), isolates
driver crashes (the supervisor restarts the pipeline and keeps the buffered footage), and
lets users on Linux benefit from their distribution's FFmpeg/VA-API stack.

## 3. Crates

| Crate | Role |
|---|---|
| `vinci-hw` | GPUs (sysfs / DXGI), FFmpeg & GStreamer discovery + capabilities, system info, resource sampler (NVML, amdgpu sysfs, Windows PDH) |
| `vinci-encoding` | Encoder catalog, profiles & bitrate model, FFmpeg argument builders (GPU frame paths), real encoder probing & selection, compression/trim |
| `vinci-capture` | Backend resolution, monitor enumeration, pipeline command lines, process session, WASAPI pump, Wayland portal |
| `vinci-replay` | ISO-BMFF parser, incremental fMP4 demuxer, GOP ring buffer (RAM / disk ring), snapshots, atomic export |
| `vinci-clips` | SQLite library: CRUD, search/filter/sort, folder sync, thumbnails |
| `vinci-storage` | Per-OS paths, atomic JSON, disk space, unique names, cross-volume moves, leftover cleanup |
| `vinci-games` | Game detection |
| `vinci-hotkeys` | Accelerator parsing/validation, global hotkey service, Wayland portal backend |
| `vinci-update` | Signed manifests, verified downloads, apply per install type, release signing tool |
| `vinci-engine` | Orchestration, settings, events, status, logging |
| `vinci-cli` | Headless front-end (`detect`, `record`, `clips`, `compress`, …) |
| `src-tauri` | Desktop shell: windows, tray, overlay, single-instance CLI actions, autostart |

## 4. The capture pipelines

All backends emit the same thing: **fragmented MP4** on stdout, fragments cut at every
keyframe *and* at least every 500 ms (`-movflags frag_keyframe+empty_moov+default_base_moof
-frag_duration 500000`). A save request waits for the next fragment (≤ 500 ms) so the clip
includes the moment the hotkey was pressed.

| Backend | Command graph | GPU path |
|---|---|---|
| WGC | `ffmpeg -init_hw_device d3d11va … -filter_complex "gfxcapture=hmonitor=…:max_framerate=60[:width=…:height=…:resize_mode=scale_aspect],fps=60[v]"` + WASAPI PCM on stdin | D3D11 texture → NVENC/AMF directly; QSV via `scale_d3d11=format=nv12,hwmap=derive_device=qsv`; scaling done inside the capture shader |
| DDA | `ddagrab=output_idx=N:framerate=60` | same; scaling with `scale_d3d11` (D3D11 video processor) |
| X11 | `x11grab` + `-f pulse -i @DEFAULT_MONITOR@` | VA-API: `format=bgr0,hwupload,scale_vaapi=format=nv12` (colour conversion on the GPU); NVENC accepts RGB directly |
| Portal | `gst-launch pipewiresrc fd=3 path=<node> ! va/nv/x264 enc ! matroskamux` → `ffmpeg -c copy` → fMP4 | VA-API through `vapostproc` (VAMemory) |
| Test | `lavfi testsrc2 + sine` in real time | — |

Exact FFmpeg option names were verified against the FFmpeg 8.1.2 sources
(`vsrc_gfxcapture.c`, `vf_scale_d3d11.c`, `vsrc_ddagrab.c`, `nvenc*.c`, `amfenc*.c`).

## 5. Replay buffer

* `Fmp4Demuxer` splits the byte stream into the init segment and `moof+mdat` fragments
  (zero-copy `BytesMut::split_to`), parsing `tfdt`/`trun` to get decode time, duration and
  whether the fragment starts with a sync sample (trun first-sample-flags → sample flags →
  tfhd defaults → trex defaults).
* `ReplayBuffer` keeps fragments of one or more **epochs** (an epoch = one encoder session;
  a restart after a crash starts a new epoch and the old footage stays saveable).
  Eviction removes **whole GOPs** from the front, so the buffer always starts on a keyframe,
  while keeping at least `buffer_secs` (+ one keyframe interval).
* **RAM mode**: `Bytes` per fragment, snapshots are reference-counted clones (no copy).
* **Disk mode**: one preallocated circular file (`ring_dir/replay-<pid>.ring`). Records carry
  a `{magic, len, seq}` header checked before *and* after reading, so a reader racing the
  writer detects an overwrite instead of producing a corrupt clip. The file is deleted on
  exit and leftovers are cleaned at startup.
* **Auto** picks RAM when the estimated peak size fits the configured budget (512 MB default).

## 6. Saving a clip

1. Snapshot the last N seconds (starting at the keyframe at or before `now − N`).
2. Write each epoch as `.<name>.partK.mp4` in the destination folder, `fsync`.
3. `ffmpeg -c copy -avoid_negative_ts make_zero -movflags +faststart` (concat demuxer for
   multiple epochs) into `.<name>.vinci-tmp`, `fsync`, **atomic rename** to the final name.
4. If remuxing fails, the raw fragmented MP4 (already a valid file) is kept — never lose a clip.
5. ffprobe metadata → SQLite row → 480 px thumbnail. Measured: 0.55–1.5 s for 15–30 s clips
   (including the wait for the freshest fragment).

## 7. Encoder selection

At first start (and when the GPU/driver/FFmpeg fingerprint changes) every encoder compiled
into FFmpeg is validated with a 15-frame test encode; results are cached. Selection order:
explicit user choice → hardware encoders along the codec chain AV1 → HEVC → H.264
(NVENC > AMF > QSV > VA-API) → CPU fallback **x264** (real-time x265/SVT-AV1 would cost the
game several cores). If an encoder fails 3 times in a row at runtime it is blacklisted for
the session and the next one is used; the UI shows why.

## 8. Resource discipline

* The engine polls nothing while idle except the game detector (4 s; 10 s in resource saver).
* Resource stats are only sampled while the UI is visible, from one shared poller.
* Closing the window destroys the webview (≈ 60–150 MB freed); the engine keeps running.
* Raw-frame queue capped at 16 frames: if the encoder falls behind, frames are dropped
  instead of buffering gigabytes of raw video.
* Capture helper priority: above normal on Windows (avoids stutter when the game pegs the CPU).

## 9. In-app playback

WebKitGTK cannot stream `<video>` from Tauri's custom `asset://` scheme (it reports a
`FormatError`, although GStreamer decodes the same file over `file://` and `http://`).
The player therefore streams from a tiny loopback HTTP server inside VINCI
(`src-tauri/src/media_server.rs`): bound to `127.0.0.1` on a random port, every URL carries
a 128-bit random token, clips are addressed **by library id** (no paths → no traversal),
and `Range` requests give instant seeking. Thumbnails still use `asset://`.

## 10. Security & privacy

* No network access except the optional, signed update check (once a day, can be disabled).
* Updates: Ed25519 signature over the manifest + SHA-256 per installer; refuses to install
  if the build has no release key, if the signature or hash mismatches, or over plain HTTP.
* The replay buffer only exists in RAM or in a temporary ring file deleted on exit.
* No injection into game processes.
