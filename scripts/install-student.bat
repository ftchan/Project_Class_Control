@echo off
REM 双击即可一键部署 LabCtl 学生端。会自动提升到管理员权限。
REM 把 labctl-student.exe 与本 .bat、install-student.ps1 放在同一目录。
REM 机房默认策略在下面这行修改（参数说明见 install-student.ps1 顶部）。

set "POLICY=-LockWallpaper -Usb readonly -Uwf on"
REM 需要口令就改成：  set "POLICY=-LockWallpaper -Usb readonly -Uwf on -Passphrase lab-2016"

REM —— 若未提升，则以管理员身份重新启动自己 ——
net session >nul 2>&1
if %errorlevel% neq 0 (
  echo 正在请求管理员权限...
  powershell -NoProfile -Command "Start-Process -Verb RunAs -FilePath '%~f0'"
  exit /b
)

REM —— 已是管理员：执行部署 ——
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0install-student.ps1" %POLICY%

echo.
pause
