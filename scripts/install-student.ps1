<#
.SYNOPSIS
  一键部署 LabCtl 学生端（受管客户端）。需以管理员身份运行。

  仅用于学校内网、管理员已授权的本机房自有设备。

.DESCRIPTION
  执行以下步骤：
    1. 将 labctl-student.exe 复制到安装目录（默认 C:\Program Files\LabCtl）
    2. 写入初始策略 C:\ProgramData\LabCtl\policy.json（按参数）
    3. （可选）安装 UWF 可选功能 Client-UnifiedWriteFilter（专业版即可）
    4. 注册并启动 Windows 服务（含失败自动重启）

.PARAMETER ExePath
  源 labctl-student.exe 的路径。默认取脚本同目录下的该文件。

.PARAMETER InstallDir
  安装目录。默认 "C:\Program Files\LabCtl"。

.PARAMETER LockWallpaper
  锁定桌面壁纸。

.PARAMETER WallpaperPath
  强制使用的壁纸绝对路径（可选）。

.PARAMETER Usb
  U 盘策略：allow | readonly | deny（默认 allow）。

.PARAMETER Uwf
  UWF 期望状态：on | off | unmanaged（默认 unmanaged）。on 会先安装可选功能。

.PARAMETER Passphrase
  客户端访问口令（可选）。脚本用 `labctl-student hash` 计算哈希后写入策略。

.EXAMPLE
  # 最常见：锁壁纸 + U盘只读 + 开 UWF + 设口令
  .\install-student.ps1 -LockWallpaper -Usb readonly -Uwf on -Passphrase "lab-2016"
#>
[CmdletBinding()]
param(
  [string]$ExePath,
  [string]$InstallDir = "C:\Program Files\LabCtl",
  [switch]$LockWallpaper,
  [string]$WallpaperPath,
  [ValidateSet("allow","readonly","deny")] [string]$Usb = "allow",
  [ValidateSet("on","off","unmanaged")]    [string]$Uwf = "unmanaged",
  [string]$Passphrase
)

$ErrorActionPreference = "Stop"

function Assert-Admin {
  $id = [Security.Principal.WindowsIdentity]::GetCurrent()
  $p  = New-Object Security.Principal.WindowsPrincipal($id)
  if (-not $p.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    throw "请以管理员身份运行此脚本。"
  }
}

Assert-Admin

# --- 1. 定位并复制可执行文件 ---
if (-not $ExePath) {
  $ExePath = Join-Path $PSScriptRoot "labctl-student.exe"
}
if (-not (Test-Path $ExePath)) {
  throw "找不到 labctl-student.exe：$ExePath （用 -ExePath 指定，或放到脚本同目录）"
}

Write-Host "[1/4] 安装到 $InstallDir ..." -ForegroundColor Cyan
New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
$targetExe = Join-Path $InstallDir "labctl-student.exe"
Copy-Item -Path $ExePath -Destination $targetExe -Force

# GUI 客户端（可视化托盘程序）——若与学生端 exe 同目录则一并安装并设开机自启。
$uiSrc = Join-Path (Split-Path $ExePath -Parent) "labctl-student-ui.exe"
$uiExe = Join-Path $InstallDir "labctl-student-ui.exe"
if (Test-Path $uiSrc) {
  Copy-Item -Path $uiSrc -Destination $uiExe -Force
  # 开机自启（HKLM Run：对本机所有登录用户生效）
  $runKey = "HKLM:\Software\Microsoft\Windows\CurrentVersion\Run"
  New-Item -Path $runKey -Force | Out-Null
  Set-ItemProperty -Path $runKey -Name "LabCtlStudentUI" -Value "`"$uiExe`""
  Write-Host "      已安装 GUI 客户端并设为开机自启。"
} else {
  Write-Host "      （未找到 labctl-student-ui.exe，跳过 GUI 客户端安装）" -ForegroundColor DarkGray
}

# --- 2. 写入初始策略 ---
Write-Host "[2/4] 写入初始策略 ..." -ForegroundColor Cyan
$dataDir = Join-Path $env:ProgramData "LabCtl"
New-Item -ItemType Directory -Force -Path $dataDir | Out-Null

$usbMap = @{ allow = "allow"; readonly = "read_only"; deny = "deny" }
$uwfVal = switch ($Uwf) { "on" { $true } "off" { $false } default { $null } }

$policy = [ordered]@{
  lock_wallpaper          = [bool]$LockWallpaper
  wallpaper_path          = if ($WallpaperPath) { $WallpaperPath } else { $null }
  usb_storage             = $usbMap[$Usb]
  uwf_enabled             = $uwfVal
  config_passphrase_hash  = $null
  keep_service_alive      = $true
  revision                = [int64]([DateTimeOffset]::UtcNow.ToUnixTimeSeconds())
}

if ($Passphrase) {
  Write-Host "      计算访问口令哈希 ..." -ForegroundColor DarkGray
  $hash = & $targetExe hash $Passphrase
  $policy.config_passphrase_hash = $hash.Trim()
}

$policyPath = Join-Path $dataDir "policy.json"
($policy | ConvertTo-Json -Depth 4) | Set-Content -Path $policyPath -Encoding UTF8
Write-Host "      已写入 $policyPath"

# --- 3. UWF 可选功能（仅在需要开启时）---
if ($Uwf -eq "on") {
  Write-Host "[3/4] 确保 UWF 功能已安装（Client-UnifiedWriteFilter）..." -ForegroundColor Cyan
  $feat = Get-WindowsOptionalFeature -Online -FeatureName Client-UnifiedWriteFilter -ErrorAction SilentlyContinue
  if ($feat -and $feat.State -ne "Enabled") {
    dism.exe /Online /Enable-Feature /FeatureName:Client-UnifiedWriteFilter /All /NoRestart | Out-Null
    Write-Host "      已安装 UWF 功能（需重启后 UWF 方可启用）。" -ForegroundColor Yellow
  } else {
    Write-Host "      UWF 功能已就绪。"
  }
} else {
  Write-Host "[3/4] 跳过 UWF 功能安装（-Uwf $Uwf）。" -ForegroundColor DarkGray
}

# --- 4. 注册并启动服务 ---
Write-Host "[4/4] 注册并启动 Windows 服务 ..." -ForegroundColor Cyan
# 若已存在旧服务，先卸载再装，保证干净。
& $targetExe uninstall 2>$null | Out-Null
& $targetExe install
if ($LASTEXITCODE -ne 0) { throw "服务安装失败（exit $LASTEXITCODE）。" }

# 启动 GUI 客户端（best-effort；下次登录会自动启动）
if (Test-Path $uiExe) {
  Start-Process -FilePath $uiExe -ErrorAction SilentlyContinue
}

Write-Host ""
Write-Host "✔ 部署完成。" -ForegroundColor Green
Write-Host "  可执行文件 : $targetExe"
if (Test-Path $uiExe) { Write-Host "  GUI 客户端 : $uiExe（托盘常驻，开机自启）" }
Write-Host "  策略文件   : $policyPath"
Write-Host "  服务名     : LabCtlStudent（services.msc 可见，可正常停止/卸载）"
if ($Uwf -eq "on") { Write-Host "  提示       : UWF 需重启后生效。" -ForegroundColor Yellow }
