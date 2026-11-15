# 出一张图: .\draw.ps1 <spec.json>   (FreeCAD 离屏, 不出窗口)
param([Parameter(Mandatory = $true)][string]$Spec, [string]$FreeCAD = "$env:LOCALAPPDATA\Programs\FreeCAD 1.1\bin")
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$env:QT_QPA_PLATFORM = 'offscreen'
$env:QT_QPA_FONTDIR = "$env:WINDIR\Fonts"
$env:PLA_DRAW_SPEC = (Resolve-Path $Spec).Path
$t = Get-Date
$p = Start-Process "$FreeCAD\freecad.exe" -ArgumentList "`"$here\run_offscreen.py`"" -PassThru -WindowStyle Hidden
if (-not $p.WaitForExit(180000)) { $p.Kill(); Write-Output "timeout" }
Write-Output ("exit {0} in {1:n1}s" -f $p.ExitCode, ((Get-Date) - $t).TotalSeconds)
