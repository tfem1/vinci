<#
 VINCI uninstaller for Windows:
   irm https://github.com/tfem1/vinci/releases/latest/download/uninstall.ps1 | iex
 Removes the program files from %LOCALAPPDATA%\vinci, the user PATH entry and the
 Start-menu shortcut (and an NSIS/MSI installation if there is one).
   $env:VINCI_PURGE = "1"  also delete settings, library index and cache.
 Your clips (Videos\VINCI) are NEVER deleted.
#>
$ErrorActionPreference = 'Stop'

function Uninstall-Vinci {
    $installDir = Join-Path $env:LOCALAPPDATA 'vinci'
    Get-Process -Name 'vinci', 'vinci-cli' -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue

    # Archive install (install.ps1): program files only - data/ and cache/ stay unless purging.
    foreach ($f in 'vinci.exe', 'vinci-cli.exe', 'ffmpeg.exe', 'ffprobe.exe', 'FFMPEG-LICENSE.txt', 'LICENSE', 'THIRD_PARTY_NOTICES.md') {
        Remove-Item -Force (Join-Path $installDir $f) -ErrorAction SilentlyContinue
    }

    # User PATH entry.
    $envKey = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey('Environment', $true)
    $raw = [string]$envKey.GetValue('Path', '', [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)
    $kept = @($raw -split ';' | Where-Object { $_.Trim() -and [Environment]::ExpandEnvironmentVariables($_).TrimEnd('\') -ine $installDir.TrimEnd('\') })
    if ($kept.Count -ne @($raw -split ';' | Where-Object { $_.Trim() }).Count) {
        $envKey.SetValue('Path', ($kept -join ';'), [Microsoft.Win32.RegistryValueKind]::ExpandString)
        [Environment]::SetEnvironmentVariable('VINCI_INSTALL_REFRESH', '1', 'User')
        [Environment]::SetEnvironmentVariable('VINCI_INSTALL_REFRESH', $null, 'User')
        Write-Host "  [ok] Removed $installDir from your PATH" -ForegroundColor Green
    }
    $envKey.Close()

    # Installer-based installs (NSIS / MSI), if any.
    $keys = @('HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\*', 'HKLM:\Software\Microsoft\Windows\CurrentVersion\Uninstall\*')
    foreach ($e in (Get-ItemProperty $keys -ErrorAction SilentlyContinue | Where-Object { $_.DisplayName -eq 'VINCI' })) {
        $cmd = if ($e.QuietUninstallString) { $e.QuietUninstallString } else { $e.UninstallString }
        if ($cmd -match 'msiexec') {
            $code = ($cmd -replace '.*(\{[0-9A-Fa-f-]+\}).*', '$1')
            Start-Process msiexec.exe -ArgumentList @('/x', $code, '/qn') -Wait
        } else {
            Start-Process -FilePath ($cmd -replace '^"([^"]+)".*', '$1') -ArgumentList '/S' -Wait
        }
    }

    Remove-ItemProperty -Path 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run' -Name 'VINCI' -ErrorAction SilentlyContinue
    foreach ($l in (Join-Path ([Environment]::GetFolderPath('Programs')) 'VINCI.lnk'), (Join-Path ([Environment]::GetFolderPath('Desktop')) 'VINCI.lnk')) {
        Remove-Item -Force $l -ErrorAction SilentlyContinue
    }
    if ($env:VINCI_PURGE -eq '1') {
        foreach ($d in (Join-Path $env:APPDATA 'VINCI'), (Join-Path $installDir 'data'), (Join-Path $installDir 'cache')) {
            Remove-Item -Recurse -Force $d -ErrorAction SilentlyContinue
        }
        Write-Host "  [ok] Settings, library index and cache deleted" -ForegroundColor Green
    }
    if ((Test-Path $installDir) -and -not (Get-ChildItem $installDir -Force)) { Remove-Item $installDir -Force }
    Write-Host "  [ok] VINCI removed. Your clips were not touched." -ForegroundColor Green
}

Uninstall-Vinci
