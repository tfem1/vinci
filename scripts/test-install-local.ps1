# Test installers\windows\install.ps1 against a locally packaged zip served from a local
# HTTP mirror (no GitHub): install, user PATH, idempotent re-install, tampered archive,
# missing asset, missing release. Installs into the real %LOCALAPPDATA%\vinci - run it
# on a CI runner or a test machine.
#
#   powershell -ExecutionPolicy Bypass -File scripts\test-install-local.ps1 [-Dist release-dist] [-Target windows-x86_64]
#
# Needs python (for the local HTTP server) and scripts\package-archive.ps1 output.
param(
    [string]$Dist = 'release-dist',
    [string]$Target = 'windows-x86_64',
    [int]$Port = 8765
)
$ErrorActionPreference = 'Stop'
Set-Location (Join-Path $PSScriptRoot '..')

$asset = "vinci-$Target.zip"
if (-not (Test-Path (Join-Path $Dist $asset))) { throw "missing $Dist\$asset - run scripts\package-archive.ps1 first" }

$work = Join-Path ([IO.Path]::GetTempPath()) ('vinci-mirror-' + [Guid]::NewGuid().ToString('N'))
$mirror = Join-Path $work 'mirror'
New-Item -ItemType Directory -Force -Path $mirror, (Join-Path $work 'empty') | Out-Null
Copy-Item (Join-Path $Dist $asset) $mirror
Copy-Item 'installers\windows\install.ps1' $mirror
$lines = foreach ($f in Get-ChildItem $mirror -File) {
    '{0}  {1}' -f (Get-FileHash -Algorithm SHA256 -Path $f.FullName).Hash.ToLowerInvariant(), $f.Name
}
Set-Content -Path (Join-Path $mirror 'SHA256SUMS') -Value $lines -Encoding Ascii

$installDir = Join-Path $env:LOCALAPPDATA 'vinci'
$server = Start-Process -FilePath 'python' -ArgumentList @('-m', 'http.server', "$Port", '--bind', '127.0.0.1', '--directory', $work) -PassThru -WindowStyle Hidden
Start-Sleep -Seconds 3

function Invoke-Installer([string]$sub) {
    # The installer writes errors to stderr; keep them as text instead of stopping here.
    $saved = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    $env:VINCI_BASE_URL = "http://127.0.0.1:$Port/$sub"
    $out = & powershell -NoProfile -ExecutionPolicy Bypass -File (Join-Path $mirror 'install.ps1') 2>&1 | Out-String
    $code = $LASTEXITCODE
    $ErrorActionPreference = $saved
    return [pscustomobject]@{ Code = $code; Out = $out }
}
function Get-PathCount {
    $target = $installDir.TrimEnd('\')
    @([Environment]::GetEnvironmentVariable('Path', 'User') -split ';' | Where-Object { $_ -and ([Environment]::ExpandEnvironmentVariables($_).TrimEnd('\') -ieq $target) }).Count
}
function Pass([string]$m) { Write-Host "PASS  $m" }

$failed = $null
try {
    $r = Invoke-Installer 'mirror'
    if ($r.Code -ne 0) { throw "install failed (exit $($r.Code)):`n$($r.Out)" }
    foreach ($b in 'vinci.exe', 'vinci-cli.exe', 'ffmpeg.exe', 'ffprobe.exe') {
        if (-not (Test-Path (Join-Path $installDir $b))) { throw "$b not installed in $installDir" }
    }
    $ver = & (Join-Path $installDir 'vinci-cli.exe') --version
    if ($LASTEXITCODE -ne 0) { throw 'the installed vinci-cli.exe does not run' }
    if ((Get-PathCount) -ne 1) { throw "user PATH contains $installDir $(Get-PathCount) times" }
    Pass "install into $installDir ($ver), user PATH updated"

    $r = Invoke-Installer 'mirror'
    if ($r.Code -ne 0) { throw "re-install failed (exit $($r.Code)):`n$($r.Out)" }
    if ((Get-PathCount) -ne 1) { throw "re-install duplicated the PATH entry ($(Get-PathCount))" }
    Pass 're-install is idempotent (one PATH entry)'

    $zip = Join-Path $mirror $asset
    $orig = Join-Path $work 'orig.zip'
    Copy-Item $zip $orig
    [IO.File]::AppendAllText($zip, 'x')
    $r = Invoke-Installer 'mirror'
    if ($r.Code -eq 0) { throw 'a tampered archive was installed' }
    if ($r.Out -notmatch 'checksum mismatch') { throw "tampered archive: unexpected output:`n$($r.Out)" }
    Pass 'tampered archive rejected (checksum mismatch)'

    Remove-Item $zip
    $r = Invoke-Installer 'mirror'
    if ($r.Code -eq 0) { throw 'install succeeded without the archive' }
    if ($r.Out -notmatch 'does not contain') { throw "missing asset: unexpected output:`n$($r.Out)" }
    Pass 'missing asset reported clearly'
    Move-Item $orig $zip

    $r = Invoke-Installer 'empty'
    if ($r.Code -eq 0) { throw 'install succeeded without a release' }
    if ($r.Out -notmatch 'not found') { throw "missing release: unexpected output:`n$($r.Out)" }
    Pass 'missing release reported clearly'
    Write-Host 'install.ps1: all checks passed'
} catch {
    $failed = $_.Exception.Message
} finally {
    Remove-Item Env:VINCI_BASE_URL -ErrorAction SilentlyContinue
    Stop-Process -Id $server.Id -Force -ErrorAction SilentlyContinue
    Remove-Item -Recurse -Force $work -ErrorAction SilentlyContinue
}
if ($failed) {
    Write-Host "FAIL  $failed"
    exit 1
}
