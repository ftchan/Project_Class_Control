<#
.SYNOPSIS
  卸载 LabCtl 学生端。需以管理员身份运行。
.PARAMETER InstallDir
  安装目录，默认 "C:\Program Files\LabCtl"。
.PARAMETER KeepData
  保留 C:\ProgramData\LabCtl（策略、机器标识、日志）。默认一并删除。
#>
[CmdletBinding()]
param(
  [string]$InstallDir = "C:\Program Files\LabCtl",
  [switch]$KeepData
)
$ErrorActionPreference = "Stop"

$id = [Security.Principal.WindowsIdentity]::GetCurrent()
if (-not (New-Object Security.Principal.WindowsPrincipal($id)).IsInRole(
      [Security.Principal.WindowsBuiltInRole]::Administrator)) {
  throw "please run this script as Administrator."
}

$exe = Join-Path $InstallDir "labctl-student.exe"
if (Test-Path $exe) {
  Write-Host "stop and remove the service ..." -ForegroundColor Cyan
  & $exe uninstall 2>$null | Out-Null
} else {
  sc.exe stop LabCtlStudent 2>$null | Out-Null
  sc.exe delete LabCtlStudent 2>$null | Out-Null
}

# remove firewall rules and autostart, stop the GUI
netsh advfirewall firewall delete rule name="LabCtl Agent (TCP 47800)"     2>$null | Out-Null
netsh advfirewall firewall delete rule name="LabCtl Discovery (UDP 47801)" 2>$null | Out-Null
Remove-ItemProperty -Path "HKLM:\Software\Microsoft\Windows\CurrentVersion\Run" `
  -Name "LabCtlStudentUI" -ErrorAction SilentlyContinue
Stop-Process -Name "labctl-student" -Force -ErrorAction SilentlyContinue

Write-Host "remove install directory ..." -ForegroundColor Cyan
Remove-Item -Recurse -Force $InstallDir -ErrorAction SilentlyContinue

if (-not $KeepData) {
  $dataDir = Join-Path $env:ProgramData "LabCtl"
  Write-Host "remove data directory $dataDir ..." -ForegroundColor Cyan
  Remove-Item -Recurse -Force $dataDir -ErrorAction SilentlyContinue
}

Write-Host "OK: uninstalled." -ForegroundColor Green
Write-Host "note: wallpaper/USB registry policies are not auto-reverted; push an" -ForegroundColor Yellow
Write-Host "      empty policy (allow/unmanaged/unlock) from the console first if needed." -ForegroundColor Yellow
