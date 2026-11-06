# Third-party software

VINCI's own code is MIT-licensed (see `LICENSE`).

* **FFmpeg** — the Windows installer and the AppImage ship an unmodified FFmpeg 8.1
  build (BtbN/FFmpeg-Builds, GPL v3 variant) as separate executables (`ffmpeg`,
  `ffprobe`) that VINCI launches as helper processes. FFmpeg is © the FFmpeg developers,
  licensed under the GPL v3 for these builds; its license is installed next to the
  binaries (`FFMPEG-LICENSE.txt`) and its source code is available at
  https://ffmpeg.org/download.html and https://github.com/BtbN/FFmpeg-Builds.
  Linux deb/rpm packages use the distribution's FFmpeg instead.
* **GStreamer** (Linux, Wayland capture) — used from the system, LGPL.
* **Tauri, Rust crates and npm packages** — licensed under MIT/Apache-2.0 or compatible
  licenses; see `Cargo.lock` / `package-lock.json` for the complete list.
* **Inter** typeface — SIL Open Font License 1.1 (via `@fontsource-variable/inter`).
* **Lucide** icons — ISC License.

## KDE screencast protocol

`crates/vinci-capture/protocols/zkde-screencast-unstable-v1.xml` is copied unchanged from
KDE's plasma-wayland-protocols (SPDX-FileCopyrightText: 2020-2021 Aleix Pol Gonzalez
<aleixpol@kde.org>, SPDX-License-Identifier: LGPL-2.1-or-later). VINCI generates Rust
bindings from it at build time to request screen streams from KWin.
