# Launches the app (or attaches to a running instance), screenshots its window and optionally closes it.
# Usage: powershell -File scripts/screenshot.ps1 -Out shot.png [-Exe target/debug/macro-recorder.exe] [-Title "Macro Recorder"] [-WaitSeconds 4] [-Keep]
param(
    [string]$Exe = "target/debug/macro-recorder.exe",
    [string]$Title = "Macro Recorder",
    [string]$Out = "screenshots/app.png",
    [int]$WaitSeconds = 4,
    [switch]$Keep,
    [switch]$FullScreen
)

$ErrorActionPreference = "Stop"
Add-Type -AssemblyName System.Drawing
Add-Type @"
using System;
using System.Runtime.InteropServices;
public static class Win32Shot {
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern IntPtr FindWindow(string cls, string title);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hwnd, out RECT rect);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hwnd);
    [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
}
"@
[void][Win32Shot]::SetProcessDPIAware()

$proc = $null
$hwnd = [Win32Shot]::FindWindow($null, $Title)
if ($hwnd -eq [IntPtr]::Zero) {
    if (-not (Test-Path $Exe)) { throw "exe not found: $Exe" }
    $proc = Start-Process -FilePath (Resolve-Path $Exe) -PassThru
    $deadline = (Get-Date).AddSeconds($WaitSeconds + 10)
    while ($hwnd -eq [IntPtr]::Zero -and (Get-Date) -lt $deadline) {
        Start-Sleep -Milliseconds 250
        $hwnd = [Win32Shot]::FindWindow($null, $Title)
    }
    if ($hwnd -eq [IntPtr]::Zero) { throw "window '$Title' did not appear" }
    Start-Sleep -Seconds $WaitSeconds
}

[void][Win32Shot]::SetForegroundWindow($hwnd)
Start-Sleep -Milliseconds 300

$rect = New-Object Win32Shot+RECT
[void][Win32Shot]::GetWindowRect($hwnd, [ref]$rect)
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
Write-Output "saved $Out ($w x $h)"

if ($proc -and -not $Keep) { Stop-Process -Id $proc.Id -Force }
