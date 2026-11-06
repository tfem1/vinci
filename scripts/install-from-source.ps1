<#
 Build VINCI from this checkout and install it into %LOCALAPPDATA%\vinci (+ user PATH).
 No release needed.

   git clone https://github.com/tfem1/vinci; cd vinci; powershell -ExecutionPolicy Bypass -File .\scripts\install-from-source.ps1

 Needs: Rust (https://rustup.rs, MSVC toolchain), Node.js 20+, Visual Studio Build Tools
 with "Desktop development with C++", WebView2 (preinstalled on Windows 11).
 Downloads the FFmpeg 8.1 build that VINCI bundles (BtbN/FFmpeg-Builds, SHA-256 checked).
#>
$ErrorActionPreference = 'Stop'
Set-Location (Join-Path $PSScriptRoot '..')

function Write-Ok([string]$m) { Write-Host "  [ok] $m" -ForegroundColor Green }
function Stop-Build([string]$m) { Write-Host ""; Write-Host "  error: $m" -ForegroundColor Red; Write-Host ""; throw "VINCI was not built." }
function Invoke-Step([string]$exe, [string[]]$arguments) {
    & $exe @arguments
    if ($LASTEXITCODE -ne 0) { Stop-Build "'$exe $($arguments -join ' ')' failed (exit code $LASTEXITCODE)." }
}

# --- prerequisites ----------------------------------------------------------
foreach ($t in @(@('cargo', 'Rust is not installed: https://rustup.rs (choose the MSVC toolchain), then open a new terminal.'),
                 @('node', 'Node.js 20+ is required: https://nodejs.org'),
                 @('npm', 'npm is required (it comes with Node.js): https://nodejs.org'))) {
    if (-not (Get-Command $t[0] -ErrorAction SilentlyContinue)) { Stop-Build $t[1] }
}
$nodeMajor = [int]((& node --version).TrimStart('v').Split('.')[0])
if ($nodeMajor -lt 20) { Stop-Build "Node.js $(& node --version) is too old; VINCI needs Node.js 20 or newer." }
$vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
if (Test-Path $vswhere) {
    $vc = & $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
    if (-not $vc) { Stop-Build "Visual Studio Build Tools with 'Desktop development with C++' are required: https://visualstudio.microsoft.com/visual-cpp-build-tools/" }
} else {
    Stop-Build "Visual Studio Build Tools with 'Desktop development with C++' are required: https://visualstudio.microsoft.com/visual-cpp-build-tools/"
}

# ARM64 PCs build the x64 version (it runs under Windows 11 emulation).
$hostArch = if ($env:PROCESSOR_ARCHITEW6432) { $env:PROCESSOR_ARCHITEW6432 } else { $env:PROCESSOR_ARCHITECTURE }
$targetArgs = @()
$outDir = 'target\release'
if ($hostArch -eq 'ARM64') {
    Write-Host "  ARM64 PC: building the x64 version (runs under emulation)." -ForegroundColor Yellow
    Invoke-Step 'rustup' @('target', 'add', 'x86_64-pc-windows-msvc')
    $targetArgs = @('--target', 'x86_64-pc-windows-msvc')
    $outDir = 'target\x86_64-pc-windows-msvc\release'
} elseif ($hostArch -ne 'AMD64') { Stop-Build "Unsupported architecture '$hostArch'." }
Write-Ok "Prerequisites: $(& cargo --version), node $(& node --version)"

# --- FFmpeg sidecar (bundled next to vinci.exe) ------------------------------
$sidecar = 'src-tauri\binaries\ffmpeg-x86_64-pc-windows-msvc.exe'
if (-not (Test-Path $sidecar)) { & (Join-Path $PSScriptRoot 'fetch-ffmpeg.ps1') }
foreach ($t in 'ffmpeg', 'ffprobe') {
    if (-not (Test-Path "src-tauri\binaries\$t-x86_64-pc-windows-msvc.exe")) { Stop-Build "FFmpeg download failed ($t missing in src-tauri\binaries)." }
}

# --- build ---------------------------------------------------------------------
# `tauri build` (not plain `cargo build`) is required: it embeds the web UI in the binary.
Invoke-Step 'npm' @('ci', '--no-audit', '--no-fund')
Invoke-Step 'npx' (@('tauri', 'build', '--no-bundle') + $targetArgs)
Invoke-Step 'cargo' (@('build', '--release', '--locked', '-p', 'vinci-cli') + $targetArgs)
Write-Ok "Built $outDir\vinci.exe and vinci-cli.exe"

# --- install -------------------------------------------------------------------
$installDir = Join-Path $env:LOCALAPPDATA 'vinci'
Get-Process -Name 'vinci', 'vinci-cli' -ErrorAction SilentlyContinue | Where-Object { $_.Path -and $_.Path -like "$installDir\*" } | Stop-Process -Force
New-Item -ItemType Directory -Force -Path $installDir | Out-Null
Copy-Item "$outDir\vinci.exe", "$outDir\vinci-cli.exe" -Destination $installDir -Force
foreach ($t in 'ffmpeg', 'ffprobe') { Copy-Item "src-tauri\binaries\$t-x86_64-pc-windows-msvc.exe" (Join-Path $installDir "$t.exe") -Force }
Copy-Item LICENSE, THIRD_PARTY_NOTICES.md -Destination $installDir -Force
if (Test-Path 'src-tauri\binaries\FFMPEG-LICENSE.txt') { Copy-Item 'src-tauri\binaries\FFMPEG-LICENSE.txt' $installDir -Force }
Write-Ok "Installed to $installDir"

# --- user PATH (idempotent; keeps %VARIABLES% unexpanded) -------------------
$envKey = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey('Environment', $true)
$raw = [string]$envKey.GetValue('Path', '', [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)
$entries = @($raw -split ';' | Where-Object { $_.Trim() })
if ($entries | Where-Object { [Environment]::ExpandEnvironmentVariables($_).TrimEnd('\') -ieq $installDir.TrimEnd('\') }) {
    Write-Ok "$installDir is already in your PATH"
} else {
    $envKey.SetValue('Path', (($entries + $installDir) -join ';'), [Microsoft.Win32.RegistryValueKind]::ExpandString)
    [Environment]::SetEnvironmentVariable('VINCI_INSTALL_REFRESH', '1', 'User')
    [Environment]::SetEnvironmentVariable('VINCI_INSTALL_REFRESH', $null, 'User')
    Write-Ok "Added $installDir to your user PATH"
}
$envKey.Close()

try {
    $s = (New-Object -ComObject WScript.Shell).CreateShortcut((Join-Path ([Environment]::GetFolderPath('Programs')) 'VINCI.lnk'))
    $s.TargetPath = Join-Path $installDir 'vinci.exe'; $s.WorkingDirectory = $installDir; $s.Save()
    Write-Ok "Start menu shortcut"
} catch { Write-Host "  [!] Start menu shortcut not created: $($_.Exception.Message)" -ForegroundColor Yellow }

Write-Host ""
Write-Host "  Done. Open a NEW terminal and run: vinci   (or start VINCI from the Start menu)" -ForegroundColor Gray
Write-Host ""
