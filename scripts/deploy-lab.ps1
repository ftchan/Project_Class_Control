<#
.SYNOPSIS
  批量部署：从教师机把学生端推送并安装到整个机房的多台教学机。
  需要：各教学机开启“文件和打印机共享”(admin$)、当前账号对目标机有管理员权限，
  且已安装 PsExec（Sysinternals）。仅用于管理员已授权的本机房设备。

.PARAMETER Hosts
  目标主机名/IP 数组。或用 -HostsFile 指定每行一个的文本文件。

.PARAMETER HostsFile
  主机清单文件（每行一个主机名/IP）。

.PARAMETER ExePath
  本机上的 labctl-student.exe。

.PARAMETER PsExec
  PsExec.exe 路径（默认在 PATH 中）。

.PARAMETER PolicyArgs
  传给 install-student.ps1 的策略参数字符串，例如 "-LockWallpaper -Usb readonly -Uwf on"。

.EXAMPLE
  .\deploy-lab.ps1 -HostsFile .\pcs.txt -ExePath .\labctl-student.exe `
      -PolicyArgs "-LockWallpaper -Usb readonly -Uwf on -Passphrase lab-2016"
#>
[CmdletBinding()]
param(
  [string[]]$Hosts,
  [string]$HostsFile,
  [Parameter(Mandatory)] [string]$ExePath,
  [string]$PsExec = "PsExec.exe",
  [string]$PolicyArgs = "-LockWallpaper -Usb readonly -Uwf on"
)
$ErrorActionPreference = "Stop"

if ($HostsFile) { $Hosts = Get-Content $HostsFile | Where-Object { $_ -and -not $_.StartsWith('#') } }
if (-not $Hosts) { throw "请用 -Hosts 或 -HostsFile 指定目标主机。" }
if (-not (Test-Path $ExePath)) { throw "找不到 $ExePath" }

$ps1 = Join-Path $PSScriptRoot "install-student.ps1"
$ok = @(); $fail = @()

foreach ($h in $Hosts) {
  Write-Host "==> $h" -ForegroundColor Cyan
  try {
    $remoteDir = "\\$h\C$\Windows\Temp\LabCtlDeploy"
    New-Item -ItemType Directory -Force -Path $remoteDir | Out-Null
    Copy-Item $ExePath (Join-Path $remoteDir "labctl-student.exe") -Force
    Copy-Item $ps1     (Join-Path $remoteDir "install-student.ps1") -Force

    # 在目标机以 SYSTEM 执行安装脚本。
    $cmd = "powershell -NoProfile -ExecutionPolicy Bypass -File C:\Windows\Temp\LabCtlDeploy\install-student.ps1 $PolicyArgs"
    & $PsExec "\\$h" -s -accepteula -nobanner cmd /c $cmd
    if ($LASTEXITCODE -ne 0) { throw "远程安装返回 $LASTEXITCODE" }
    $ok += $h
    Write-Host "    ✔ 成功" -ForegroundColor Green
  } catch {
    $fail += $h
    Write-Host "    x 失败：$($_.Exception.Message)" -ForegroundColor Red
  }
}

Write-Host ""
Write-Host "完成：成功 $($ok.Count) 台，失败 $($fail.Count) 台。" -ForegroundColor Cyan
if ($fail.Count) { Write-Host "失败主机：$($fail -join ', ')" -ForegroundColor Yellow }
