<#
.SYNOPSIS
  在 Windows 上一键编译 LabCtl，并把教师端、学生端分别打包成两个独立产品。

.DESCRIPTION
  前置：安装 Rust (https://rustup.rs) 与 “Desktop development with C++”(MSVC 构建工具)。
  <version> 取自 Cargo.toml 的 [workspace.package] version（两端共用一个版本号）。
  运行后产出：
    dist\LabCtl-Teacher-<version>\               教师端：labctl-teacher.exe、启动教师端.bat、README.md
    dist\LabCtl-Teacher-<version>.zip
    dist\LabCtl-Student-<version>\students\      学生端：labctl-student.exe 与安装/卸载/批量部署脚本
    dist\LabCtl-Student-<version>.zip
  若装有 Inno Setup 6（ISCC.exe），还会产出两个安装包：
    dist\LabCtl-Teacher-Setup-<version>.exe
    dist\LabCtl-Student-Setup-<version>.exe

.PARAMETER Target
  Rust 目标三元组。默认 x86_64-pc-windows-msvc（64 位，推荐）。
  32 位机房可用 i686-pc-windows-msvc（需先 rustup target add i686-pc-windows-msvc）。

.PARAMETER SkipInstaller
  不生成 Inno Setup 安装包。
#>
[CmdletBinding()]
param(
  [string]$Target = "x86_64-pc-windows-msvc",
  [switch]$SkipInstaller
)
$ErrorActionPreference = "Stop"
$root = $PSScriptRoot
Push-Location $root
try {
  Write-Host "== 编译 (release, $Target) ==" -ForegroundColor Cyan
  cargo build --release --target $Target --bin labctl-teacher --bin labctl-student
  if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }

  $bin = Join-Path $root "target\$Target\release"
  $teacher = Join-Path $bin "labctl-teacher.exe"
  $student = Join-Path $bin "labctl-student.exe"
  foreach ($f in @($teacher, $student)) {
    if (-not (Test-Path $f)) { throw "missing artifact: $f" }
  }

  $ver = (Select-String -Path (Join-Path $root "Cargo.toml") -Pattern '^version\s*=\s*"([^"]+)"' |
          Select-Object -First 1).Matches.Groups[1].Value
  if (-not $ver) { throw "version not found in Cargo.toml" }

  $dist = Join-Path $root "dist"
  Remove-Item -Recurse -Force $dist -ErrorAction SilentlyContinue

  # --- 教师端：只含教师端产物 ---
  $tDir = Join-Path $dist "LabCtl-Teacher-$ver"
  Write-Host "== 打包教师端 $tDir ==" -ForegroundColor Cyan
  New-Item -ItemType Directory -Force -Path $tDir | Out-Null
  Copy-Item $teacher $tDir -Force
  Copy-Item (Join-Path $root "README.md") $tDir -Force
  @"
@echo off
cd /d "%~dp0"
start "" "labctl-teacher.exe"
"@ | Set-Content -Path (Join-Path $tDir "启动教师端.bat") -Encoding OEM
  Compress-Archive -Path $tDir -DestinationPath "$tDir.zip" -Force

  # --- 学生端：students\ 下是 exe 与部署脚本，双击 install-student.bat 即可 ---
  $sDir = Join-Path $dist "LabCtl-Student-$ver"
  $sApp = Join-Path $sDir "students"
  Write-Host "== 打包学生端 $sDir ==" -ForegroundColor Cyan
  New-Item -ItemType Directory -Force -Path $sApp | Out-Null
  Copy-Item $student $sApp -Force
  foreach ($f in "install-student.bat", "install-student.ps1", "uninstall-student.ps1", "deploy-lab.ps1") {
    Copy-Item (Join-Path $root "students\$f") $sApp -Force
  }
  Copy-Item (Join-Path $root "README.md") $sDir -Force
  Compress-Archive -Path $sDir -DestinationPath "$sDir.zip" -Force

  # --- Inno Setup 安装包（可选） ---
  $setups = @()
  if (-not $SkipInstaller) {
    $iscc = @(
      (Get-Command ISCC.exe -ErrorAction SilentlyContinue).Source,
      "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe",
      "$env:ProgramFiles\Inno Setup 6\ISCC.exe",
      "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe"
    ) | Where-Object { $_ -and (Test-Path $_) } | Select-Object -First 1
    if ($iscc) {
      foreach ($iss in "LabCtl-Teacher.iss", "LabCtl-Student.iss") {
        Write-Host "== 编译安装包 $iss ==" -ForegroundColor Cyan
        & $iscc /Q "/DMyAppVersion=$ver" "/DBinDir=$bin" "/O$dist" (Join-Path $root "installer\$iss")
        if ($LASTEXITCODE -ne 0) { throw "ISCC failed: $iss" }
      }
      $setups = Get-ChildItem $dist -Filter "LabCtl-*-Setup-$ver.exe" | ForEach-Object { $_.FullName }
    } else {
      Write-Host "未找到 Inno Setup 6（ISCC.exe），跳过安装包。" -ForegroundColor Yellow
    }
  }

  Write-Host ""
  Write-Host "✔ 完成 v$ver" -ForegroundColor Green
  Write-Host "  教师端: $tDir.zip"
  Write-Host "  学生端: $sDir.zip"
  foreach ($s in $setups) { Write-Host "  安装包: $s" }
  Write-Host "  教师端用法: 双击 '启动教师端.bat'"
  Write-Host "  学生端用法: 进 students\ 双击 install-student.bat（管理员）"
}
finally { Pop-Location }
