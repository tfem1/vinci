# Package the built Windows binaries into the portable zip installed by install.ps1.
#   powershell -File scripts\package-archive.ps1 [-Target windows-x86_64] [-Out dist]
# Expects `npx tauri build --no-bundle`, `cargo build --release -p vinci-cli` and
# scripts\fetch-ffmpeg.ps1 to have run. Produces <Out>\vinci-<Target>.zip containing
# vinci-<Target>\{vinci.exe, vinci-cli.exe, ffmpeg.exe, ffprobe.exe, licenses}.
param(
    [string]$Target = 'windows-x86_64',
    [string]$Out = 'dist',
    [string]$Bin = 'target\release'
)
$ErrorActionPreference = 'Stop'
Set-Location (Join-Path $PSScriptRoot '..')

$name = "vinci-$Target"
$work = Join-Path ([IO.Path]::GetTempPath()) ("vinci-pkg-" + [Guid]::NewGuid().ToString('N'))
$stage = Join-Path $work $name
New-Item -ItemType Directory -Force -Path $stage, $Out | Out-Null
try {
    foreach ($b in 'vinci.exe', 'vinci-cli.exe') {
        if (-not (Test-Path "$Bin\$b")) { throw "missing $Bin\$b - build first" }
        Copy-Item "$Bin\$b" $stage
    }
    foreach ($t in 'ffmpeg', 'ffprobe') {
        $src = "src-tauri\binaries\$t-x86_64-pc-windows-msvc.exe"
        if (-not (Test-Path $src)) { throw "missing $src - run scripts\fetch-ffmpeg.ps1" }
        Copy-Item $src (Join-Path $stage "$t.exe")
    }
    Copy-Item LICENSE, THIRD_PARTY_NOTICES.md $stage
    if (Test-Path 'src-tauri\binaries\FFMPEG-LICENSE.txt') { Copy-Item 'src-tauri\binaries\FFMPEG-LICENSE.txt' $stage }
    $zip = Join-Path $Out "$name.zip"
    if (Test-Path $zip) { Remove-Item $zip }
    Compress-Archive -Path $stage -DestinationPath $zip
    Write-Host $zip
} finally {
    Remove-Item -Recurse -Force $work -ErrorAction SilentlyContinue
}
