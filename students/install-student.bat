@echo off
REM Compatibility launcher. The EXE now performs installation and requests
REM administrator privileges itself; this batch file is optional.
if "%~1"=="" (
  "%~dp0labctl-student.exe"
) else (
  "%~dp0labctl-student.exe" setup %*
)
