# å‡ºä¸€å¼ å›¾: .\draw.ps1 <spec.json>   (FreeCAD ç¦»å±, ä¸å‡ºçª—å£)
# 1) freecad.exe ç¦»å±è·‘ compile_freecad.py â†’ <base>.svg / .dxf / .FCStd / .report.json(+ å¤‡ç”¨ <base>.qt.pdf)
# 2) æ— å¤´ Edge / Chrome æŠŠ SVG æ‰“å°æˆ <base>.pdf â€”â€” Qt å†™çš„ PDF Acrobat æŠ¥é”™ã€å°ºå¯¸æ–‡å­—ä¸æ˜¾ç¤º, æµè§ˆå™¨æ‰“å°çš„æ²¡é—®é¢˜ã€‚
#    æµè§ˆå™¨ç”¨ä¸´æ—¶é…ç½®ç›®å½•, ä¸ç¢°ç”¨æˆ·è‡ªå·±çš„æµè§ˆå™¨é…ç½®; åœ¨ FreeCAD è¿›ç¨‹é‡Œèµ·æµè§ˆå™¨ä¼šæŒ‚ä½, æ‰€ä»¥æ”¾åœ¨è¿™é‡Œã€‚
param([Parameter(Mandatory = $true)][string]$Spec, [string]$FreeCAD = "$env:LOCALAPPDATA\Programs\FreeCAD 1.1\bin", [switch]$Png)
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$specPath = (Resolve-Path $Spec).Path
$env:QT_QPA_PLATFORM = 'offscreen'
$env:QT_QPA_FONTDIR = "$env:WINDIR\Fonts"
$env:PLA_DRAW_SPEC = $specPath
# FreeCAD ç”¨ç‹¬ç«‹çš„ä¸´æ—¶ç›®å½•: æ¢å¤æ–‡ä»¶ä¸è¿›ç”¨æˆ·ç¼“å­˜; æ¯æ¬¡å…ˆæ¸…ç©º â€”â€” æ®‹ç•™çš„æ¢å¤ç›®å½•ä¼šè®©å¯åŠ¨å¼¹"æ¢å¤æ–‡æ¡£"å¯¹è¯æ¡†,
# ç¦»å±æ¨¡å¼ä¸‹ä¼šä¸€ç›´æŒ‚ç€ã€‚(ä¸éš”ç¦»ç”¨æˆ·ç›®å½•: å…¨æ–°ç”¨æˆ·ç›®å½•ä¼šå¼¹ 1.x çš„é¦–æ¬¡å¯åŠ¨å‘å¯¼, åŒæ ·æŒ‚ä½ã€‚)
$fcTemp = Join-Path $env:LOCALAPPDATA 'PLAhelper\freecad-temp'
Remove-Item $fcTemp -Recurse -Force -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force $fcTemp | Out-Null
$env:FREECAD_USER_TEMP = $fcTemp
$t = Get-Date
$p = Start-Process "$FreeCAD\freecad.exe" -ArgumentList "`"$here\run_offscreen.py`"" -PassThru -WindowStyle Hidden
if (-not $p.WaitForExit(900000)) { $p.Kill(); Write-Output "timeout (FreeCAD)"; exit 1 }
Remove-Item Env:QT_QPA_PLATFORM, Env:QT_QPA_FONTDIR, Env:FREECAD_USER_HOME, Env:FREECAD_USER_TEMP -ErrorAction SilentlyContinue
if ($p.ExitCode -ne 0) { Write-Output ("compile failed (exit {0}), see compile_error.txt" -f $p.ExitCode); exit 1 }

# æ‰¾è¾“å‡º(ä¸Ž compile_freecad.py çš„è§„åˆ™ä¸€è‡´: spec.out æˆ– tools/drawing/out; basename æˆ– title.pn æˆ– drawing)
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
  $svg = $svg.Substring($svg.IndexOf('<svg'))   # å†…åµŒè¿› HTML(å†…åµŒ SVG æ‰èƒ½ç”¨ç³»ç»Ÿå­—ä½“)
  $html = "<!doctype html><meta charset='utf-8'><style>@page{size:${w}mm ${h}mm;margin:0}html,body{margin:0;padding:0}svg{display:block;width:${w}mm;height:${h}mm}</style>$svg"
  [IO.File]::WriteAllText("$tmp\page.html", $html, (New-Object Text.UTF8Encoding $false))
  if (Test-Path $pdfPath) { Remove-Item $pdfPath -Force }
  $b = Start-Process $browser -PassThru -WindowStyle Hidden -ArgumentList '--headless=new', '--disable-gpu', '--no-first-run',
    '--no-default-browser-check', '--no-pdf-header-footer', "--user-data-dir=$tmp\profile", "--print-to-pdf=$pdfPath",
    ("file:///" + ($tmp -replace '\\', '/') + "/page.html")
  if (-not $b.WaitForExit(90000)) { $b.Kill(); Write-Output "timeout (browser PDF)" }
  Start-Sleep -Milliseconds 300
  if ($Png) {
    # é¢„è§ˆå›¾(PLAhelper å‡ºå›¾é¢æ¿ç”¨): åŒä¸€é¡µ HTML æˆªå±, å®½ = çº¸å®½ mm Ã— 96/25.4 Ã— 1.5
    $pngPath = Join-Path $out "$base.png"
    $pw = [int]([double]$w * 96 / 25.4 * 1.5); $ph = [int]([double]$h * 96 / 25.4 * 1.5)
    $html2 = $html -replace 'svg\{display:block;width:[^}]*\}', "svg{display:block;width:${pw}px;height:${ph}px}"
    [IO.File]::WriteAllText("$tmp\shot.html", $html2, (New-Object Text.UTF8Encoding $false))
    if (Test-Path $pngPath) { Remove-Item $pngPath -Force }
    $b = Start-Process $browser -PassThru -WindowStyle Hidden -ArgumentList '--headless=new', '--disable-gpu', '--no-first-run',
      '--no-default-browser-check', '--hide-scrollbars', "--user-data-dir=$tmp\profile", "--window-size=$pw,$ph", "--screenshot=$pngPath",
      ("file:///" + ($tmp -replace '\\', '/') + "/shot.html")
    if (-not $b.WaitForExit(60000)) { $b.Kill(); Write-Output "timeout (browser PNG)" }
    Start-Sleep -Milliseconds 300
  }
  Remove-Item $tmp -Recurse -Force -ErrorAction SilentlyContinue
  if (-not (Test-Path $pdfPath)) { Copy-Item (Join-Path $out "$base.qt.pdf") $pdfPath -Force; Write-Output "browser PDF failed: using the Qt one" }
}
Write-Output ("done in {0:n1}s -> {1}" -f ((Get-Date) - $t).TotalSeconds, $pdfPath)
