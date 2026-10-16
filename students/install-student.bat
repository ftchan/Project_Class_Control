@echo off
REM One-click deploy of the LabCtl student client. Auto-elevates to admin.
REM Put labctl-student.exe next to this .bat and install-student.ps1.
REM Edit the default policy on the line below (see install-student.ps1 header).

set "POLICY=-Usb deny -Uwf on"
REM with a passphrase:  set "POLICY=-Usb deny -Uwf on -Passphrase lab-2016"

net session >nul 2>&1
if %errorlevel% neq 0 (
  echo Requesting administrator privileges...
  powershell -NoProfile -Command "Start-Process -Verb RunAs -FilePath '%~f0'"
  exit /b
)

powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0install-student.ps1" %POLICY%

echo.
pause
