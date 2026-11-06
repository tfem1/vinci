# Test report

Environment of the development sandbox: Ubuntu 24.04 VM, 4 vCPU (Xeon 2.1 GHz),
16 GB RAM, **no GPU**, FFmpeg 6.1.1, GStreamer 1.24, Xvfb + PulseAudio for a virtual
desktop, Rust 1.97, Node 22. No Windows machine and no Wayland compositor were available.

Legend: ✅ verified by running it · 🧪 covered by automated tests with real tools ·
🔧 compiled & unit-tested only · ⏳ verified in CI only (GitHub runners)

## Automated tests (79)

| Suite | Tests | What is exercised |
|---|---|---|
| `vinci-hw` | 7 | FFmpeg version/encoders/filters/devices parsing (incl. the FFmpeg 6.1 ` --` separator), sysfs GPU discovery on a fake tree, pci.ids lookup, GStreamer inventory, live resource sampler |
| `vinci-encoding` | 14 | Argument builders per encoder family and frame source (D3D11 / RGB / YUV), bitrate model, encoder selection & fallbacks, **real probes** (x264 OK, NVENC fails cleanly without a GPU), **real compression to ≤ 1 MB and real lossless trim** |
| `vinci-replay` | 16 | Box parser, incremental demuxer (byte-by-byte = bulk), GOP-aligned eviction, byte budget, epochs, RAM vs disk ring byte-identical output, overwrite detection, **real FFmpeg → buffer → export → ffprobe** (memory + disk, across an encoder restart, without FFmpeg) |
| `vinci-capture` | 8 | Command lines for WGC, DDA, X11/VA-API, GStreamer portal; filtergraph escaping; backend resolution; pactl parsing |
| `vinci-clips` | 5 | SQLite CRUD, search (LIKE escaping), sort, favourites, rename (file renamed), move, delete, folder sync, migrations, real thumbnails |
| `vinci-storage` | 3 | Atomic JSON + corruption recovery, file-name sanitising, unique names, moves, free space, leftover cleanup |
| `vinci-games` | 4 | Database sanity (85 games), Proton/Wine detection, store-library heuristics, foreground priority, custom games |
| `vinci-hotkeys` | 3 | Accelerator parsing/normalisation/validation, backend string formats, conflicts |
| `vinci-update` | 5 | Ed25519 sign/verify, tampering, wrong key, semver channel rules, **end-to-end check + download + SHA-256 against a local HTTP server**, plain-HTTP refusal, AppImage atomic replacement |
| `vinci-engine` | 6 | **Full engine end-to-end** (RAM and disk): capture → buffer → save (video + audio + thumbnail) → library → in-place compression (no duplicate file) → ring file deleted on shutdown; live buffer resize keeps footage; settings forward-compat |
| `vinci-app` | 3 | CLI arguments, media server (token, ids only, Range/206/416) |
| Frontend (vitest) | 5 | Formatting, accelerator capture from keyboard events |

`cargo fmt --check` and `cargo clippy --workspace --all-targets -D warnings` pass for
both `x86_64-unknown-linux-gnu` and `x86_64-pc-windows-gnu`.

## Manual / scripted end-to-end checks

| Check | Result |
|---|---|
| X11 capture of a 1920×1080 animated scene + PulseAudio 440 Hz tone | ✅ clips contain the real screen (frame extracted and inspected) and the tone (−21 dB mean) |
| Global hotkey **Alt+F10** pressed with `xdotool` while another window has focus | ✅ clip saved (CLI and GUI) |
| `scripts/e2e-linux.sh` (hotkey + command save, ffprobe/volumedetect validation) | ✅ `E2E PASSED` |
| Save latency | ✅ 0.55 – 1.5 s for 15–30 s clips (includes waiting for the freshest fragment) |
| Desktop app (Tauri, WebKitGTK) — Dashboard, Library, Capture, Settings, player | ✅ screenshots in `docs/assets/screenshots/` are from the real app |
| In-game overlay on X11 | ✅ shown; on a desktop without compositor VINCI now falls back to a notification (a transparent window would be black) |
| In-app playback | ✅ after replacing `asset://` with the loopback media server (WebKitGTK could not stream video over the custom scheme) |
| `.deb` / `.rpm` built with `tauri build` | ✅ 8.1 MB each, correct dependencies |
| `install.sh` against the built `.deb` from a local mirror | ✅ install, ✅ tampered package rejected, ✅ Ed25519-signed `SHA256SUMS` verified with OpenSSL, ✅ tampered `SHA256SUMS` rejected, ✅ uninstall keeps clips |

## Measured resource usage (sandbox VM, **CPU encoding only**)

| Process | CPU | RAM |
|---|---|---|
| VINCI engine (`vinci-cli`) | 0.1 – 0.4 % | 11 – 15 MB |
| Encoder helper, x264 1080p60 (no GPU available) | ≈ 82 % of one core (≈ 20 % of the machine) | ≈ 300 – 430 MB |
| Replay buffer (2 min, RAM) | — | ≈ buffered bytes (e.g. 2–10 MB for this static test scene) |

With a GPU encoder (NVENC/AMF/QSV/VA-API) the encoding cost moves to the GPU's
dedicated video engine; this could not be measured here (no GPU). Capture throughput on
this 4-vCPU VM: 44–57 unique fps at 1080p60 with audio while also rendering the test scene
in software (60 fps without contention).

## Not verified in the sandbox (and why)

| Item | Status | Why / how it is covered |
|---|---|---|
| Windows capture (WGC, DDA), WASAPI audio, DXGI enumeration, PDH GPU stats, Windows hotkey message loop | 🔧 | No Windows machine. Code compiles and passes clippy for the Windows target; CI runs the test-suite on `windows-latest` and checks the bundled FFmpeg exposes `gfxcapture`/`ddagrab`. **Needs a first manual run on a Windows gaming PC.** |
| Hardware encoders NVENC / AMF / QSV / VA-API | 🔧 | No GPU. Command lines follow the FFmpeg 8.1 sources; every encoder is validated by a real test encode at startup and VINCI falls back automatically if one fails at runtime. |
| Wayland (portal + PipeWire + GStreamer) and GlobalShortcuts portal | 🔧 | No compositor in the sandbox. Pipeline/argument generation is unit-tested. |
| NSIS / MSI / AppImage packages, `install.ps1` | ⏳ | Built and linted in GitHub Actions (GitHub downloads are blocked in the sandbox, and PowerShell is not installed there). |
| In-app updater applying a real update | 🔧 | Check/download/verify tested end-to-end; the final "replace and relaunch" step is per-OS (AppImage replacement unit-tested). |
