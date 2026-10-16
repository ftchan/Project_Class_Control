; LabCtl 教师端安装包（Inno Setup 6+）。
; 用法：先运行 build-release.ps1 编译（装有 Inno Setup 时它会自动编译本文件），
; 或手动用 ISCC 编译，产出 LabCtl-Teacher-Setup-<版本>.exe。
; 只安装教师端控制台，不含学生端。
;
; 仅用于学校内网、管理员授权的自有设备。

#define MyAppName "LabCtl 教师端"
#define MyAppPublisher "Lab Admin Tools"
; 编译产物目录（相对本 .iss），可用 ISCC /DBinDir=... 覆盖
#ifndef BinDir
  #define BinDir "..\target\x86_64-pc-windows-msvc\release"
#endif
; 版本取自编译产物（即 Cargo.toml 的 [workspace.package] version），可用 /DMyAppVersion=x.y.z 覆盖
#ifndef MyAppVersion
  #define MyAppVersion GetStringFileInfo(BinDir + "\labctl-teacher.exe", "ProductVersion")
#endif
; 简体中文界面是 Inno Setup 的非官方语言包，没有时退回英文界面
#if FileExists(AddBackslash(CompilerPath) + "Languages\ChineseSimplified.isl")
  #define HasChinese
#endif

[Setup]
AppId={{23CB96CD-7CF7-4A05-B0AC-5D64070EBBB3}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppPublisher={#MyAppPublisher}
DefaultDirName={autopf}\LabCtl Teacher
DefaultGroupName=LabCtl
DisableProgramGroupPage=yes
OutputBaseFilename=LabCtl-Teacher-Setup-{#MyAppVersion}
UninstallDisplayIcon={app}\labctl-teacher.exe
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

[Tasks]
Name: "desktopicon"; Description: "创建桌面快捷方式"; Flags: unchecked

[Files]
Source: "{#BinDir}\labctl-teacher.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\README.md";                 DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\教师端控制台"; Filename: "{app}\labctl-teacher.exe"
Name: "{group}\卸载 LabCtl 教师端"; Filename: "{uninstallexe}"
Name: "{autodesktop}\LabCtl 教师端"; Filename: "{app}\labctl-teacher.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\labctl-teacher.exe"; Flags: nowait postinstall skipifsilent; Description: "启动教师端控制台"

[UninstallRun]
Filename: "{sys}\taskkill.exe"; Parameters: "/f /im labctl-teacher.exe"; Flags: runhidden; RunOnceId: "StopLabCtlTeacher"

#ifdef HasChinese
[Messages]
cn.WelcomeLabel2=本向导将安装 {#MyAppName}。%n%n仅供学校机房管理员在内网自有设备上使用。
#endif
