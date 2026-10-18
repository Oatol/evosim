# 在客户区里拖一条线：按下 → 分步移动 → 抬起。
# egui 的拖动需要中间有移动帧，所以不能只在终点抬起。
# 用法：powershell -File tools/drag.ps1 -From "84,562" -To "130,562" [-Steps 12]
param(
    [string]$From = "",
    [string]$To = "",
    [int]$Steps = 12,
    [string]$ProcName = "evosim-app"
)

Add-Type @"
using System;
using System.Runtime.InteropServices;
public class D {
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] public static extern void mouse_event(uint f, uint dx, uint dy, uint d, IntPtr e);
    [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr h, ref POINT p);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
    public struct POINT { public int X, Y; }
}
"@

$p = Get-Process -Name $ProcName -ErrorAction Stop | Where-Object { $_.MainWindowHandle -ne 0 } | Select-Object -First 1
$h = $p.MainWindowHandle
[void][D]::SetForegroundWindow($h)
Start-Sleep -Milliseconds 300

$pt = New-Object D+POINT
[void][D]::ClientToScreen($h, [ref]$pt)

$a = $From.Split(",") | ForEach-Object { [int]$_ }
$b = $To.Split(",") | ForEach-Object { [int]$_ }

[void][D]::SetCursorPos($pt.X + $a[0], $pt.Y + $a[1])
Start-Sleep -Milliseconds 150
[D]::mouse_event(0x0002, 0, 0, 0, [IntPtr]::Zero)   # left down
Start-Sleep -Milliseconds 120

for ($i = 1; $i -le $Steps; $i++) {
    $x = $a[0] + [int](($b[0] - $a[0]) * $i / $Steps)
    $y = $a[1] + [int](($b[1] - $a[1]) * $i / $Steps)
    [void][D]::SetCursorPos($pt.X + $x, $pt.Y + $y)
    Start-Sleep -Milliseconds 60
}

Start-Sleep -Milliseconds 120
[D]::mouse_event(0x0004, 0, 0, 0, [IntPtr]::Zero)   # left up
Start-Sleep -Milliseconds 250
Write-Output "drag $From -> $To"
