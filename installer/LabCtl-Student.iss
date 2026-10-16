; LabCtl 学生端安装包（Inno Setup 6+）。
; 用法：先运行 build-release.ps1 编译（装有 Inno Setup 时它会自动编译本文件），
; 或手动用 ISCC 编译，产出 LabCtl-Student-Setup-<版本>.exe。
; 安装学生端受管客户端：注册 Windows 服务、放行防火墙、登录自启托盘界面。
;
; 仅用于学校内网、管理员授权的自有设备。

#define MyAppName "LabCtl 学生端"
#define MyAppPublisher "Lab Admin Tools"
; 编译产物目录（相对本 .iss），可用 ISCC /DBinDir=... 覆盖
#ifndef BinDir
  #define BinDir "..\target\x86_64-pc-windows-msvc\release"
#endif
; 版本取自编译产物（即 Cargo.toml 的 [workspace.package] version），可用 /DMyAppVersion=x.y.z 覆盖
#ifndef MyAppVersion
  #define MyAppVersion GetStringFileInfo(BinDir + "\labctl-student.exe", "ProductVersion")
#endif
; 简体中文界面是 Inno Setup 的非官方语言包，没有时退回英文界面
#if FileExists(AddBackslash(CompilerPath) + "Languages\ChineseSimplified.isl")
  #define HasChinese
#endif

[Setup]
; 沿用旧合并安装包的 AppId，已装旧版的学生机可直接覆盖升级
AppId={{B6B8F3A2-0C1D-4E2A-9A77-LABCTL000001}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppPublisher={#MyAppPublisher}
DefaultDirName={autopf}\LabCtl
DefaultGroupName=LabCtl
DisableProgramGroupPage=yes
OutputBaseFilename=LabCtl-Student-Setup-{#MyAppVersion}
UninstallDisplayIcon={app}\labctl-student.exe
Compression=lzma2
SolidCompression=yes
PrivilegesRequired=admin
ArchitecturesInstallIn64BitMode=x64compatible
WizardStyle=modern

[Languages]
#ifdef HasChinese
Name: "cn"; MessagesFile: "compiler:Languages\ChineseSimplified.isl"
#else
Name: "en"; MessagesFile: "compiler:Default.isl"
#endif

[Files]
Source: "{#BinDir}\labctl-student.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\students\uninstall-student.ps1"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\README.md";                 DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\学生端客户端"; Filename: "{app}\labctl-student.exe"; Parameters: "ui --show"
Name: "{group}\卸载 LabCtl 学生端"; Filename: "{uninstallexe}"

[Registry]
; 学生端界面开机自启（同一 exe 的 ui 模式；本机所有用户）。
Root: HKLM; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; \
  ValueType: string; ValueName: "LabCtlStudentUI"; \
  ValueData: """{app}\labctl-student.exe"" ui"; Flags: uninsdeletevalue

[Run]
; 先移除旧服务（覆盖升级时），再注册并启动服务（含失败自动重启）。
Filename: "{app}\labctl-student.exe"; Parameters: "uninstall"; \
  Flags: runhidden waituntilterminated; StatusMsg: "正在移除旧版服务..."
Filename: "{app}\labctl-student.exe"; Parameters: "install"; \
  Flags: runhidden waituntilterminated; StatusMsg: "正在注册学生端服务..."
; 防火墙放行教师端访问（LAN 入站 TCP 47800 / UDP 47801）；先删再加，避免重复规则。
Filename: "{sys}\netsh.exe"; Flags: runhidden waituntilterminated; \
  Parameters: "advfirewall firewall delete rule name=""LabCtl Agent (TCP 47800)"""
Filename: "{sys}\netsh.exe"; Flags: runhidden waituntilterminated; \
  Parameters: "advfirewall firewall delete rule name=""LabCtl Discovery (UDP 47801)"""
Filename: "{sys}\netsh.exe"; Flags: runhidden waituntilterminated; \
  Parameters: "advfirewall firewall add rule name=""LabCtl Agent (TCP 47800)"" dir=in action=allow protocol=TCP localport=47800 profile=any"
Filename: "{sys}\netsh.exe"; Flags: runhidden waituntilterminated; \
  Parameters: "advfirewall firewall add rule name=""LabCtl Discovery (UDP 47801)"" dir=in action=allow protocol=UDP localport=47801 profile=any"
; 立即启动托盘界面（同一 exe 的 ui 模式）。
Filename: "{app}\labctl-student.exe"; Parameters: "ui"; \
  Flags: nowait postinstall skipifsilent; Description: "启动学生端客户端"

[UninstallRun]
; 卸载前先结束托盘界面、移除防火墙规则并移除服务。
Filename: "{sys}\netsh.exe"; Parameters: "advfirewall firewall delete rule name=""LabCtl Agent (TCP 47800)"""; \
  Flags: runhidden; RunOnceId: "DelFwTcp"
Filename: "{sys}\netsh.exe"; Parameters: "advfirewall firewall delete rule name=""LabCtl Discovery (UDP 47801)"""; \
  Flags: runhidden; RunOnceId: "DelFwUdp"
Filename: "{sys}\taskkill.exe"; Parameters: "/f /im labctl-student.exe"; \
  Flags: runhidden; RunOnceId: "StopLabCtlUi"
Filename: "{app}\labctl-student.exe"; Parameters: "uninstall"; \
  Flags: runhidden waituntilterminated; RunOnceId: "RemoveLabCtlService"
; 服务移除后再恢复它改动过的系统功能（UWF、U 盘、联网、壁纸锁定、任务管理器）。
Filename: "{sys}\WindowsPowerShell\v1.0\powershell.exe"; \
  Parameters: "-NoProfile -ExecutionPolicy Bypass -File ""{app}\uninstall-student.ps1"" -RestoreOnly"; \
  Flags: runhidden waituntilterminated; RunOnceId: "RestoreSystem"

#ifdef HasChinese
[Messages]
cn.WelcomeLabel2=本向导将安装 {#MyAppName}。%n%n仅供学校机房管理员在内网自有设备上使用。
#endif

[Code]
// 覆盖升级前先停掉正在运行的服务和托盘界面，否则 exe 被占用无法替换。
function PrepareToInstall(var NeedsRestart: Boolean): String;
var
  Code: Integer;
begin
  Exec(ExpandConstant('{sys}\sc.exe'), 'stop LabCtlStudent', '', SW_HIDE, ewWaitUntilTerminated, Code);
  Exec(ExpandConstant('{sys}\taskkill.exe'), '/f /im labctl-student.exe', '', SW_HIDE, ewWaitUntilTerminated, Code);
  Sleep(2000);
  Result := '';
end;
