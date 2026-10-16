<#
.SYNOPSIS
  一键卸载 LabCtl 学生端，并恢复它改动过的系统功能。需以管理员身份运行。

.DESCRIPTION
  1. UWF：若正在保护系统盘，此时的改动重启后都会被还原，因此先关闭 UWF 并提示
     重启后再运行一次；否则关闭筛选器并移除 UWF 可选功能（重启后完成）。
  2. 停止并移除服务、托盘界面、开机自启（本机所有用户）。
  3. 删除 LabCtl 防火墙规则；若用过"禁止联网"，恢复 Windows 默认的出站放行。
  4. 恢复 U 盘：允许大容量存储、取消只读。
  5. 解除壁纸锁定与任务管理器禁用（机器级策略 + 每个用户，包括未登录的账户）。
  6. 删除程序目录、数据目录与各用户的界面日志。

.PARAMETER InstallDir
  安装目录，默认 "C:\Program Files\LabCtl"。
.PARAMETER KeepData
  保留 C:\ProgramData\LabCtl（策略、机器标识、日志）。
.PARAMETER KeepUwfFeature
  不移除 UWF 可选功能（只关闭筛选器）。
.PARAMETER RestoreOnly
  只恢复系统设置，不移除程序文件与服务（供 Inno 安装包的卸载程序调用）。
#>
[CmdletBinding()]
param(
  [string]$InstallDir = "C:\Program Files\LabCtl",
  [switch]$KeepData,
  [switch]$KeepUwfFeature,
  [switch]$RestoreOnly
)
$ErrorActionPreference = "Stop"

$id = [Security.Principal.WindowsIdentity]::GetCurrent()
if (-not (New-Object Security.Principal.WindowsPrincipal($id)).IsInRole(
      [Security.Principal.WindowsBuiltInRole]::Administrator)) {
  throw "please run this script as Administrator."
}

function Step($msg) { Write-Host $msg -ForegroundColor Cyan }

# Run a native tool quietly and return its exit code. PowerShell 5.1 turns a
# native tool's stderr into a terminating error under ErrorActionPreference
# Stop, which would abort the whole uninstall over a harmless warning.
function Invoke-Native([string]$exe, [string[]]$arguments) {
  $old = $ErrorActionPreference
  $ErrorActionPreference = "Continue"
  try { & $exe @arguments *> $null } finally { $ErrorActionPreference = $old }
  $code = $LASTEXITCODE
  $global:LASTEXITCODE = 0
  return $code
}
$dataDir = Join-Path $env:ProgramData "LabCtl"
$needsReboot = $false

# --- 1. UWF ---
Step "[1/6] UWF ..."
$filter = $null
try { $filter = Get-CimInstance -Namespace root/standardcimv2/embedded -ClassName UWF_Filter -ErrorAction Stop } catch { }
if ($filter -and $filter.CurrentEnabled) {
  # Everything written now would be discarded at the next reboot.
  Invoke-CimMethod -InputObject $filter -MethodName Disable | Out-Null
  Write-Host ""
  Write-Host "UWF is protecting this machine right now, so changes made now would be" -ForegroundColor Yellow
  Write-Host "discarded at reboot. UWF has been turned OFF for the next boot." -ForegroundColor Yellow
  Write-Host "Please REBOOT, then run this uninstaller again to finish." -ForegroundColor Yellow
  exit 2
}
if ($filter -and $filter.NextEnabled) {
  Invoke-CimMethod -InputObject $filter -MethodName Disable | Out-Null
  Write-Host "      UWF filter turned off."
}
if (-not $KeepUwfFeature) {
  $feat = Get-WindowsOptionalFeature -Online -FeatureName Client-UnifiedWriteFilter -ErrorAction SilentlyContinue
  if ($feat -and $feat.State -ne "Disabled") {
    $code = Invoke-Native dism.exe @("/Online", "/Disable-Feature", "/FeatureName:Client-UnifiedWriteFilter", "/NoRestart")
    if ($code -notin 0, 3010) { Write-Host "      WARN: removing the UWF feature failed (dism $code)." -ForegroundColor Yellow }
    else { Write-Host "      UWF feature removed (completes after a reboot)."; $needsReboot = $true }
  }
}

