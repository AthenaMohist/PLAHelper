# 上传的图纸 → 每页一张 PNG(预览 + 合订本用): .\upload2png.ps1 -In <a.pdf|a.png|a.jpg> -Out <前缀> [-Dpi 150]
# 输出 <前缀>-1.png, <前缀>-2.png …; 最后一行打印 JSON: {"pages":[[宽mm,高mm],…]}
# PDF 用 Windows 自带的 Windows.Data.Pdf(WinRT), 图片用 System.Drawing —— 不装任何东西。
param([Parameter(Mandatory = $true)][string]$In, [Parameter(Mandatory = $true)][string]$Out, [int]$Dpi = 150)
$ErrorActionPreference = 'Stop'
$full = [IO.Path]::GetFullPath($In)
$ext = [IO.Path]::GetExtension($full).ToLowerInvariant()
$pages = @()

if ($ext -ne '.pdf') {
  # 图片(扫描件 / 截图): 转成 PNG; 图片的 DPI 常常不可信 → 纸张一律按长边 420(A3)等比
  Add-Type -AssemblyName System.Drawing
  $img = [System.Drawing.Image]::FromFile($full)
  $img.Save("$Out-1.png", [System.Drawing.Imaging.ImageFormat]::Png)
  $k = 420.0 / [math]::Max($img.Width, $img.Height)
  $pages += , @([math]::Round($img.Width * $k, 1), [math]::Round($img.Height * $k, 1))
  $img.Dispose()
  ConvertTo-Json -Compress @{ pages = $pages } -Depth 4
  exit 0
}

Add-Type -AssemblyName System.Runtime.WindowsRuntime
$null = [Windows.Storage.StorageFile, Windows.Storage, ContentType = WindowsRuntime]
$null = [Windows.Data.Pdf.PdfDocument, Windows.Data.Pdf, ContentType = WindowsRuntime]
$null = [Windows.Storage.Streams.InMemoryRandomAccessStream, Windows.Storage.Streams, ContentType = WindowsRuntime]
$asTask = [System.WindowsRuntimeSystemExtensions].GetMethods() | Where-Object {
  $_.Name -eq 'AsTask' -and $_.GetParameters().Count -eq 1 -and $_.GetParameters()[0].ParameterType.Name -eq 'IAsyncOperation`1' } | Select-Object -First 1
$asTaskAction = [System.WindowsRuntimeSystemExtensions].GetMethods() | Where-Object {
  $_.Name -eq 'AsTask' -and $_.GetParameters().Count -eq 1 -and $_.GetParameters()[0].ParameterType.Name -eq 'IAsyncAction' } | Select-Object -First 1
function Await($op, [type]$t) { $task = $asTask.MakeGenericMethod($t).Invoke($null, @($op)); $task.Wait(); $task.Result }
function AwaitAction($op) { $task = $asTaskAction.Invoke($null, @($op)); $task.Wait() }

$file = Await ([Windows.Storage.StorageFile]::GetFileFromPathAsync($full)) ([Windows.Storage.StorageFile])
$doc = Await ([Windows.Data.Pdf.PdfDocument]::LoadFromFileAsync($file)) ([Windows.Data.Pdf.PdfDocument])
for ($i = 0; $i -lt $doc.PageCount; $i++) {
  $page = $doc.GetPage($i)
  $sz = $page.Size  # DIP(1/96 英寸)
  $opt = New-Object Windows.Data.Pdf.PdfPageRenderOptions
  $opt.DestinationWidth = [uint32]([math]::Round($sz.Width * $Dpi / 96))
  $opt.DestinationHeight = [uint32]([math]::Round($sz.Height * $Dpi / 96))
  $opt.BackgroundColor = [Windows.UI.Color]::FromArgb(255, 255, 255, 255)
  $stream = New-Object Windows.Storage.Streams.InMemoryRandomAccessStream
  AwaitAction ($page.RenderToStreamAsync($stream, $opt))
  $net = [System.IO.WindowsRuntimeStreamExtensions]::AsStreamForRead($stream.GetInputStreamAt(0))
  $fs = [IO.File]::Create("$Out-$($i + 1).png")
  $net.CopyTo($fs)
  $fs.Close(); $net.Close(); $stream.Dispose(); $page.Dispose()
  $pages += , @([math]::Round($sz.Width * 25.4 / 96, 1), [math]::Round($sz.Height * 25.4 / 96, 1))
}
ConvertTo-Json -Compress @{ pages = $pages } -Depth 4
