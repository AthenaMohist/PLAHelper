# R28: 生成 PLAhelper 程序图标(PH)。风格对齐 Plasticity 左上角 "P": 粗几何白字;
# H 横梁用助手强调色(琥珀 #CA8A1E), 深色圆角底。每个尺寸直接矢量绘制(小尺寸也清晰)。
# 输出: src/crates/pla-ui/assets/pla-helper.ico (PNG 压缩条目 16..256) + pla-helper.png (256)。
# 用法: powershell -ExecutionPolicy Bypass -File tools/icon/make-icon.ps1
Add-Type -AssemblyName System.Drawing
$ErrorActionPreference = 'Stop'
$root = Resolve-Path (Join-Path $PSScriptRoot '..\..')
$assets = Join-Path $root 'src\crates\pla-ui\assets'
$sizes = 16, 20, 24, 32, 40, 48, 64, 128, 256

function New-RoundRect([single]$x, [single]$y, [single]$w, [single]$h, [single]$r) {
    $p = New-Object System.Drawing.Drawing2D.GraphicsPath
    $d = 2 * $r
    $p.AddArc($x, $y, $d, $d, 180, 90)
    $p.AddArc($x + $w - $d, $y, $d, $d, 270, 90)
    $p.AddArc($x + $w - $d, $y + $h - $d, $d, $d, 0, 90)
    $p.AddArc($x, $y + $h - $d, $d, $d, 90, 90)
    $p.CloseFigure()
    $p
}

# 字形(256 网格): 字高 y 66..190, 笔画 28。小尺寸(<=24)加粗笔画、收紧字距保证可读。
function Draw-Icon([int]$size) {
    $bmp = New-Object System.Drawing.Bitmap $size, $size, ([System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.SmoothingMode = 'AntiAlias'
    $g.PixelOffsetMode = 'HighQuality'
    $g.Clear([System.Drawing.Color]::Transparent)
    $s = $size / 256.0
    $g.ScaleTransform($s, $s)

    $small = $size -le 24
    $inset = if ($small) { 0 } else { 8 }
    $tile = New-RoundRect $inset $inset (256 - 2 * $inset) (256 - 2 * $inset) $(if ($small) { 40 } else { 54 })
    $g.FillPath((New-Object System.Drawing.SolidBrush ([System.Drawing.Color]::FromArgb(255, 0x1c, 0x1c, 0x1e))), $tile)
    if (-not $small) {
        $g.DrawPath((New-Object System.Drawing.Pen ([System.Drawing.Color]::FromArgb(255, 0x3a, 0x3a, 0x3c)), 4), $tile)
    }

    $sw = if ($small) { 36 } else { 28 }        # 笔画宽
    $top = if ($small) { 56 } else { 66 }
    $bot = if ($small) { 200 } else { 190 }
    $capH = $bot - $top
    $white = New-Object System.Drawing.SolidBrush ([System.Drawing.Color]::FromArgb(255, 0xf4, 0xf4, 0xf4))
    $amber = New-Object System.Drawing.SolidBrush ([System.Drawing.Color]::FromArgb(255, 0xca, 0x8a, 0x1e))

    # ---- P: 竖干 + 半圆碗(碗高 = 字高 0.58), 内留小孔(Alternate 填充挖空) ----
    $px = if ($small) { 22 } else { 36 }
    $bowlH = [single]([math]::Round($capH * 0.58))
    $r = $bowlH / 2
    $bowlFlat = if ($small) { 26 } else { 22 }   # 碗顶平直段
    $P = New-Object System.Drawing.Drawing2D.GraphicsPath ([System.Drawing.Drawing2D.FillMode]::Alternate)
    $P.AddRectangle((New-Object System.Drawing.RectangleF $px, $top, $sw, $capH))
    $o = New-Object System.Drawing.Drawing2D.GraphicsPath
    $cx = $px + $sw + $bowlFlat
    $o.AddLine($px + $sw, $top, $cx, $top)
    $o.AddArc($cx - $r, $top, 2 * $r, 2 * $r, 270, 180)
    $o.AddLine($cx, $top + $bowlH, $px + $sw, $top + $bowlH)
    $o.CloseFigure()
    $P.AddPath($o, $false)
    $ri = $r - $sw
    if ($ri -gt 2) {
        $h = New-Object System.Drawing.Drawing2D.GraphicsPath
        $h.AddLine($px + $sw, $top + $sw, $cx, $top + $sw)
        $h.AddArc($cx - $ri, $top + $sw, 2 * $ri, 2 * $ri, 270, 180)
        $h.AddLine($cx, $top + $bowlH - $sw, $px + $sw, $top + $bowlH - $sw)
        $h.CloseFigure()
        # 碗外轮廓与竖干重叠部分不挖; 只挖碗内孔
        $g.FillPath($white, $P)
        $g.FillPath($white, $o)
        $g.FillPath((New-Object System.Drawing.SolidBrush ([System.Drawing.Color]::FromArgb(255, 0x1c, 0x1c, 0x1e))), $h)
    } else {
        $g.FillPath($white, $P)
        $g.FillPath($white, $o)
    }

    # ---- H: 两竖干白色 + 横梁琥珀色 ----
    $hx = if ($small) { 130 } else { 128 }
    $hw = if ($small) { 104 } else { 96 }     # H 外宽(横梁可见长度 = 外宽 - 2 笔画)
    $g.FillRectangle($white, $hx, $top, $sw, $capH)
    $g.FillRectangle($white, $hx + $hw - $sw, $top, $sw, $capH)
    $barY = $top + $capH / 2 - $sw / 2
    $g.FillRectangle($amber, $hx + $sw, $barY, $hw - 2 * $sw, $sw)

    $g.Dispose()
    $bmp
}

$entries = @()
foreach ($sz in $sizes) {
    $bmp = Draw-Icon $sz
    $ms = New-Object System.IO.MemoryStream
    $bmp.Save($ms, [System.Drawing.Imaging.ImageFormat]::Png)
    if ($sz -eq 256) { $bmp.Save((Join-Path $assets 'pla-helper.png'), [System.Drawing.Imaging.ImageFormat]::Png) }
    if ($env:PLA_ICON_PREVIEW) { $bmp.Save((Join-Path $env:PLA_ICON_PREVIEW "icon-$sz.png"), [System.Drawing.Imaging.ImageFormat]::Png) }
    $bmp.Dispose()
    $entries += , @($sz, $ms.ToArray())
}

# ICO: ICONDIR(6) + ICONDIRENTRY(16)*n + PNG 数据
$out = New-Object System.IO.MemoryStream
$w = New-Object System.IO.BinaryWriter $out
$w.Write([uint16]0); $w.Write([uint16]1); $w.Write([uint16]$entries.Count)
$offset = 6 + 16 * $entries.Count
foreach ($e in $entries) {
    $sz = $e[0]; $data = $e[1]
    $dim = if ($sz -ge 256) { 0 } else { $sz }
    $w.Write([byte]$dim); $w.Write([byte]$dim); $w.Write([byte]0); $w.Write([byte]0)
    $w.Write([uint16]1); $w.Write([uint16]32)
    $w.Write([uint32]$data.Length); $w.Write([uint32]$offset)
    $offset += $data.Length
}
foreach ($e in $entries) { $w.Write([byte[]]$e[1]) }
$w.Flush()
[System.IO.File]::WriteAllBytes((Join-Path $assets 'pla-helper.ico'), $out.ToArray())
"ok: $($entries.Count) sizes -> $assets\pla-helper.ico"
