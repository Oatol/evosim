# 原生窗口截图：按客户区裁剪，让图像像素 == egui 逻辑像素。
# 用法：powershell -File tools/shot.ps1 -Out d6.png [-Crop x,y,w,h]
param(
    [string]$Out = "shot.png",
    [string]$Crop = "",
    [int]$Scale = 1,
    [string]$ProcName = "evosim-app"
)

Add-Type -AssemblyName System.Drawing
$Out = [System.IO.Path]::GetFullPath((Join-Path (Get-Location).Path $Out))

Add-Type @"
using System;
using System.Runtime.InteropServices;
public class W {
    [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr h, ref POINT p);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
    public struct RECT { public int Left, Top, Right, Bottom; }
    public struct POINT { public int X, Y; }
}
"@

$p = Get-Process -Name $ProcName -ErrorAction Stop | Where-Object { $_.MainWindowHandle -ne 0 } | Select-Object -First 1
$h = $p.MainWindowHandle
[void][W]::SetForegroundWindow($h)
Start-Sleep -Milliseconds 250

$rc = New-Object W+RECT
[void][W]::GetClientRect($h, [ref]$rc)
$pt = New-Object W+POINT
[void][W]::ClientToScreen($h, [ref]$pt)

$w = $rc.Right - $rc.Left
$hh = $rc.Bottom - $rc.Top
$bmp = New-Object System.Drawing.Bitmap($w, $hh)
$g = [System.Drawing.Graphics]::FromImage($bmp)
$g.CopyFromScreen($pt.X, $pt.Y, 0, 0, (New-Object System.Drawing.Size($w, $hh)))
$g.Dispose()

if ($Crop -ne "") {
    $a = $Crop.Split(",") | ForEach-Object { [int]$_ }
    $ow = $a[2] * $Scale
    $oh = $a[3] * $Scale
    $c = New-Object System.Drawing.Bitmap($ow, $oh)
    $g2 = [System.Drawing.Graphics]::FromImage($c)
    # 最近邻：UI 是像素精确的，插值会把 1px 的轨道线糊掉
    $g2.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::NearestNeighbor
    $g2.PixelOffsetMode = [System.Drawing.Drawing2D.PixelOffsetMode]::Half
    $g2.DrawImage($bmp, (New-Object System.Drawing.Rectangle(0, 0, $ow, $oh)), (New-Object System.Drawing.Rectangle($a[0], $a[1], $a[2], $a[3])), [System.Drawing.GraphicsUnit]::Pixel)
    $g2.Dispose()
    $c.Save($Out, [System.Drawing.Imaging.ImageFormat]::Png)
    $c.Dispose()
} else {
    $bmp.Save($Out, [System.Drawing.Imaging.ImageFormat]::Png)
}
$bmp.Dispose()
Write-Output "client=${w}x${hh} at ${($pt.X)},${($pt.Y)} -> $Out"
