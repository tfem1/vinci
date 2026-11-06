<#
 VINCI uninstaller:  irm https://github.com/tfem1/vinci/releases/latest/download/uninstall.ps1 | iex
   $env:VINCI_PURGE = "1"  also delete settings, library index and cache.
 Your clips (Videos\VINCI) are NEVER deleted.
#>
$ErrorActionPreference = 'Stop'

function Uninstall-Vinci {
    Get-Process -Name 'vinci' -ErrorAction SilentlyContinue | Stop-Process -Force
    $keys = @('HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\*', 'HKLM:\Software\Microsoft\Windows\CurrentVersion\Uninstall\*')
    $entries = Get-ItemProperty $keys -ErrorAction SilentlyContinue | Where-Object { $_.DisplayName -eq 'VINCI' }
    if (-not $entries) { Write-Host "  VINCI does not seem to be installed." -ForegroundColor Yellow }
    foreach ($e in $entries) {
        $cmd = if ($e.QuietUninstallString) { $e.QuietUninstallString } else { $e.UninstallString }
        if ($cmd -match 'msiexec') {
            $code = ($cmd -replace '.*(\{[0-9A-Fa-f-]+\}).*', '$1')
            Start-Process msiexec.exe -ArgumentList @('/x', $code, '/qn') -Wait
        } else {
            $exe = ($cmd -replace '^"([^"]+)".*', '$1')
            Start-Process -FilePath $exe -ArgumentList '/S' -Wait
        }
        Write-Host "  [ok] VINCI removed" -ForegroundColor Green
    }
    Remove-ItemProperty -Path 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run' -Name 'VINCI' -ErrorAction SilentlyContinue
    Remove-Item (Join-Path ([Environment]::GetFolderPath('Desktop')) 'VINCI.lnk') -ErrorAction SilentlyContinue
    if ($env:VINCI_PURGE -eq '1') {
        foreach ($d in @((Join-Path $env:APPDATA 'VINCI'), (Join-Path $env:LOCALAPPDATA 'VINCI\data'), (Join-Path $env:LOCALAPPDATA 'VINCI\cache'))) {
            Remove-Item -Recurse -Force $d -ErrorAction SilentlyContinue
        }
        Write-Host "  [ok] Settings, library index and cache deleted" -ForegroundColor Green
    }
    Write-Host "  Your clips were not touched." -ForegroundColor Gray
}

Uninstall-Vinci
