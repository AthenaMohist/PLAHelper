@echo off
setlocal EnableExtensions
title PLA-CDP 补丁工具
rem ------------------------------------------------------------------
rem  PLA-CDP 补丁工具 v6
rem  作用: 给 Plasticity 主进程入口打补丁, 启动时开放 CDP 调试端口 9333,
rem        让 PLAhelper 能连上(选择集/视口状态/实体树等功能依赖它)。
rem  用法: 双击运行即可(会自动找 Plasticity 安装位置);
rem        也可以把 index.js 直接拖到本文件上。
rem  权限: 只有当 Plasticity 装在需要管理员权限的目录(如 C:\Program Files)
rem        时才会申请提权(UAC); 装在用户目录或 E:\Program Files 之类位置时
rem        不会有任何权限弹窗。
rem  备份: 同目录生成 index.js.pla-backup(已存在则不覆盖), 可随时还原。
rem ------------------------------------------------------------------

set "PATCH_JS=%~dp0pla-cdp-patch.js"
set "TARGET=%~1"

echo ============================================================
echo   PLA-CDP 补丁工具  --  为 Plasticity 开放 CDP 端口 9333
echo ============================================================
echo.

if not exist "%PATCH_JS%" (
    echo [错误] 同目录下找不到补丁文件 pla-cdp-patch.js
    echo        请把 pla-cdp-patch.js 和本 bat 放在一起再运行。
    echo.
    pause
    exit /b 1
)

if defined TARGET goto :check_target

echo [1/4] 正在查找 Plasticity 安装目录 ...
call :locate
if not defined TARGET goto :not_found

:check_target
if not exist "%TARGET%" (
    echo [错误] 目标文件不存在: %TARGET%
    pause
    exit /b 1
)
echo [1/4] 目标文件:
echo       %TARGET%
echo.

rem ---- 是否已打过补丁 ----
findstr /C:"pla-cdp-patch" "%TARGET%" >nul 2>&1
if not errorlevel 1 (
    echo [提示] 该文件已经打过补丁, 无需重复操作。
    echo.
    pause
    exit /b 0
)

rem ---- 写权限探测(不触发 UAC; 探测不到写权限才提权) ----
copy /y "%TARGET%" "%TARGET%.pla-wtest" >nul 2>&1
if errorlevel 1 goto :need_admin
del /q "%TARGET%.pla-wtest" >nul 2>&1

echo [2/4] 备份原文件 ...
set "BAK=%TARGET%.pla-backup"
if exist "%BAK%" (
    echo       已存在备份, 保留不动: %BAK%
) else (
    copy /y "%TARGET%" "%BAK%" >nul
    if errorlevel 1 (
        echo [错误] 备份失败, 已中止(未改动原文件)。
        pause
        exit /b 1
    )
    echo       备份到: %BAK%
)
echo.

echo [3/4] 写入补丁 ...
set "TMPF=%TARGET%.pla-tmp"
if exist "%TMPF%" del /q "%TMPF%" >nul 2>&1
copy /b "%PATCH_JS%" + "%BAK%" "%TMPF%" >nul
if errorlevel 1 goto :need_admin
move /y "%TMPF%" "%TARGET%" >nul
if errorlevel 1 goto :need_admin
echo       完成。
echo.

echo [4/4] 校验 ...
findstr /C:"pla-cdp-patch" "%TARGET%" >nul 2>&1
if errorlevel 1 (
    echo [错误] 校验失败, 正在从备份还原 ...
    copy /y "%BAK%" "%TARGET%" >nul
    pause
    exit /b 1
)
echo       补丁已生效。
echo.
echo ------------------------------------------------------------
echo   完成! 请完全退出并重新启动 Plasticity。
echo   启动后可用浏览器打开 http://127.0.0.1:9333/json/version
echo   能看到 JSON 即表示 CDP 端口已开放, PLAhelper 可正常连接。
echo   日志: %USERPROFILE%\.pla-cdp-patch.log
echo   还原: 把 %BAK% 复制回 index.js 即可。
echo ------------------------------------------------------------
echo.
tasklist /FI "IMAGENAME eq Plasticity.exe" 2>nul | findstr /I "Plasticity.exe" >nul
if not errorlevel 1 (
    echo [注意] 检测到 Plasticity 正在运行, 补丁需重启后才生效。
    echo.
)
pause
exit /b 0

rem ====================== 子过程 ======================
:locate
set "TARGET="
for %%R in ("%LOCALAPPDATA%\Programs\Plasticity" "%ProgramFiles%\Plasticity" "%ProgramFiles(x86)%\Plasticity" "C:\Program Files\Plasticity" "D:\Program Files\Plasticity" "E:\Program Files\Plasticity" "F:\Program Files\Plasticity" "G:\Program Files\Plasticity" "C:\Plasticity" "D:\Plasticity" "E:\Plasticity") do (
    if exist "%%~R" call :scan_root "%%~R"
)
goto :eof

:scan_root
rem 布局 A: <root>\resources\app\.webpack\main\index.js
if exist "%~1\resources\app\.webpack\main\index.js" (
    set "TARGET=%~1\resources\app\.webpack\main\index.js"
    goto :eof
)
rem 布局 B: <root>\app-26.1.3\resources\app\.webpack\main\index.js (取版本号最大的)
set "BESTVER="
set "BESTDIR="
for /d %%D in ("%~1\app-*") do (
    if exist "%%~D\resources\app\.webpack\main\index.js" (
        set "CUR=%%~nxD"
        set "CURDIR=%%~D"
        call :pick_best
    )
)
if defined BESTDIR set "TARGET=%BESTDIR%\resources\app\.webpack\main\index.js"
goto :eof

:pick_best
if not defined BESTVER (
    set "BESTVER=%CUR%"
    set "BESTDIR=%CURDIR%"
    goto :eof
)
if "%CUR%" GTR "%BESTVER%" (
    set "BESTVER=%CUR%"
    set "BESTDIR=%CURDIR%"
)
goto :eof

:need_admin
echo.
echo [提示] 目标目录需要管理员权限才能写入:
echo       %TARGET%
echo       正在申请提权, 请在弹出的窗口里点"是" ...
echo       (若 Plasticity 装在用户目录则不会出现这个提示)
powershell -NoProfile -Command "Start-Process -FilePath '%~f0' -ArgumentList '\"%TARGET%\"' -Verb RunAs"
echo.
pause
exit /b 2

:not_found
echo [错误] 没找到 Plasticity 的 index.js。
echo.
echo   可以把这个文件手动拖到本 bat 上:
echo     <Plasticity安装目录>\resources\app\.webpack\main\index.js
echo   常见位置:
echo     %%LOCALAPPDATA%%\Programs\Plasticity\app-*\resources\app\.webpack\main\index.js
echo     C:\Program Files\Plasticity\app-*\resources\app\.webpack\main\index.js
echo.
pause
exit /b 3
