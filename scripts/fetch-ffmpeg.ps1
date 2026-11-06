# Download the FFmpeg build bundled with VINCI on Windows (needed before `npm run tauri dev`
# or `npm run tauri build` on Windows). Verified against the published checksums.
#   powershell -ExecutionPolicy Bypass -File scripts\fetch-ffmpeg.ps1
param([string]$Series = $(if ($env:FFMPEG_SERIES) { $env:FFMPEG_SERIES } else { '8.1' }))
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
Set-Location (Join-Path $PSScriptRoot '..')

$name = "ffmpeg-n$Series-latest-win64-gpl-$Series.zip"
$base = 'https://github.com/BtbN/FFmpeg-Builds/releases/download/latest'
$work = Join-Path ([IO.Path]::GetTempPath()) ("vinci-ffmpeg-" + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $work | Out-Null
try {
    Write-Host "downloading $name"
    Invoke-WebRequest -UseBasicParsing "$base/$name" -OutFile "$work\$name"
    Invoke-WebRequest -UseBasicParsing "$base/checksums.sha256" -OutFile "$work\checksums.sha256"
    $line = Get-Content "$work\checksums.sha256" | Where-Object { $_ -match "\s$([regex]::Escape($name))$" } | Select-Object -First 1
    if (-not $line) { throw "no checksum for $name" }
    $expected = ($line -split '\s+')[0].ToLowerInvariant()
    $actual = (Get-FileHash -Algorithm SHA256 "$work\$name").Hash.ToLowerInvariant()
    if ($expected -ne $actual) { throw "CHECKSUM MISMATCH for $name" }
    Write-Host "sha256 OK ($actual)"
    Expand-Archive "$work\$name" -DestinationPath "$work\x"
    New-Item -ItemType Directory -Force -Path 'src-tauri\binaries' | Out-Null
    foreach ($tool in 'ffmpeg', 'ffprobe') {
        $src = Get-ChildItem -Recurse "$work\x" -Filter "$tool.exe" | Select-Object -First 1
        if (-not $src) { throw "$tool.exe not found" }
        Copy-Item $src.FullName "src-tauri\binaries\$tool-x86_64-pc-windows-msvc.exe" -Force
    }
    $lic = Get-ChildItem -Recurse "$work\x" -Filter 'LICENSE*' | Select-Object -First 1
    if ($lic) { Copy-Item $lic.FullName 'src-tauri\binaries\FFMPEG-LICENSE.txt' -Force }
    Write-Host "sidecars ready in src-tauri\binaries\"
} finally {
    Remove-Item -Recurse -Force $work -ErrorAction SilentlyContinue
}