# --- 2. service, tray UI, autostart ---
if (-not $RestoreOnly) {
  Step "[2/6] stop and remove the service and tray UI ..."
  $exe = Join-Path $InstallDir "labctl-student.exe"
  if (Test-Path $exe) {
    try { & $exe uninstall 2>&1 | Out-Null } catch { }
    $global:LASTEXITCODE = 0
  } else {
    Invoke-Native sc.exe @("stop", "LabCtlStudent") | Out-Null
    Invoke-Native sc.exe @("delete", "LabCtlStudent") | Out-Null
  }
  Stop-Process -Name "labctl-student" -Force -ErrorAction SilentlyContinue
  # Wait for the service/UI processes to exit so the exe is no longer locked.
  for ($i = 0; $i -lt 20 -and (Get-Process -Name "labctl-student" -ErrorAction SilentlyContinue); $i++) {
    Start-Sleep -Milliseconds 500
  }
  Remove-ItemProperty -Path "HKLM:\Software\Microsoft\Windows\CurrentVersion\Run" `
    -Name "LabCtlStudentUI" -ErrorAction SilentlyContinue
} else {
  Step "[2/6] (restore only: service left to the installer)"
}

# --- 3. firewall ---
Step "[3/6] firewall ..."
$blockedInternet = [bool](Get-NetFirewallRule -DisplayName "LabCtl Allow LAN (out)" -ErrorAction SilentlyContinue)
foreach ($rule in "LabCtl Agent (TCP 47800)", "LabCtl Discovery (UDP 47801)", "LabCtl Allow LAN (out)", "LabCtl Allow LAN (in)") {
  Invoke-Native netsh.exe @("advfirewall", "firewall", "delete", "rule", "name=$rule") | Out-Null
}
if ($blockedInternet) {
  # "Block internet" switched the default outbound action to block.
  Invoke-Native netsh.exe @("advfirewall", "set", "allprofiles", "firewallpolicy", "blockinbound,allowoutbound") | Out-Null
  Write-Host "      internet access restored (default outbound: allow)."
}

# --- 4. USB storage ---
Step "[4/6] USB storage ..."
Set-ItemProperty "HKLM:\SYSTEM\CurrentControlSet\Services\USBSTOR" -Name Start -Value 3 -Type DWord
$sdp = "HKLM:\SYSTEM\CurrentControlSet\Control\StorageDevicePolicies"
if (Test-Path $sdp) {
  Remove-ItemProperty $sdp -Name WriteProtect -ErrorAction SilentlyContinue
  if (-not (Get-Item $sdp).ValueCount -and -not (Get-Item $sdp).SubKeyCount) { Remove-Item $sdp }
}

# --- 5. wallpaper lock and Task Manager, machine-wide and for every user ---
Step "[5/6] wallpaper lock and Task Manager (all users) ..."
Remove-ItemProperty "HKLM:\Software\Microsoft\Windows\CurrentVersion\Policies\ActiveDesktop" `
  -Name NoChangingWallPaper -ErrorAction SilentlyContinue
$csp = "HKLM:\Software\Microsoft\Windows\CurrentVersion\PersonalizationCSP"
if ((Get-ItemProperty $csp -ErrorAction SilentlyContinue).DesktopImagePath -like "$dataDir*") {
  foreach ($v in "DesktopImagePath", "DesktopImageUrl", "DesktopImageStatus") {
    Remove-ItemProperty $csp -Name $v -ErrorAction SilentlyContinue
  }
}
$cspItem = Get-Item $csp -ErrorAction SilentlyContinue
if ($cspItem -and -not $cspItem.ValueCount -and -not $cspItem.SubKeyCount) { Remove-Item $csp }

# Remove LabCtl's per-user values from one user hive mounted at $root.
function Reset-UserHive($root) {
  $ad  = "$root\Software\Microsoft\Windows\CurrentVersion\Policies\ActiveDesktop"
  $sys = "$root\Software\Microsoft\Windows\CurrentVersion\Policies\System"
  Remove-ItemProperty $ad -Name NoChangingWallPaper -ErrorAction SilentlyContinue
  if ((Get-ItemProperty $sys -ErrorAction SilentlyContinue).Wallpaper -like "$dataDir*") {
    Remove-ItemProperty $sys -Name Wallpaper, WallpaperStyle -ErrorAction SilentlyContinue
  }
  Remove-ItemProperty $sys -Name DisableTaskMgr -ErrorAction SilentlyContinue
  Remove-ItemProperty "$root\Software\Microsoft\Windows\CurrentVersion\Run" -Name LabCtlStudentUI -ErrorAction SilentlyContinue
  foreach ($k in $ad, $sys) {   # drop keys that are now empty
    $item = Get-Item $k -ErrorAction SilentlyContinue
    if ($item -and -not $item.ValueCount -and -not $item.SubKeyCount) { Remove-Item $k -ErrorAction SilentlyContinue }
  }
}

$profiles = Get-CimInstance Win32_UserProfile | Where-Object { -not $_.Special -and $_.SID -like "S-1-5-21-*" }
$skipped = 0
foreach ($p in $profiles) {
  $loaded = "Registry::HKEY_USERS\$($p.SID)"
  if (Test-Path $loaded) {
    Reset-UserHive $loaded
  } else {
    # Signed-out user: mount their hive temporarily.
    $hive = Join-Path $p.LocalPath "NTUSER.DAT"
    if (Test-Path $hive) {
      $code = Invoke-Native reg.exe @("load", "HKU\LabCtlRestore", $hive)
      if ($code -eq 0) {
        Reset-UserHive "Registry::HKEY_USERS\LabCtlRestore"
        [gc]::Collect(); [gc]::WaitForPendingFinalizers()
        Invoke-Native reg.exe @("unload", "HKU\LabCtlRestore") | Out-Null
      } else {
        # Security software can block mounting hives; handled at sign-in below.
        Write-Host "      $($p.LocalPath): hive could not be loaded, will be restored at next sign-in" -ForegroundColor DarkGray
        $skipped++
      }
    }
  }
  Remove-Item (Join-Path $p.LocalPath "AppData\Local\LabCtl") -Recurse -Force -ErrorAction SilentlyContinue
}

if ($skipped) {
  # Active Setup runs StubPath once for each user at their next sign-in, so
  # accounts whose hive could not be mounted are restored then.
  $as = "HKLM:\Software\Microsoft\Active Setup\Installed Components\LabCtlRestore"
  $cleanup = @"
`$s = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Policies\System'
`$a = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Policies\ActiveDesktop'
Remove-ItemProperty `$s DisableTaskMgr -ErrorAction SilentlyContinue
if ((Get-ItemProperty `$s -ErrorAction SilentlyContinue).Wallpaper -like '$dataDir*') { Remove-ItemProperty `$s Wallpaper, WallpaperStyle -ErrorAction SilentlyContinue }
Remove-ItemProperty `$a NoChangingWallPaper -ErrorAction SilentlyContinue
Remove-ItemProperty 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run' LabCtlStudentUI -ErrorAction SilentlyContinue
exit 0
"@
  # Encoded so no quoting survives the command line; not -WindowStyle Hidden,
  # which security software commonly kills (a console flashes once instead).
  $encoded = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($cleanup))
  New-Item $as -Force | Out-Null
  Set-ItemProperty $as "(default)" "LabCtl restore"
  Set-ItemProperty $as StubPath "powershell.exe -NoProfile -NonInteractive -EncodedCommand $encoded"
  Set-ItemProperty $as Version ("1,0,0," + [DateTimeOffset]::UtcNow.ToUnixTimeSeconds() % 60000)
  Set-ItemProperty $as IsInstalled 1 -Type DWord
  Write-Host "      $skipped account(s) will be restored at their next sign-in."
}

# --- 6. files ---
if (-not $RestoreOnly) {
  Step "[6/6] remove program and data ..."
  Remove-Item -Recurse -Force $InstallDir -ErrorAction SilentlyContinue
  if (-not $KeepData) { Remove-Item -Recurse -Force $dataDir -ErrorAction SilentlyContinue }
} else {
  Step "[6/6] (restore only: files left to the installer)"
  if (-not $KeepData) { Remove-Item -Recurse -Force $dataDir -ErrorAction SilentlyContinue }
}

Write-Host ""
Write-Host "OK: uninstalled; wallpaper, Task Manager, USB, firewall and UWF restored." -ForegroundColor Green
Write-Host "    The wallpaper itself stays as it is; users can change it again." -ForegroundColor DarkGray
if ($needsReboot) { Write-Host "    Reboot to finish removing the UWF feature." -ForegroundColor Yellow }
