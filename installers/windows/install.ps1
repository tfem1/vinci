<#
=============================================================================
 VINCI — Capture Every Moment
 Windows installer:   irm https://github.com/tfem1/vinci/releases/latest/download/install.ps1 | iex

 Options — environment variables (work with `irm | iex`):
   $env:VINCI_VERSION   = "v1.2.0"   specific release (default: latest stable)
   $env:VINCI_CHANNEL   = "beta"     latest pre-release
   $env:VINCI_SILENT    = "1"        no installer wizard, no prompts
   $env:VINCI_AUTOSTART = "1"        start VINCI with Windows (minimized)
   $env:VINCI_DESKTOP   = "0"        do not create a desktop shortcut
   $env:VINCI_NO_LAUNCH = "1"        do not start VINCI at the end
   $env:VINCI_MSI       = "1"        use the MSI (per-machine, needs admin)
 …or as parameters:
   & ([scriptblock]::Create((irm https://github.com/tfem1/vinci/releases/latest/download/install.ps1))) -Silent -Autostart

 Security: the installer is verified against SHA256SUMS before it runs, and its
 Authenticode signature is checked when the release is code-signed. No admin
 rights are needed (per-user install into %LOCALAPPDATA%\VINCI).
=============================================================================
#>
param(
    [string]$Version = $env:VINCI_VERSION,
    [switch]$Beta = ($env:VINCI_CHANNEL -eq 'beta'),
    [switch]$Silent = ($env:VINCI_SILENT -eq '1'),
    [switch]$Autostart = ($env:VINCI_AUTOSTART -eq '1'),
    [switch]$NoDesktopShortcut = ($env:VINCI_DESKTOP -eq '0'),
    [switch]$NoLaunch = ($env:VINCI_NO_LAUNCH -eq '1'),
    [switch]$Msi = ($env:VINCI_MSI -eq '1')
)

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'   # Invoke-WebRequest is 10x faster without the progress bar

function Install-Vinci {
    $Repo = if ($env:VINCI_REPO) { $env:VINCI_REPO } else { 'tfem1/vinci' }

    function Write-Step($t) { Write-Host ""; Write-Host "> $t" -ForegroundColor White }
    function Write-Ok($t) { Write-Host "  [ok] $t" -ForegroundColor Green }
    function Write-Warn2($t) { Write-Host "  [!] $t" -ForegroundColor Yellow }
    function Stop-Install($t) { Write-Host "  [x] $t" -ForegroundColor Red; throw "VINCI installation aborted: $t" }

    Write-Host ""
    Write-Host "  V I N C I" -ForegroundColor White
    Write-Host "  Capture Every Moment" -ForegroundColor DarkGray

    # TLS 1.2+ on Windows PowerShell 5.1
    try { [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12 } catch {}

    # ---------------------------------------------------------------- system
    Write-Step "Checking your system"
    if ($PSVersionTable.PSEdition -ne 'Core' -and $PSVersionTable.PSVersion.Major -lt 5) { Stop-Install "PowerShell 5.1 or newer is required" }
    if (-not [Environment]::Is64BitOperatingSystem) { Stop-Install "VINCI requires 64-bit Windows" }
    $build = [int](Get-CimInstance Win32_OperatingSystem).BuildNumber
    if ($build -lt 17763) { Stop-Install "Windows 10 version 1809 (build 17763) or newer is required (you have build $build)" }
    $osName = if ($build -ge 22000) { "Windows 11" } else { "Windows 10" }
    $arch = $env:PROCESSOR_ARCHITECTURE
    if ($arch -eq 'ARM64') {
        Write-Warn2 "ARM64 detected: the x64 build runs under emulation (hardware encoding depends on your GPU drivers)."
    } elseif ($arch -ne 'AMD64') { Stop-Install "Unsupported architecture: $arch" }
    Write-Ok "$osName (build $build) - $arch"
    if ($build -lt 18362) {
        Write-Warn2 "Windows Graphics Capture needs Windows 10 1903+; VINCI will use Desktop Duplication instead."
    }
    $isAdmin = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
    if ($Msi -and -not $isAdmin) { Stop-Install "The MSI installs for all users and needs an elevated PowerShell. Omit VINCI_MSI for a per-user install (no admin)." }

    # --------------------------------------------------------------- release
    Write-Step "Finding the release"
    $asset = if ($Msi) { 'VINCI-windows-x64.msi' } else { 'VINCI-windows-x64-setup.exe' }
    if ($env:VINCI_BASE_URL) {
        $base = $env:VINCI_BASE_URL.TrimEnd('/')
        if (-not $base.StartsWith('https://') -and $env:VINCI_ALLOW_HTTP -ne '1') { Stop-Install "Refusing non-HTTPS mirror $base" }
    } else {
        if ($Repo -like 'USERNAME/*') { Stop-Install "This installer is not configured yet (repository placeholder). Maintainers: run scripts/configure-project.sh" }
        if ($Version) {
            $base = "https://github.com/$Repo/releases/download/$Version"
        } elseif ($Beta) {
            $releases = Invoke-RestMethod -UseBasicParsing -Uri "https://api.github.com/repos/$Repo/releases?per_page=15" -Headers @{ 'User-Agent' = 'vinci-installer' }
            $rel = $releases | Where-Object { -not $_.draft } | Select-Object -First 1
            if (-not $rel) { Stop-Install "No release found" }
            $Version = $rel.tag_name
            $base = "https://github.com/$Repo/releases/download/$Version"
        } else {
            $base = "https://github.com/$Repo/releases/latest/download"
            $Version = 'latest'
        }
    }
    Write-Ok "$Repo - $Version$(if ($Beta) { ' (beta)' })"

    # -------------------------------------------------------------- download
    Write-Step "Downloading"
    $tmp = Join-Path ([IO.Path]::GetTempPath()) ("vinci-install-" + [Guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $tmp | Out-Null
    try {
        $sumsPath = Join-Path $tmp 'SHA256SUMS'
        $filePath = Join-Path $tmp $asset
        Invoke-WebRequest -UseBasicParsing -Uri "$base/SHA256SUMS" -OutFile $sumsPath
        Invoke-WebRequest -UseBasicParsing -Uri "$base/$asset" -OutFile $filePath

        # SHA-256 verification (mandatory)
        $expected = (Get-Content $sumsPath | Where-Object { $_ -match "^([0-9a-fA-F]{64})\s+\*?$([regex]::Escape($asset))$" } | Select-Object -First 1)
        if (-not $expected) { Stop-Install "$asset is not listed in SHA256SUMS" }
        $expected = ($expected -split '\s+')[0].ToLowerInvariant()
        $actual = (Get-FileHash -Algorithm SHA256 -Path $filePath).Hash.ToLowerInvariant()
        if ($expected -ne $actual) { Stop-Install "CHECKSUM MISMATCH for $asset (expected $expected, got $actual). Nothing was installed." }
        Write-Ok "$asset verified (SHA-256 $actual)"

        # Authenticode (when the release is code-signed)
        $sig = Get-AuthenticodeSignature -FilePath $filePath
        switch ($sig.Status) {
            'Valid' { Write-Ok "Signed by $($sig.SignerCertificate.Subject)" }
            'NotSigned' { Write-Warn2 "The installer is not code-signed (integrity is guaranteed by the SHA-256 check above). Windows SmartScreen may warn on first launch." }
            default { Stop-Install "Invalid Authenticode signature ($($sig.Status)). Nothing was installed." }
        }

        # --------------------------------------------------------------- install
        Write-Step "Installing VINCI"
        if (Get-Process -Name 'vinci' -ErrorAction SilentlyContinue) {
            Write-Warn2 "Closing the running VINCI (your clips and settings are kept)."
            Get-Process -Name 'vinci' -ErrorAction SilentlyContinue | Stop-Process -Force
            Start-Sleep -Seconds 1
        }
        if ($Msi) {
            $instArgs = @('/i', "`"$filePath`"", '/norestart', $(if ($Silent) { '/qn' } else { '/passive' }))
            $p = Start-Process -FilePath 'msiexec.exe' -ArgumentList $instArgs -Wait -PassThru
        } else {
            # NSIS per-user installer: /S = silent.
            if ($Silent) { $p = Start-Process -FilePath $filePath -ArgumentList '/S' -Wait -PassThru }
            else { $p = Start-Process -FilePath $filePath -Wait -PassThru }
        }
        if ($p.ExitCode -ne 0) { Stop-Install "The installer exited with code $($p.ExitCode)" }
    } finally {
        Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
    }

    # ---------------------------------------------------------------- verify
    Write-Step "Verifying"
    $candidates = @(
        (Join-Path $env:LOCALAPPDATA 'VINCI\vinci.exe'),
        (Join-Path $env:LOCALAPPDATA 'Programs\VINCI\vinci.exe'),
        (Join-Path $env:ProgramFiles 'VINCI\vinci.exe')
    )
    $uninstallKeys = @('HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\*', 'HKLM:\Software\Microsoft\Windows\CurrentVersion\Uninstall\*')
    $reg = Get-ItemProperty $uninstallKeys -ErrorAction SilentlyContinue | Where-Object { $_.DisplayName -eq 'VINCI' } | Select-Object -First 1
    if ($reg -and $reg.InstallLocation) { $candidates = @((Join-Path $reg.InstallLocation 'vinci.exe')) + $candidates }
    $exe = $candidates | Where-Object { $_ -and (Test-Path $_) } | Select-Object -First 1
    if (-not $exe) { Stop-Install "vinci.exe was not found after installation" }
    $ffmpeg = Join-Path (Split-Path $exe) 'ffmpeg.exe'
    if (-not (Test-Path $ffmpeg)) { Write-Warn2 "Bundled FFmpeg not found next to vinci.exe — capture will not work until it is present." }
    $ver = (Get-Item $exe).VersionInfo.ProductVersion
    Write-Ok "VINCI $ver installed at $exe"

    # ------------------------------------------------------------- shortcuts
    $shell = New-Object -ComObject WScript.Shell
    $startMenu = Join-Path ([Environment]::GetFolderPath('Programs')) 'VINCI\VINCI.lnk'
    if (-not (Test-Path $startMenu) -and -not (Get-ChildItem ([Environment]::GetFolderPath('Programs')) -Recurse -Filter 'VINCI.lnk' -ErrorAction SilentlyContinue)) {
        New-Item -ItemType Directory -Force -Path (Split-Path $startMenu) | Out-Null
        $lnk = $shell.CreateShortcut($startMenu); $lnk.TargetPath = $exe; $lnk.WorkingDirectory = (Split-Path $exe); $lnk.Description = 'VINCI — Capture Every Moment'; $lnk.Save()
    }
    Write-Ok "Start menu entry"
    $desktop = Join-Path ([Environment]::GetFolderPath('Desktop')) 'VINCI.lnk'
    if (-not $NoDesktopShortcut -and -not (Test-Path $desktop)) {
        $lnk = $shell.CreateShortcut($desktop); $lnk.TargetPath = $exe; $lnk.WorkingDirectory = (Split-Path $exe); $lnk.Description = 'VINCI — Capture Every Moment'; $lnk.Save()
        Write-Ok "Desktop shortcut"
    }

    # ------------------------------------------------------------- autostart
    $runKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
    if ($Autostart) {
        New-ItemProperty -Path $runKey -Name 'VINCI' -Value "`"$exe`" --minimized" -PropertyType String -Force | Out-Null
        Write-Ok "VINCI will start with Windows (minimized to the tray)"
    }

    Write-Host ""
    Write-Host "  Your clips go to $([Environment]::GetFolderPath('MyVideos'))\VINCI. Default hotkey: Alt+F10 (save the last 30 s)." -ForegroundColor Gray
    Write-Host "  Uninstall: Settings > Apps > VINCI, or: irm https://github.com/tfem1/vinci/releases/latest/download/uninstall.ps1 | iex" -ForegroundColor DarkGray

    if (-not $NoLaunch) {
        $launch = $true
        if (-not $Silent -and [Environment]::UserInteractive -and $Host.UI.RawUI) {
            $answer = Read-Host "  Launch VINCI now? [Y/n]"
            $launch = -not ($answer -match '^[Nn]')
        }
        if ($launch) { Start-Process -FilePath $exe; Write-Ok "VINCI is starting — look for it in the system tray." }
    }
}

Install-Vinci
