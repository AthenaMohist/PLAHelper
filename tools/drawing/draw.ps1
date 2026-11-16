# 出一张图: .\draw.ps1 <spec.json>   (FreeCAD 离屏, 不出窗口)
# 1) freecad.exe 离屏跑 compile_freecad.py → <base>.svg / .dxf / .FCStd / .report.json(+ 备用 <base>.qt.pdf)
# 2) 无头 Edge / Chrome 把 SVG 打印成 <base>.pdf —— Qt 写的 PDF Acrobat 报错、尺寸文字不显示, 浏览器打印的没问题。
#    浏览器用临时配置目录, 不碰用户自己的浏览器配置; 在 FreeCAD 进程里起浏览器会挂住, 所以放在这里。
param([Parameter(Mandatory = $true)][string]$Spec, [string]$FreeCAD = "$env:LOCALAPPDATA\Programs\FreeCAD 1.1\bin")
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$specPath = (Resolve-Path $Spec).Path
$env:QT_QPA_PLATFORM = 'offscreen'
$env:QT_QPA_FONTDIR = "$env:WINDIR\Fonts"
$env:PLA_DRAW_SPEC = $specPath
# FreeCAD 用独立的临时目录: 恢复文件不进用户缓存; 每次先清空 —— 残留的恢复目录会让启动弹"恢复文档"对话框,
# 离屏模式下会一直挂着。(不隔离用户目录: 全新用户目录会弹 1.x 的首次启动向导, 同样挂住。)
$fcTemp = Join-Path $env:LOCALAPPDATA 'PLAhelper\freecad-temp'
Remove-Item $fcTemp -Recurse -Force -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force $fcTemp | Out-Null
$env:FREECAD_USER_TEMP = $fcTemp
$t = Get-Date
$p = Start-Process "$FreeCAD\freecad.exe" -ArgumentList "`"$here\run_offscreen.py`"" -PassThru -WindowStyle Hidden
if (-not $p.WaitForExit(240000)) { $p.Kill(); Write-Output "timeout (FreeCAD)"; exit 1 }
Remove-Item Env:QT_QPA_PLATFORM, Env:QT_QPA_FONTDIR, Env:FREECAD_USER_HOME, Env:FREECAD_USER_TEMP -ErrorAction SilentlyContinue
if ($p.ExitCode -ne 0) { Write-Output ("compile failed (exit {0}), see compile_error.txt" -f $p.ExitCode); exit 1 }

# 找输出(与 compile_freecad.py 的规则一致: spec.out 或 tools/drawing/out; basename 或 title.pn 或 drawing)
$s = Get-Content -Raw -Encoding utf8 $specPath | ConvertFrom-Json
$out = if ($s.out) { $s.out } else { Join-Path $here 'out' }
$base = if ($s.basename) { $s.basename } elseif ($s.title -and $s.title.pn) { $s.title.pn } else { 'drawing' }
$svgPath = Join-Path $out "$base.svg"
$pdfPath = Join-Path $out "$base.pdf"
$rep = Get-Content -Raw -Encoding utf8 (Join-Path $out "$base.report.json") | ConvertFrom-Json
$w, $h = $rep.paper_mm[0], $rep.paper_mm[1]

$browser = @("${env:ProgramFiles(x86)}\Microsoft\Edge\Application\msedge.exe", "$env:ProgramFiles\Microsoft\Edge\Application\msedge.exe",
  "$env:ProgramFiles\Google\Chrome\Application\chrome.exe") | Where-Object { Test-Path $_ } | Select-Object -First 1
if (-not $browser -or -not (Test-Path $svgPath)) {
  Copy-Item (Join-Path $out "$base.qt.pdf") $pdfPath -Force
  Write-Output "no Edge/Chrome: PDF is the Qt one (Acrobat may complain)"
} else {
  $tmp = Join-Path $env:TEMP ("pla-draw-" + [guid]::NewGuid().ToString('N').Substring(0, 8))
  New-Item -ItemType Directory -Force $tmp | Out-Null
  $svg = [IO.File]::ReadAllText($svgPath)
  $svg = $svg.Substring($svg.IndexOf('<svg'))   # 内嵌进 HTML(内嵌 SVG 才能用系统字体)
  $html = "<!doctype html><meta charset='utf-8'><style>@page{size:${w}mm ${h}mm;margin:0}html,body{margin:0;padding:0}svg{display:block;width:${w}mm;height:${h}mm}</style>$svg"
  [IO.File]::WriteAllText("$tmp\page.html", $html, (New-Object Text.UTF8Encoding $false))
  if (Test-Path $pdfPath) { Remove-Item $pdfPath -Force }
  $b = Start-Process $browser -PassThru -WindowStyle Hidden -ArgumentList '--headless=new', '--disable-gpu', '--no-first-run',
    '--no-default-browser-check', '--no-pdf-header-footer', "--user-data-dir=$tmp\profile", "--print-to-pdf=$pdfPath",
    ("file:///" + ($tmp -replace '\\', '/') + "/page.html")
  if (-not $b.WaitForExit(90000)) { $b.Kill(); Write-Output "timeout (browser PDF)" }
  Start-Sleep -Milliseconds 300
  Remove-Item $tmp -Recurse -Force -ErrorAction SilentlyContinue
  if (-not (Test-Path $pdfPath)) { Copy-Item (Join-Path $out "$base.qt.pdf") $pdfPath -Force; Write-Output "browser PDF failed: using the Qt one" }
}
Write-Output ("done in {0:n1}s -> {1}" -f ((Get-Date) - $t).TotalSeconds, $pdfPath)
