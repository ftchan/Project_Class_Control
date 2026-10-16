; LabCtl 安装包脚本（Inno Setup 6+）。
; 用法：先运行 scripts\build-release.ps1 生成 exe，再用 Inno Setup 编译本文件，
; 产出 Output\LabCtl-Setup.exe —— 一个正规安装包，会出现在“程序和功能”中，
; 可选择安装“教师端”或“学生端（受管客户端，自动注册服务）”。
;
; 仅用于学校内网、管理员授权的自有设备。

#define MyAppName "LabCtl 机房课堂管理"
#define MyAppVersion "0.1.0"
#define MyAppPublisher "Lab Admin Tools"
; 相对本 .iss 的编译产物目录
#define BinDir "..\target\x86_64-pc-windows-msvc\release"

[Setup]
AppId={{B6B8F3A2-0C1D-4E2A-9A77-LABCTL000001}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppPublisher={#MyAppPublisher}
DefaultDirName={autopf}\LabCtl
DefaultGroupName=LabCtl
DisableProgramGroupPage=yes
OutputBaseFilename=LabCtl-Setup
Compression=lzma2
SolidCompression=yes
PrivilegesRequired=admin
ArchitecturesInstallIn64BitMode=x64compatible
WizardStyle=modern

[Languages]
Name: "cn"; MessagesFile: "compiler:Languages\ChineseSimplified.isl"

[Types]
Name: "teacher"; Description: "教师端（控制台）"
Name: "student"; Description: "学生端（受管客户端）"
Name: "custom";  Description: "自定义"; Flags: iscustom

[Components]
Name: "teacher"; Description: "教师端控制台 labctl-teacher"; Types: teacher custom
Name: "student"; Description: "学生端受管客户端 labctl-student（注册为服务）"; Types: student custom

[Files]
Source: "{#BinDir}\labctl-teacher.exe";    DestDir: "{app}"; Components: teacher; Flags: ignoreversion
Source: "{#BinDir}\labctl-student.exe";    DestDir: "{app}"; Components: student; Flags: ignoreversion
Source: "{#BinDir}\labctl-student-ui.exe"; DestDir: "{app}"; Components: student; Flags: ignoreversion
Source: "..\README.md";                    DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\教师端控制台"; Filename: "{app}\labctl-teacher.exe"; Components: teacher
Name: "{group}\学生端客户端"; Filename: "{app}\labctl-student-ui.exe"; Components: student
Name: "{group}\卸载 LabCtl"; Filename: "{uninstallexe}"

[Registry]
; 学生端 GUI 开机自启（本机所有用户）。
Root: HKLM; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; \
  ValueType: string; ValueName: "LabCtlStudentUI"; \
  ValueData: """{app}\labctl-student-ui.exe"""; \
  Components: student; Flags: uninsdeletevalue

[Run]
; 安装学生端：注册并启动服务（install 子命令内部完成 SCM 注册与失败恢复设置）。
Filename: "{app}\labctl-student.exe"; Parameters: "install"; Components: student; \
  Flags: runhidden waituntilterminated; StatusMsg: "正在注册学生端服务..."
; 防火墙放行教师端访问（LAN 入站 TCP 47800 / UDP 47801）。
Filename: "{sys}\netsh.exe"; Components: student; Flags: runhidden; \
  Parameters: "advfirewall firewall add rule name=""LabCtl Agent (TCP 47800)"" dir=in action=allow protocol=TCP localport=47800 profile=any"
Filename: "{sys}\netsh.exe"; Components: student; Flags: runhidden; \
  Parameters: "advfirewall firewall add rule name=""LabCtl Discovery (UDP 47801)"" dir=in action=allow protocol=UDP localport=47801 profile=any"
; 立即启动 GUI 客户端。
Filename: "{app}\labctl-student-ui.exe"; Components: student; \
  Flags: nowait postinstall skipifsilent; Description: "启动学生端客户端"

[UninstallRun]
; 卸载前先结束 GUI 客户端、移除防火墙规则并移除服务。
Filename: "{sys}\netsh.exe"; Parameters: "advfirewall firewall delete rule name=""LabCtl Agent (TCP 47800)"""; \
  Components: student; Flags: runhidden; RunOnceId: "DelFwTcp"
Filename: "{sys}\netsh.exe"; Parameters: "advfirewall firewall delete rule name=""LabCtl Discovery (UDP 47801)"""; \
  Components: student; Flags: runhidden; RunOnceId: "DelFwUdp"
Filename: "{sys}\taskkill.exe"; Parameters: "/f /im labctl-student-ui.exe"; \
  Components: student; Flags: runhidden; RunOnceId: "StopLabCtlUi"
Filename: "{app}\labctl-student.exe"; Parameters: "uninstall"; Components: student; \
  Flags: runhidden waituntilterminated; RunOnceId: "RemoveLabCtlService"

[Messages]
cn.WelcomeLabel2=本向导将安装 {#MyAppName}。%n%n仅供学校机房管理员在内网自有设备上使用。
