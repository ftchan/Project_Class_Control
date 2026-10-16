<#
.SYNOPSIS
  一键部署 LabCtl 学生端（单文件）。需以管理员身份运行。
  仅用于学校内网、管理员已授权的本机房自有设备。

.DESCRIPTION
  单个 labctl-student.exe 同时提供后台服务与托盘界面：
    1. 复制 labctl-student.exe 到安装目录（默认 C:\Program Files\LabCtl）
    2. 写入初始策略 C:\ProgramData\LabCtl\policy.json（按参数）
    3. （可选）安装 UWF 可选功能 Client-UnifiedWriteFilter（专业版即可）
    4. 注册并启动 Windows 服务（含失败自动重启）
    5. 把界面（exe ui）加入开机自启，并放行防火墙

.PARAMETER ExePath
  源 labctl-student.exe 的路径。默认取脚本同目录下的该文件。

.PARAMETER InstallDir
  安装目录。默认 "C:\Program Files\LabCtl"。

.PARAMETER LockWallpaper
  锁定桌面壁纸。

.PARAMETER WallpaperPath
  强制使用的壁纸绝对路径（可选）。

.PARAMETER Usb
  U 盘策略：allow | readonly | deny（默认 deny）。

.PARAMETER Uwf
  UWF 期望状态：on | off | unmanaged（默认 unmanaged）。on 会先安装可选功能。

.PARAMETER Passphrase
  客户端访问口令（可选）。

.EXAMPLE
  .\install-student.ps1 -Usb deny -Uwf on -Passphrase "lab-2016"
#>
[CmdletBinding()]
param(
  [string]$ExePath,
  [string]$InstallDir = "C:\Program Files\LabCtl",
  [switch]$LockWallpaper,
  [string]$WallpaperPath,
  [ValidateSet("allow","readonly","deny")] [string]$Usb = "deny",
  [ValidateSet("on","off","unmanaged")]    [string]$Uwf = "unmanaged",
  [string]$Passphrase
)

$ErrorActionPreference = "Stop"

function Assert-Admin {
  $id = [Security.Principal.WindowsIdentity]::GetCurrent()
  $p  = New-Object Security.Principal.WindowsPrincipal($id)
  if (-not $p.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    throw "please run this script as Administrator."
  }
}

Assert-Admin

# --- 1. locate and copy the executable ---
if (-not $ExePath) {
  $ExePath = Join-Path $PSScriptRoot "labctl-student.exe"
}
if (-not (Test-Path $ExePath)) {
  throw "labctl-student.exe not found: $ExePath (use -ExePath, or put it next to this script)"
}

Write-Host "[1/5] install to $InstallDir ..." -ForegroundColor Cyan
New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
$targetExe = Join-Path $InstallDir "labctl-student.exe"

# Pre-clean a previous install before overwriting the exe: the running service
# and tray UI keep it locked. Ignore output/errors (merge stderr so a native
# stderr write can't abort the script under ErrorActionPreference=Stop).
if (Test-Path $targetExe) {
  try { & $targetExe uninstall 2>&1 | Out-Null } catch { }
  $global:LASTEXITCODE = 0
  Get-Process -Name "labctl-student" -ErrorAction SilentlyContinue |
    Where-Object { $_.Path -eq $targetExe } |
    Stop-Process -Force -ErrorAction SilentlyContinue
  for ($i = 0; $i -lt 20 -and (Get-Process -Name "labctl-student" -ErrorAction SilentlyContinue |
         Where-Object { $_.Path -eq $targetExe }); $i++) { Start-Sleep -Milliseconds 500 }
}
Copy-Item -Path $ExePath -Destination $targetExe -Force

# --- 2. write the initial policy ---
Write-Host "[2/5] write initial policy ..." -ForegroundColor Cyan
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
  $hash = (& $targetExe hash $Passphrase 2>&1 | Out-String).Trim()
  if ($LASTEXITCODE -ne 0) { throw "hashing passphrase failed: $hash" }
  $policy.config_passphrase_hash = $hash
}

$policyPath = Join-Path $dataDir "policy.json"
# UTF-8 without BOM: PowerShell 5.1's -Encoding UTF8 adds a BOM that the
# agent's JSON parser rejects (it would then fall back to the strict baseline).
[IO.File]::WriteAllText($policyPath, ($policy | ConvertTo-Json -Depth 4), (New-Object Text.UTF8Encoding($false)))
Write-Host "      wrote $policyPath"

# --- 3. UWF optional feature (only when enabling) ---
if ($Uwf -eq "on") {
  Write-Host "[3/5] ensure UWF feature (Client-UnifiedWriteFilter) ..." -ForegroundColor Cyan
  $feat = Get-WindowsOptionalFeature -Online -FeatureName Client-UnifiedWriteFilter -ErrorAction SilentlyContinue
  if ($feat -and $feat.State -ne "Enabled") {
    dism.exe /Online /Enable-Feature /FeatureName:Client-UnifiedWriteFilter /All /NoRestart | Out-Null
    Write-Host "      UWF feature installed (reboot required to enable)." -ForegroundColor Yellow
  } else {
    Write-Host "      UWF feature ready."
  }
} else {
  Write-Host "[3/5] skip UWF feature (-Uwf $Uwf)." -ForegroundColor DarkGray
}

# --- 4. register and start the service ---
Write-Host "[4/5] register and start the Windows service ..." -ForegroundColor Cyan

$installOut = & $targetExe install 2>&1 | Out-String
if ($LASTEXITCODE -ne 0) { throw "service install failed (exit $LASTEXITCODE): $installOut" }
Write-Host ("      " + $installOut.Trim())

# --- 5. GUI autostart + firewall ---
Write-Host "[5/5] configure autostart and firewall ..." -ForegroundColor Cyan
$runKey = "HKLM:\Software\Microsoft\Windows\CurrentVersion\Run"
# Only create the key when missing: New-Item -Force on an existing key
# recreates it and wipes every other autostart entry.
if (-not (Test-Path $runKey)) { New-Item -Path $runKey | Out-Null }
Set-ItemProperty -Path $runKey -Name "LabCtlStudentUI" -Value "`"$targetExe`" ui"

netsh advfirewall firewall delete rule name="LabCtl Agent (TCP 47800)"      2>$null | Out-Null
netsh advfirewall firewall delete rule name="LabCtl Discovery (UDP 47801)"  2>$null | Out-Null
netsh advfirewall firewall add rule name="LabCtl Agent (TCP 47800)"     dir=in action=allow protocol=TCP localport=47800 profile=any | Out-Null
netsh advfirewall firewall add rule name="LabCtl Discovery (UDP 47801)" dir=in action=allow protocol=UDP localport=47801 profile=any | Out-Null

# launch the GUI now (hidden in tray)
Start-Process -FilePath $targetExe -ArgumentList "ui" -ErrorAction SilentlyContinue

Start-Sleep -Seconds 2
$svc = Get-Service -Name LabCtlStudent -ErrorAction SilentlyContinue
if (-not $svc -or $svc.Status -ne 'Running') {
  Write-Host "WARN: service not Running (current: $($svc.Status)). Run '$targetExe doctor' to diagnose." -ForegroundColor Yellow
}

Write-Host ""
Write-Host "OK: deployment complete." -ForegroundColor Green
Write-Host "  exe     : $targetExe"
Write-Host "  service : $((Get-Service LabCtlStudent -ErrorAction SilentlyContinue).Status)"
Write-Host "  policy  : $policyPath"
if ($Uwf -eq "on") { Write-Host "  note    : UWF takes effect after a reboot." -ForegroundColor Yellow }
