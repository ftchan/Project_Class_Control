@echo off
REM One-click uninstall of the LabCtl student client. Auto-elevates to admin.
REM Removes the client and restores what it changed: UWF, USB storage,
REM firewall / internet access, wallpaper lock and Task Manager (all users).
REM If UWF is protecting the machine, reboot when asked and run this again.

net session >nul 2>&1
if %errorlevel% neq 0 (
  echo Requesting administrator privileges...
  powershell -NoProfile -Command "Start-Process -Verb RunAs -FilePath '%~f0'"
  exit /b
)

powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0uninstall-student.ps1"

echo.
pause
