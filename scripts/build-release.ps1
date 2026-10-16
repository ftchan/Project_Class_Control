<#
.SYNOPSIS
  在 Windows 上一键编译并打包 LabCtl 教师端 + 学生端为可直接运行的 exe。

.DESCRIPTION
  前置：安装 Rust (https://rustup.rs) 与 “Desktop development with C++”(MSVC 构建工具)。
  运行后产出：
    dist\LabCtl\labctl-teacher.exe      教师端控制台（双击运行，浏览器打开 127.0.0.1:8080）
    dist\LabCtl\labctl-student.exe      学生端（受管客户端）
    dist\LabCtl\scripts\*               一键部署/卸载脚本
    dist\LabCtl\README.md
    dist\LabCtl-<version>.zip            整包压缩，便于分发

.PARAMETER Target
  Rust 目标三元组。默认 x86_64-pc-windows-msvc（64 位，推荐）。
  32 位机房可用 i686-pc-windows-msvc（需先 rustup target add i686-pc-windows-msvc）。
#>
[CmdletBinding()]
param(
  [string]$Target = "x86_64-pc-windows-msvc"
)
$ErrorActionPreference = "Stop"
$root = Split-Path $PSScriptRoot -Parent
Push-Location $root
try {
  Write-Host "== 编译 (release, $Target) ==" -ForegroundColor Cyan
  cargo build --release --target $Target --bin labctl-teacher --bin labctl-student --bin labctl-student-ui
  if ($LASTEXITCODE -ne 0) { throw "cargo build 失败" }

  $bin = Join-Path $root "target\$Target\release"
  $teacher = Join-Path $bin "labctl-teacher.exe"
  $student = Join-Path $bin "labctl-student.exe"
  $studentUi = Join-Path $bin "labctl-student-ui.exe"
  foreach ($f in @($teacher, $student, $studentUi)) {
    if (-not (Test-Path $f)) { throw "缺少产物：$f" }
  }

  $ver = (Select-String -Path (Join-Path $root "Cargo.toml") -Pattern '^version\s*=\s*"([^"]+)"' |
          Select-Object -First 1).Matches.Groups[1].Value
  if (-not $ver) { $ver = "0.1.0" }

  $out = Join-Path $root "dist\LabCtl"
  Write-Host "== 打包到 $out ==" -ForegroundColor Cyan
  Remove-Item -Recurse -Force (Join-Path $root "dist") -ErrorAction SilentlyContinue
  New-Item -ItemType Directory -Force -Path (Join-Path $out "scripts") | Out-Null

  Copy-Item $teacher $out -Force
  Copy-Item $student $out -Force
  Copy-Item $studentUi $out -Force
  Copy-Item (Join-Path $root "README.md") $out -Force
  Copy-Item (Join-Path $PSScriptRoot "install-student.ps1")   (Join-Path $out "scripts") -Force
  Copy-Item (Join-Path $PSScriptRoot "install-student.bat")   (Join-Path $out "scripts") -Force
  Copy-Item (Join-Path $PSScriptRoot "uninstall-student.ps1") (Join-Path $out "scripts") -Force
  Copy-Item (Join-Path $PSScriptRoot "deploy-lab.ps1")        (Join-Path $out "scripts") -Force
  # 把学生端 exe（含 GUI）也放一份到 scripts 旁，方便“双击 bat 一键部署”。
  Copy-Item $student (Join-Path $out "scripts") -Force
  Copy-Item $studentUi (Join-Path $out "scripts") -Force

  # 教师端快捷启动器
  @"
@echo off
cd /d "%~dp0"
start "" "labctl-teacher.exe"
timeout /t 2 >nul
start "" http://127.0.0.1:8080
"@ | Set-Content -Path (Join-Path $out "启动教师端.bat") -Encoding OEM

  $zip = Join-Path $root "dist\LabCtl-$ver.zip"
  Compress-Archive -Path $out -DestinationPath $zip -Force

  Write-Host ""
  Write-Host "✔ 完成" -ForegroundColor Green
  Write-Host "  目录: $out"
  Write-Host "  压缩: $zip"
  Write-Host "  教师端: 双击 '启动教师端.bat'"
  Write-Host "  学生端: 进 scripts\ 双击 install-student.bat（管理员）"
}
finally { Pop-Location }
