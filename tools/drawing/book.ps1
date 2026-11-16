# 多张图合成一份 PDF: .\book.ps1 -Out <合订.pdf> -Svg <a.svg>,<b.svg>,…
# 每张 SVG 一页, 纸张按同名 .report.json 的 paper_mm(没有就按 SVG 的 width/height); 不同图幅用 CSS 命名页。
# 与 draw.ps1 一样用无头 Edge / Chrome 打印(临时配置目录, 不碰用户的浏览器配置)。
#   或 -List <文本文件, 每行一个 SVG 路径>(PLAhelper 用这个: 文件名里可能有逗号)
param([Parameter(Mandatory = $true)][string]$Out, [string[]]$Svg, [string]$List)
if ($List) { $Svg = @(Get-Content -Encoding utf8 $List | Where-Object { $_.Trim() }) }
$browser = @("${env:ProgramFiles(x86)}\Microsoft\Edge\Application\msedge.exe", "$env:ProgramFiles\Microsoft\Edge\Application\msedge.exe",
  "$env:ProgramFiles\Google\Chrome\Application\chrome.exe") | Where-Object { Test-Path $_ } | Select-Object -First 1
if (-not $browser) { Write-Output "no Edge/Chrome"; exit 1 }
$Out = [IO.Path]::GetFullPath((Join-Path (Get-Location).Path $Out))  # 浏览器的工作目录不是这里
$pages = New-Object System.Collections.Generic.List[string]
$sizes = @{}
foreach ($f in $Svg) {
  if (-not (Test-Path $f)) { Write-Output "missing: $f"; continue }
  $text = [IO.File]::ReadAllText($f)
  $text = $text.Substring($text.IndexOf('<svg'))
  $rep = [IO.Path]::ChangeExtension($f, $null).TrimEnd('.') + '.report.json'
  $w = $null; $h = $null
  if (Test-Path $rep) {
    $r = Get-Content -Raw -Encoding utf8 $rep | ConvertFrom-Json
    if ($r.paper_mm) { $w, $h = $r.paper_mm[0], $r.paper_mm[1] }
  }
  if (-not $w) {
    $m = [regex]::Match($text, 'width="([0-9.]+)mm"\s+height="([0-9.]+)mm"')
    if ($m.Success) { $w, $h = $m.Groups[1].Value, $m.Groups[2].Value } else { $w, $h = 420, 297 }
  }
  # 同一 HTML 里多个内嵌 SVG: id(剖面线图案等)加页号前缀, 免得互相顶掉
  $k = "g$($pages.Count)_"
  $text = [regex]::Replace($text, '\bid="([^"]+)"', "id=`"$k`$1`"")
  $text = [regex]::Replace($text, 'url\(#([^)]+)\)', "url(#$k`$1)")
  $text = [regex]::Replace($text, '(xlink:)?href="#([^"]+)"', "`$1href=`"#$k`$2`"")
  $name = "p{0}x{1}" -f ([int]$w), ([int]$h)
  $sizes[$name] = "${w}mm ${h}mm"
  $pages.Add("<div class='pg' style='page:$name;width:${w}mm;height:${h}mm'>" + ($text -replace '<svg', "<svg style='display:block;width:${w}mm;height:${h}mm'") + "</div>")
}
if ($pages.Count -eq 0) { Write-Output "no pages"; exit 1 }
$css = ($sizes.GetEnumerator() | ForEach-Object { "@page $($_.Key){size:$($_.Value);margin:0}" }) -join ''
$html = "<!doctype html><meta charset='utf-8'><style>$css html,body{margin:0;padding:0}.pg{overflow:hidden;break-after:page}.pg:last-child{break-after:auto}</style>" + ($pages -join '')
$tmp = Join-Path $env:TEMP ("pla-book-" + [guid]::NewGuid().ToString('N').Substring(0, 8))
New-Item -ItemType Directory -Force $tmp | Out-Null
[IO.File]::WriteAllText("$tmp\book.html", $html, (New-Object Text.UTF8Encoding $false))
if (Test-Path $Out) { Remove-Item $Out -Force }
$b = Start-Process $browser -PassThru -WindowStyle Hidden -ArgumentList '--headless=new', '--disable-gpu', '--no-first-run',
  '--no-default-browser-check', '--no-pdf-header-footer', "--user-data-dir=$tmp\profile", "--print-to-pdf=$Out",
  ("file:///" + ($tmp -replace '\\', '/') + "/book.html")
if (-not $b.WaitForExit(180000)) { $b.Kill(); Write-Output "timeout (browser PDF)" }
Start-Sleep -Milliseconds 300
Remove-Item $tmp -Recurse -Force -ErrorAction SilentlyContinue
if (Test-Path $Out) { Write-Output ("done: {0} page(s) -> {1}" -f $pages.Count, $Out) } else { Write-Output "PDF not written"; exit 1 }
