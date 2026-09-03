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
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;
public static class Win32Shot {
    public delegate bool EnumProc(IntPtr hwnd, IntPtr lparam);
    [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc cb, IntPtr lparam);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr hwnd);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint pid);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetWindowText(IntPtr hwnd, StringBuilder text, int max);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hwnd, out RECT rect);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hwnd);
    [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }

    // Visible top-level windows of a process as (handle, title) pairs.
    public static List<KeyValuePair<IntPtr, string>> WindowsOf(uint pid) {
        var found = new List<KeyValuePair<IntPtr, string>>();
        EnumWindows((hwnd, lparam) => {
            uint owner;
            GetWindowThreadProcessId(hwnd, out owner);
            if (owner == pid && IsWindowVisible(hwnd)) {
                var sb = new StringBuilder(512);
                GetWindowText(hwnd, sb, sb.Capacity);
                found.Add(new KeyValuePair<IntPtr, string>(hwnd, sb.ToString()));
            }
            return true;
        }, IntPtr.Zero);
        return found;
    }
}
"@
[void][Win32Shot]::SetProcessDPIAware()

function Get-WindowRect([IntPtr]$hwnd) {
    $r = New-Object Win32Shot+RECT
    [void][Win32Shot]::GetWindowRect($hwnd, [ref]$r)
    return $r
}

# The app's main window: titled, wide enough, and not the click-through overlay.
function Find-AppWindow([int]$processId) {
    foreach ($pair in [Win32Shot]::WindowsOf([uint32]$processId)) {
        if ($pair.Value -eq "" -or $pair.Value -like "*Overlay*") { continue }
        $r = Get-WindowRect $pair.Key
        if (($r.Right - $r.Left) -gt 200) { return @{ Handle = $pair.Key; Title = $pair.Value } }
    }
    return $null
}

$started = $false
$proc = Get-Process -Name $ProcessName -ErrorAction SilentlyContinue | Select-Object -First 1
if (-not $proc) {
    if (-not (Test-Path $Exe)) { throw "exe not found: $Exe" }
    $proc = Start-Process -FilePath (Resolve-Path $Exe) -PassThru
    $started = $true
}

$deadline = (Get-Date).AddSeconds($WaitSeconds + 20)
$window = $null
while ((Get-Date) -lt $deadline) {
    Start-Sleep -Milliseconds 250
    $window = Find-AppWindow $proc.Id
    if ($window) { break }
}
if (-not $window) { throw "process $($proc.Id) never got a main window" }
if ($started) { Start-Sleep -Seconds $WaitSeconds }

[void][Win32Shot]::SetForegroundWindow($window.Handle)
Start-Sleep -Milliseconds 400

$rect = Get-WindowRect $window.Handle
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
Write-Output "saved $Out ($w x $h) title=[$($window.Title)]"

if ($started -and -not $Keep) { Stop-Process -Id $proc.Id -Force }

