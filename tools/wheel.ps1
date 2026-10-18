# Scroll the wheel at a client-area position.
# Usage: powershell -File tools/wheel.ps1 -At "200,551" [-Notches 3]  (positive = up)
# NOTE: keep this file ASCII-only. Windows PowerShell 5.1 reads .ps1 as ANSI,
# so UTF-8 Chinese comments can eat the following line and break parsing.
param(
    [string]$At = "",
    [int]$Notches = 3,
    [string]$ProcName = "evosim-app"
)

Add-Type @"
using System;
using System.Runtime.InteropServices;
public class WH {
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] public static extern void mouse_event(uint f, uint dx, uint dy, int d, IntPtr e);
    [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr h, ref POINT p);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
    public struct POINT { public int X, Y; }
}
"@

$p = Get-Process -Name $ProcName -ErrorAction Stop | Where-Object { $_.MainWindowHandle -ne 0 } | Select-Object -First 1
$h = $p.MainWindowHandle
[void][WH]::SetForegroundWindow($h)
Start-Sleep -Milliseconds 300

$pt = New-Object WH+POINT
[void][WH]::ClientToScreen($h, [ref]$pt)
$a = $At.Split(",") | ForEach-Object { [int]$_ }
[void][WH]::SetCursorPos($pt.X + $a[0], $pt.Y + $a[1])
Start-Sleep -Milliseconds 200

$dir = if ($Notches -ge 0) { 1 } else { -1 }
for ($i = 0; $i -lt [Math]::Abs($Notches); $i++) {
    [WH]::mouse_event(0x0800, 0, 0, (120 * $dir), [IntPtr]::Zero)   # MOUSEEVENTF_WHEEL
    Start-Sleep -Milliseconds 90
}
Start-Sleep -Milliseconds 250
Write-Output "wheel $Notches at $At"
