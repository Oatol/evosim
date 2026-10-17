# 点击客户区坐标（相对客户区左上角）。
# 用法：powershell -File tools/click.ps1 -Points "780,13;1405,13" [-Key 0x20]
param(
    [string]$Points = "",
    [string]$ProcName = "evosim-app",
    [string]$Keys = ""      # 逗号分隔的虚拟键码，点击后依次按下抬起，如 "32"=空格
)

Add-Type @"
using System;
using System.Runtime.InteropServices;
public class M {
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] public static extern void mouse_event(uint f, uint dx, uint dy, uint d, IntPtr e);
    [DllImport("user32.dll")] public static extern void keybd_event(byte k, byte s, uint f, IntPtr e);
    [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr h, ref POINT p);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
    public struct POINT { public int X, Y; }
}
"@

$p = Get-Process -Name $ProcName -ErrorAction Stop | Where-Object { $_.MainWindowHandle -ne 0 } | Select-Object -First 1
$h = $p.MainWindowHandle
[void][M]::SetForegroundWindow($h)
Start-Sleep -Milliseconds 300

$pt = New-Object M+POINT
[void][M]::ClientToScreen($h, [ref]$pt)

foreach ($pair in ($Points.Split(";") | Where-Object { $_ -ne "" })) {
    $a = $pair.Split(",") | ForEach-Object { [int]$_ }
    [void][M]::SetCursorPos($pt.X + $a[0], $pt.Y + $a[1])
    Start-Sleep -Milliseconds 130
    [M]::mouse_event(0x0002, 0, 0, 0, [IntPtr]::Zero)
    Start-Sleep -Milliseconds 70
    [M]::mouse_event(0x0004, 0, 0, 0, [IntPtr]::Zero)
    Start-Sleep -Milliseconds 300
    Write-Output "click $($a[0]),$($a[1])"
}

foreach ($k in ($Keys.Split(",") | Where-Object { $_ -ne "" })) {
    $code = [byte][int]$k
    [M]::keybd_event($code, 0, 0, [IntPtr]::Zero)
    Start-Sleep -Milliseconds 60
    [M]::keybd_event($code, 0, 2, [IntPtr]::Zero)
    Start-Sleep -Milliseconds 250
    Write-Output "key $code"
}
