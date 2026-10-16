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
Source: "{#BinDir}\labctl-teacher.exe"; DestDir: "{app}"; Components: teacher; Flags: ignoreversion
Source: "{#BinDir}\labctl-student.exe"; DestDir: "{app}"; Components: student; Flags: ignoreversion
Source: "..\README.md";                 DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\教师端控制台"; Filename: "{app}\labctl-teacher.exe"; Components: teacher
Name: "{group}\卸载 LabCtl"; Filename: "{uninstallexe}"

[Run]
; 安装学生端：注册并启动服务（install 子命令内部完成 SCM 注册与失败恢复设置）。
Filename: "{app}\labctl-student.exe"; Parameters: "install"; Components: student; \
  Flags: runhidden waituntilterminated; StatusMsg: "正在注册学生端服务..."

[UninstallRun]
; 卸载前先移除服务。
Filename: "{app}\labctl-student.exe"; Parameters: "uninstall"; Components: student; \
  Flags: runhidden waituntilterminated; RunOnceId: "RemoveLabCtlService"

[Messages]
cn.WelcomeLabel2=本向导将安装 {#MyAppName}。%n%n仅供学校机房管理员在内网自有设备上使用。
