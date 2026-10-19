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

.PARAMETER SetupArgs
  传给学生端 EXE 的策略参数，例如 "--lock-wallpaper --usb readonly --uwf on"。

.EXAMPLE
  .\deploy-lab.ps1 -HostsFile .\pcs.txt -ExePath .\labctl-student.exe `
      -SetupArgs "--lock-wallpaper --usb readonly --uwf on --passphrase lab-2016"
#>
[CmdletBinding()]
param(
  [string[]]$Hosts,
  [string]$HostsFile,
  [Parameter(Mandatory)] [string]$ExePath,
  [string]$PsExec = "PsExec.exe",
  [Alias('PolicyArgs')] [string]$SetupArgs = "--lock-wallpaper --usb readonly --uwf on"
)
$ErrorActionPreference = "Stop"

if ($HostsFile) { $Hosts = Get-Content $HostsFile | Where-Object { $_ -and -not $_.StartsWith('#') } }
if (-not $Hosts) { throw "请用 -Hosts 或 -HostsFile 指定目标主机。" }
if (-not (Test-Path $ExePath)) { throw "找不到 $ExePath" }

# Accept the old -PolicyArgs spelling used by earlier deployments.
$SetupArgs = $SetupArgs -replace '(?i)(?<!-)-LockWallpaper\b', '--lock-wallpaper' `
                        -replace '(?i)(?<!-)-InstallDir\b', '--install-dir' `
                        -replace '(?i)(?<!-)-WallpaperPath\b', '--wallpaper-path' `
                        -replace '(?i)(?<!-)-Passphrase\b', '--passphrase' `
                        -replace '(?i)(?<!-)-Usb\b', '--usb' `
                        -replace '(?i)(?<!-)-Uwf\b', '--uwf'
$ok = @(); $fail = @()

foreach ($h in $Hosts) {
  Write-Host "==> $h" -ForegroundColor Cyan
  try {
    $remoteDir = "\\$h\C$\Windows\Temp\LabCtlDeploy"
    New-Item -ItemType Directory -Force -Path $remoteDir | Out-Null
    Copy-Item $ExePath (Join-Path $remoteDir "labctl-student.exe") -Force
    # 在目标机以 SYSTEM 执行 EXE 自带安装流程。
    $cmd = "C:\Windows\Temp\LabCtlDeploy\labctl-student.exe setup --quiet $SetupArgs"
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
