<#
=============================================================================
 VINCI - Capture Every Moment - installer for Windows 10/11 (x64, ARM64)

   irm https://github.com/tfem1/vinci/releases/latest/download/install.ps1 | iex

 Installs vinci.exe (desktop app), vinci-cli.exe and the bundled FFmpeg into
 %LOCALAPPDATA%\vinci and adds that folder to your *user* PATH. No admin rights.

 Optional environment variables (set them before running the line above):
   $env:VINCI_VERSION  = "v0.2.0"   install that release instead of the latest
   $env:VINCI_BASE_URL = "https://..." download the release assets from a mirror

 Execution policy: the `irm ... | iex` form is not affected by the execution
 policy. If you saved this file and PowerShell says "running scripts is
 disabled on this system", run it for this one process only with:
   powershell -ExecutionPolicy Bypass -File .\install.ps1

 Every download is verified against the release's SHA256SUMS before anything
 is installed.
=============================================================================
#>

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'   # Invoke-WebRequest is much faster without it

function Install-Vinci {
    $Repo = 'tfem1/vinci'
    $FromSource = "git clone https://github.com/$Repo; cd vinci; powershell -ExecutionPolicy Bypass -File .\scripts\install-from-source.ps1"

    function Write-Ok([string]$m) { Write-Host "  [ok] $m" -ForegroundColor Green }
    function Write-Note([string]$m) { Write-Host "  [!] $m" -ForegroundColor Yellow }
    function Stop-Install([string]$m) {
        Write-Host ""
        Write-Host "  error: $m" -ForegroundColor Red
        Write-Host ""
        throw "VINCI was not installed."
    }

    Write-Host ""
    Write-Host "  V I N C I  -  Capture Every Moment" -ForegroundColor White
    Write-Host ""

    # --- Policies that would break the installer -----------------------------
    if ($ExecutionContext.SessionState.LanguageMode -ne 'FullLanguage') {
        Stop-Install ("PowerShell runs in '$($ExecutionContext.SessionState.LanguageMode)' mode on this PC " +
            "(an AppLocker / WDAC policy), which blocks this installer. Download the zip from " +
            "https://github.com/$Repo/releases/latest, extract it to $env:LOCALAPPDATA\vinci and add that folder to your PATH.")
    }
    try { [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12 } catch { }

    # --- Architecture ----------------------------------------------------------
    $arch = if ($env:PROCESSOR_ARCHITEW6432) { $env:PROCESSOR_ARCHITEW6432 } else { $env:PROCESSOR_ARCHITECTURE }
    $build = [int](Get-CimInstance Win32_OperatingSystem).BuildNumber
    switch ($arch) {
        'AMD64' { $assets = @('vinci-windows-x86_64.zip') }
        'ARM64' {
            if ($build -lt 22000) { Stop-Install "Windows 10 on ARM cannot run x64 programs; VINCI on ARM64 needs Windows 11 (x64 emulation)." }
            # Prefer a native build if a release ever ships one, else the x64 build under emulation.
            $assets = @('vinci-windows-arm64.zip', 'vinci-windows-x86_64.zip')
        }
        default { Stop-Install "Unsupported processor architecture '$arch'. VINCI supports x64 and ARM64 (via x64 emulation)." }
    }
    $version = if ($env:VINCI_VERSION) { $env:VINCI_VERSION } else { 'latest' }
    if ($env:VINCI_BASE_URL) { $base = $env:VINCI_BASE_URL.TrimEnd('/') }
    elseif ($version -eq 'latest') { $base = "https://github.com/$Repo/releases/latest/download" }
    else { $base = "https://github.com/$Repo/releases/download/$version" }
    $installDir = Join-Path $env:LOCALAPPDATA 'vinci'
    Write-Ok "Windows build $build, $arch - installing $version to $installDir"

    # --- Downloads (system proxy + Windows credentials are used automatically) -
    function Get-Asset([string]$url, [string]$dest) {
        $extra = @{}
        try {
            $proxy = [System.Net.WebRequest]::GetSystemWebProxy().GetProxy([Uri]$url)
            if ($proxy -and $proxy.AbsoluteUri -ne ([Uri]$url).AbsoluteUri) {
                $extra = @{ Proxy = $proxy.AbsoluteUri; ProxyUseDefaultCredentials = $true }
            }
        } catch { }
        try {
            Invoke-WebRequest -UseBasicParsing -Uri $url -OutFile $dest @extra
            return 200
        } catch {
            $resp = $_.Exception.Response
            if ($resp) {
                $code = [int]$resp.StatusCode
                if ($code -eq 407) {
                    Stop-Install ("your proxy requires authentication (HTTP 407). Run these two lines, then the installer again:`n" +
                        "    [System.Net.WebRequest]::DefaultWebProxy = New-Object System.Net.WebProxy('http://YOUR-PROXY:PORT')`n" +
                        "    [System.Net.WebRequest]::DefaultWebProxy.Credentials = [System.Net.CredentialCache]::DefaultNetworkCredentials")
                }
                return $code
            }
            Stop-Install ("could not reach github.com: $($_.Exception.Message)`n" +
                "  If you are behind a proxy or firewall, allow github.com and objects.githubusercontent.com,`n" +
                "  or download the zip manually from https://github.com/$Repo/releases.")
        }
    }

    $tmp = Join-Path ([IO.Path]::GetTempPath()) ("vinci-install-" + [Guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $tmp | Out-Null
    try {
        $sums = Join-Path $tmp 'SHA256SUMS'
        $code = Get-Asset "$base/SHA256SUMS" $sums
        if ($code -eq 404) {
            if ($version -eq 'latest' -and -not $env:VINCI_BASE_URL) {
                Stop-Install ("no published release found for $Repo (.../releases/latest returned 404).`n" +
                    "  Releases appear after a version tag is built by CI. Until then, build from source:`n    $FromSource")
            }
            Stop-Install "release '$version' not found at $base (SHA256SUMS returned 404)."
        }
        if ($code -ne 200) { Stop-Install "downloading $base/SHA256SUMS failed with HTTP $code." }

        $asset = $null
        foreach ($candidate in $assets) {
            $c = Get-Asset "$base/$candidate" (Join-Path $tmp $candidate)
            if ($c -eq 200) { $asset = $candidate; break }
            if ($c -ne 404) { Stop-Install "downloading $base/$candidate failed with HTTP $c." }
        }
        if (-not $asset) {
            Stop-Install ("this release does not contain $($assets -join ' or ') (release builds: linux-x86_64, linux-arm64, windows-x86_64).`n" +
                "  See https://github.com/$Repo/releases or build from source:`n    $FromSource")
        }
        if ($arch -eq 'ARM64' -and $asset -like '*x86_64*') { Write-Note "Using the x64 build (runs under Windows 11 x64 emulation)." }

        $zip = Join-Path $tmp $asset
        $line = Get-Content $sums | Where-Object { $_ -match "^([0-9a-fA-F]{64})\s+\*?$([regex]::Escape($asset))$" } | Select-Object -First 1
        if (-not $line) { Stop-Install "$asset is not listed in SHA256SUMS - refusing to install an unverified file." }
        $expected = ($line -split '\s+')[0].ToLowerInvariant()
        $actual = (Get-FileHash -Algorithm SHA256 -Path $zip).Hash.ToLowerInvariant()
        if ($expected -ne $actual) { Stop-Install "checksum mismatch for $asset (expected $expected, got $actual) - nothing was installed." }
        Write-Ok "Downloaded and verified $asset"

        # --- Install ------------------------------------------------------------
        Expand-Archive -Path $zip -DestinationPath (Join-Path $tmp 'x') -Force
        $src = Get-ChildItem (Join-Path $tmp 'x') -Directory | Select-Object -First 1
        if (-not $src -or -not (Test-Path (Join-Path $src.FullName 'vinci.exe'))) { Stop-Install "unexpected archive layout in $asset." }
        $running = Get-Process -Name 'vinci', 'vinci-cli' -ErrorAction SilentlyContinue | Where-Object { $_.Path -and $_.Path -like "$installDir\*" }
        if ($running) {
            Write-Note "Closing the running VINCI so it can be updated (clips and settings are kept)."
            $running | Stop-Process -Force
            Start-Sleep -Seconds 1
        }
        New-Item -ItemType Directory -Force -Path $installDir | Out-Null
        # Only the program files are replaced; data folders next to them are left alone.
        Copy-Item -Path (Join-Path $src.FullName '*') -Destination $installDir -Recurse -Force
        Write-Ok "Installed to $installDir"
    } finally {
        Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
    }

    # --- User PATH (idempotent; keeps %VARIABLES% unexpanded) -------------------
    $envKey = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey('Environment', $true)
    $raw = [string]$envKey.GetValue('Path', '', [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)
    $entries = @($raw -split ';' | Where-Object { $_.Trim() })
    $target = $installDir.TrimEnd('\')
    $present = $entries | Where-Object { [Environment]::ExpandEnvironmentVariables($_).TrimEnd('\') -ieq $target }
    if ($present) {
        Write-Ok "$installDir is already in your PATH"
    } else {
        $envKey.SetValue('Path', (($entries + $installDir) -join ';'), [Microsoft.Win32.RegistryValueKind]::ExpandString)
        # Notify Explorer so new terminals see the change (setting any user variable broadcasts it).
        [Environment]::SetEnvironmentVariable('VINCI_INSTALL_REFRESH', '1', 'User')
        [Environment]::SetEnvironmentVariable('VINCI_INSTALL_REFRESH', $null, 'User')
        Write-Ok "Added $installDir to your user PATH"
    }
    $envKey.Close()
    if (-not (($env:Path -split ';') | Where-Object { $_.TrimEnd('\') -ieq $target })) { $env:Path = "$env:Path;$installDir" }

    # --- Start menu shortcut ------------------------------------------------------
    try {
        $lnk = Join-Path ([Environment]::GetFolderPath('Programs')) 'VINCI.lnk'
        $s = (New-Object -ComObject WScript.Shell).CreateShortcut($lnk)
        $s.TargetPath = Join-Path $installDir 'vinci.exe'
        $s.WorkingDirectory = $installDir
        $s.Description = 'VINCI - Capture Every Moment'
        $s.Save()
        Write-Ok "Start menu shortcut"
    } catch { Write-Note "Could not create the Start menu shortcut: $($_.Exception.Message)" }

    # --- Runtime check: Microsoft Edge WebView2 (needed by the VINCI window) ----
    $wv = 'Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}'
    $hasWebView = @("HKLM:\SOFTWARE\WOW6432Node\$wv", "HKLM:\SOFTWARE\$wv", "HKCU:\Software\$wv") |
        Where-Object { (Get-ItemProperty -Path $_ -Name pv -ErrorAction SilentlyContinue).pv -match '^[1-9]' }
    if (-not $hasWebView) {
        Write-Note "Microsoft Edge WebView2 Runtime not found - the VINCI window needs it: https://developer.microsoft.com/microsoft-edge/webview2/"
    }

    Write-Host ""
    Write-Host "  Done. Start VINCI from the Start menu, or open a NEW terminal and run: vinci" -ForegroundColor Gray
    Write-Host "  Default hotkey: Alt+F10 saves the last 30 seconds. Clips go to Videos\VINCI." -ForegroundColor Gray
    Write-Host "  Uninstall: irm https://github.com/$Repo/releases/latest/download/uninstall.ps1 | iex" -ForegroundColor DarkGray
    Write-Host ""
}

Install-Vinci
