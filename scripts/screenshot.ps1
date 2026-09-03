# Launches the app (or attaches to a running instance), screenshots its window and optionally closes it.
# Usage: powershell -File scripts/screenshot.ps1 -Out shot.png [-Exe target/debug/macro-recorder.exe] [-WaitSeconds 4] [-Keep] [-FullScreen]
param(
    [string]$Exe = "target/debug/macro-recorder.exe",
    [string]$ProcessName = "macro-recorder",
    [string]$Out = "screenshots/app.png",
    [int]$WaitSeconds = 4,
    [switch]$Keep,
    [switch]$FullScreen
)

$ErrorActionPreference = "Stop"
Add-Type -AssemblyName System.Drawing
Add-Type -AssemblyName System.Windows.Forms
Add-Type @"
using System;
using System.Runtime.InteropServices;
public static class Win32Shot {
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hwnd, out RECT rect);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hwnd);
    [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
}
"@
[void][Win32Shot]::SetProcessDPIAware()

function Get-WindowRect([IntPtr]$hwnd) {
    $r = New-Object Win32Shot+RECT
    [void][Win32Shot]::GetWindowRect($hwnd, [ref]$r)
    return $r
}

$started = $false
$proc = Get-Process -Name $ProcessName -ErrorAction SilentlyContinue | Select-Object -First 1
if (-not $proc) {
    if (-not (Test-Path $Exe)) { throw "exe not found: $Exe" }
    $proc = Start-Process -FilePath (Resolve-Path $Exe) -PassThru
    $started = $true
}

# winit first creates a tiny placeholder window, so wait for a titled window of real size.
$deadline = (Get-Date).AddSeconds($WaitSeconds + 20)
$hwnd = [IntPtr]::Zero
while ((Get-Date) -lt $deadline) {
    Start-Sleep -Milliseconds 250
    $proc.Refresh()
    $hwnd = $proc.MainWindowHandle
    if ($hwnd -ne [IntPtr]::Zero -and $proc.MainWindowTitle -ne "") {
        $r = Get-WindowRect $hwnd
        if (($r.Right - $r.Left) -gt 200) { break }
    }
    $hwnd = [IntPtr]::Zero
}
if ($hwnd -eq [IntPtr]::Zero) { throw "process $($proc.Id) never got a main window" }
if ($started) { Start-Sleep -Seconds $WaitSeconds }

[void][Win32Shot]::SetForegroundWindow($hwnd)
Start-Sleep -Milliseconds 400

$rect = Get-WindowRect $hwnd
if ($FullScreen) {
    $bounds = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds
    $rect.Left = $bounds.Left; $rect.Top = $bounds.Top; $rect.Right = $bounds.Right; $rect.Bottom = $bounds.Bottom
}
$w = $rect.Right - $rect.Left
$h = $rect.Bottom - $rect.Top
$bmp = New-Object System.Drawing.Bitmap $w, $h
$g = [System.Drawing.Graphics]::FromImage($bmp)
$g.CopyFromScreen($rect.Left, $rect.Top, 0, 0, $bmp.Size)
$dir = Split-Path -Parent $Out
if ($dir -and -not (Test-Path $dir)) { New-Item -ItemType Directory -Path $dir | Out-Null }
$bmp.Save((Join-Path (Get-Location) $Out), [System.Drawing.Imaging.ImageFormat]::Png)
$g.Dispose(); $bmp.Dispose()
Write-Output "saved $Out ($w x $h) title=[$($proc.MainWindowTitle)]"

if ($started -and -not $Keep) { Stop-Process -Id $proc.Id -Force }
