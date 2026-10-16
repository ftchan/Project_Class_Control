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
  throw "请以管理员身份运行此脚本。"
}

$exe = Join-Path $InstallDir "labctl-student.exe"
if (Test-Path $exe) {
  Write-Host "停止并卸载服务 ..." -ForegroundColor Cyan
  & $exe uninstall 2>$null | Out-Null
} else {
  # 退而求其次，直接用 sc 删除。
  sc.exe stop LabCtlStudent 2>$null | Out-Null
  sc.exe delete LabCtlStudent 2>$null | Out-Null
}

# 移除防火墙规则
netsh advfirewall firewall delete rule name="LabCtl Agent (TCP 47800)"     2>$null | Out-Null
netsh advfirewall firewall delete rule name="LabCtl Discovery (UDP 47801)" 2>$null | Out-Null

# 结束 GUI 客户端并移除自启项
Stop-Process -Name "labctl-student-ui" -Force -ErrorAction SilentlyContinue
Remove-ItemProperty -Path "HKLM:\Software\Microsoft\Windows\CurrentVersion\Run" `
  -Name "LabCtlStudentUI" -ErrorAction SilentlyContinue

Write-Host "删除安装目录 ..." -ForegroundColor Cyan
Remove-Item -Recurse -Force $InstallDir -ErrorAction SilentlyContinue

if (-not $KeepData) {
  $dataDir = Join-Path $env:ProgramData "LabCtl"
  Write-Host "删除数据目录 $dataDir ..." -ForegroundColor Cyan
  Remove-Item -Recurse -Force $dataDir -ErrorAction SilentlyContinue
}

Write-Host "✔ 已卸载。" -ForegroundColor Green
Write-Host "  注：壁纸/U盘等注册表策略不会随卸载自动回退；如需恢复，先在控制台下发" -ForegroundColor Yellow
Write-Host "      allow/unmanaged/解锁壁纸 的空策略，再执行卸载。" -ForegroundColor Yellow
