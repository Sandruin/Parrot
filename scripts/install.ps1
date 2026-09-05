# Installs Parrot for the current user: copies the binary to Programs, adds a Start menu
# entry so it is found by searching for "Parrot", and registers it under Installed apps.
# Usage: powershell -ExecutionPolicy Bypass -File scripts/install.ps1 [-Force] [-Uninstall]
param(
    [string]$Exe,
    [string]$Dest = "$env:LOCALAPPDATA\Programs\Parrot",
    [switch]$Force,
    [switch]$Uninstall
)

$ErrorActionPreference = "Stop"
$AppName = "Parrot"
$shortcut = Join-Path $env:APPDATA "Microsoft\Windows\Start Menu\Programs\$AppName.lnk"
$regKey = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\$AppName"
$target = Join-Path $Dest "$AppName.exe"

function Stop-Running {
    $running = Get-Process -Name $AppName -ErrorAction SilentlyContinue
    if (-not $running) { return }
    if (-not $Force) { throw "$AppName is running. Close it first, or pass -Force." }
    $running | Stop-Process -Force
    Start-Sleep -Milliseconds 500
}

if ($Uninstall) {
    Stop-Running
    if (Test-Path $shortcut) { Remove-Item $shortcut -Force }
    if (Test-Path $regKey) { Remove-Item $regKey -Recurse -Force }
    if (Test-Path $Dest) { Remove-Item $Dest -Recurse -Force }
    Write-Output "$AppName removed. Macros in Documents\$AppName and settings in %APPDATA% were kept."
    return
}

# Fall back to the release build in this repository, building it if it is missing.
$repo = Split-Path $PSScriptRoot -Parent
if (-not $Exe) { $Exe = Join-Path $repo "target\release\macro-recorder.exe" }
if (-not (Test-Path $Exe)) {
    Write-Output "building the release binary..."
    Push-Location $repo
    try { cargo build --release } finally { Pop-Location }
}
if (-not (Test-Path $Exe)) { throw "binary not found: $Exe" }

$version = "0.1.0"
$manifest = Join-Path $repo "Cargo.toml"
if (Test-Path $manifest) {
    $match = Select-String -Path $manifest -Pattern '^version\s*=\s*"([^"]+)"' | Select-Object -First 1
    if ($match) { $version = $match.Matches[0].Groups[1].Value }
}

Stop-Running
New-Item -ItemType Directory -Force -Path $Dest | Out-Null
Copy-Item -Path $Exe -Destination $target -Force
Copy-Item -Path (Join-Path $PSScriptRoot "install.ps1") -Destination $Dest -Force

$shell = New-Object -ComObject WScript.Shell
$link = $shell.CreateShortcut($shortcut)
$link.TargetPath = $target
$link.WorkingDirectory = $Dest
$link.Description = "$AppName macro recorder"
$link.IconLocation = "$target,0"
$link.Save()

New-Item -Path $regKey -Force | Out-Null
$uninstallCmd = "powershell -NoProfile -ExecutionPolicy Bypass -File `"$Dest\install.ps1`" -Uninstall -Force"
$strings = @{
    DisplayName     = $AppName
    DisplayIcon     = $target
    DisplayVersion  = $version
    Publisher       = "Sandruin"
    InstallLocation = $Dest
    UninstallString = $uninstallCmd
}
foreach ($name in $strings.Keys) {
    New-ItemProperty -Path $regKey -Name $name -Value $strings[$name] -PropertyType String -Force | Out-Null
}
$sizeKb = [int]((Get-Item $target).Length / 1KB)
foreach ($pair in @{ NoModify = 1; NoRepair = 1; EstimatedSize = $sizeKb }.GetEnumerator()) {
    New-ItemProperty -Path $regKey -Name $pair.Key -Value $pair.Value -PropertyType DWord -Force | Out-Null
}

Write-Output "installed $AppName $version to $Dest"
Write-Output "start menu entry: $shortcut"
Write-Output "search for $AppName in the start menu, or uninstall from Installed apps"
