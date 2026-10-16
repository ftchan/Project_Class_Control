# LabCtl — 机房课堂终端管理

面向学校机房的局域网课堂管理工具，参考极域电子教室 2016 的管理模式，使用 **Rust** 编写，
Web 控制台采用 **shadcn 风格**界面。分为 **教师端（控制台）** 与 **学生端（受管客户端）**。

> **适用范围与声明**
> 本工具仅用于 **学校内网**、由 **机房管理员** 部署与运行；所有受管终端均为 **学校自有设备**，
> 处于管理员已知的管理范围内。**不用于公网、攻防测试、入侵或任何未授权场景。**
> 所有功能均采用 Windows 文档化、可逆的机制实现（组策略等效的注册表项、服务控制管理器、
> WMI、DISM），不使用任何隐藏或对抗性手段：管理员随时可停止/卸载。

---

## 功能

### 学生端（受管客户端） `labctl-student`
| 功能 | 实现方式 | 说明 |
|------|----------|------|
| 锁定桌面壁纸 | 组策略等效注册表项 `NoChangingWallPaper` + 可选强制壁纸 | 可逆，清除即恢复 |
| U 盘 / 移动存储策略 | `USBSTOR\Start`（允许/禁用）、`StorageDevicePolicies\WriteProtect`（只读） | 仅影响大容量存储，不影响键鼠 |
| UWF 写入筛选器 开/关 | 读：WMI `root\standardcimv2\embedded` 的 `UWF_Filter`；写：`uwfmgr.exe`；必要时 DISM 安装可选功能 `Client-UnifiedWriteFilter` | **Windows 10 专业版**即可，无需转企业版/教育版，参考 [UWF-Manager](https://github.com/JIRBOY/UWF-Manager) |
| 客户端访问口令 | Argon2id 口令哈希校验 | 仅客户端下发哈希，防止学生随意改动客户端配置 |
| 进程保持运行 | 注册为 Windows 服务 + SCM 失败恢复（自动重启） | 常规运维防护，非反查杀；管理员可正常停止/卸载 |
| 可视化 GUI 客户端 | 同一 exe 的 `ui` 模式（slint，亮色）| 托盘常驻；可本机开关 UWF；受教师端策略控制；**首次运行设密码**，之后每次打开/退出都需密码；关闭窗口仅最小化到托盘；**开机自启** |

命令：
```
labctl-student run         前台运行（开发/调试）
labctl-student service     作为 Windows 服务运行（由 SCM 调用）
labctl-student install     注册 Windows 服务（需管理员）
labctl-student uninstall   卸载 Windows 服务（需管理员）
labctl-student ui          口令解锁界面
```

### 教师端（控制台） `labctl-teacher`
- **课堂屏幕巡览**：以缩略图网格查看本班各教学机当前画面
- **远程唤起客户端**：向受管终端下发启动客户端界面指令
- **重启并还原**：下发“重启 + 还原”指令（配合 UWF 覆盖层丢弃 / 还原机制）
- **下发班级策略**：壁纸锁定、U 盘策略、UWF、访问口令，一键下发到全部终端
- **可视化状态**：主机名、IP、登录用户、UWF 状态、策略版本、在线/离线

运行后会自动在浏览器打开控制台（默认仅监听 `127.0.0.1:8080`）：
```
labctl-teacher
# 打开 http://127.0.0.1:8080
```
若 8080 被占用或被系统保留（开启 Hyper-V/WSL/Docker 的机器常见），会依次改用
18080、28080 或系统分配的空闲端口，实际地址见窗口输出。也可用环境变量
`LABCTL_CONSOLE_BIND=127.0.0.1:9000` 指定。

---

## 架构

```
crates/common     共享：策略模型、线协议、Argon2 口令、路径、帧编解码
crates/student    学生端服务：策略执行(features/)、截屏、LAN agent
(GUI 已合并进 crates/student：同一 exe 的 ui 模式)
crates/teacher    教师端：发现/轮询引擎(engine) + axum HTTP/WS API + 内嵌前端
frontend/dist    shadcn 风格控制台界面（无外部 CDN，可离线运行）
```

通信：局域网内 TCP（长度前缀 JSON 帧，端口 `47800`）传输指令/状态/缩略图；
UDP 广播（端口 `47801`）发现终端。控制台对每条指令建立短连接，简单稳健。

---

## 构建

需要 Rust（建议 1.85+）。

```bash
cargo build --release
# 产物：
#   target/release/labctl-student(.exe)
#   target/release/labctl-teacher(.exe)
```

### 打包：教师端、学生端是两个独立产品

> 学生端依赖 Windows 专有库（服务 / WMI / 截屏），**必须在 Windows 目标下编译**，
> 无法从 macOS/Linux 交叉编译。

两端共用一个版本号，取自 `Cargo.toml` 的 `[workspace.package] version`，发布新版本时只改这一处。

| 产品 | 目录 / zip | 内容 | 安装包 |
|------|-----------|------|--------|
| 教师端 | `LabCtl-Teacher-<版本>` | `labctl-teacher.exe`、`启动教师端.bat`、README | `LabCtl-Teacher-Setup-<版本>.exe` |
| 学生端 | `LabCtl-Student-<版本>` | `students\`：`labctl-student.exe` 与安装/卸载/批量部署脚本 | `LabCtl-Student-Setup-<版本>.exe` |

1. **在 Windows 本机一键编译+打包**（推荐）
   先装好 [Rust](https://rustup.rs) 和 VS 的 “Desktop development with C++”(MSVC)，然后在仓库根目录：
   ```powershell
   .\build-release.ps1
   ```
   产出 `dist\` 下两个产品的目录和 zip；若装有 [Inno Setup 6+](https://jrsoftware.org/isinfo.php)，
   还会同时产出两个安装包（[installer/LabCtl-Teacher.iss](installer/LabCtl-Teacher.iss)、
   [installer/LabCtl-Student.iss](installer/LabCtl-Student.iss)，会进“程序和功能”）。

2. **用 GitHub Actions 云端编译**（无需本地 Windows）
   [.github/workflows/windows-build.yml](.github/workflows/windows-build.yml) 运行同一个 `build-release.ps1`。
   每次推送后在该次运行的 **Artifacts** 下载 `LabCtl-Teacher-<版本>`、`LabCtl-Student-<版本>`。
   推送到 main 时若 `v<版本>` 还没有 tag，会**自动打 tag 并创建 Release**，附带两个 zip 和两个安装包。

学生端的 Windows 专有功能（服务、注册表、WMI、截屏）仅在 **Windows 目标**编译；
在其他平台上这些实现为记录日志的空操作，便于在本机开发与测试网络/逻辑部分。

### 前端
`frontend/dist` 已包含一份可直接使用的离线界面（原生 HTML/CSS/JS，shadcn 风格），
已内嵌进 `labctl-teacher` 可执行文件。若需二次开发，可在此目录替换为 Vite+React+shadcn 构建产物
（保持输出目录为 `frontend/dist`），重新 `cargo build` 即可重新内嵌。

---

## 部署（机房管理员）

### A. 单机一键部署（最简单）
把学生端产品里的 `students\` 文件夹（含 `labctl-student.exe`、`install-student.bat`、
`install-student.ps1`）拷到 U 盘，在每台教学机**双击 `install-student.bat`**（会自动弹 UAC 提权）。
也可以直接运行 `LabCtl-Student-Setup-<版本>.exe` 安装。它会：
复制程序 → 写入初始策略 → （需要时）安装 UWF 功能 → 注册并启动服务。

机房默认策略在 `install-student.bat` 顶部这行改：
```bat
set "POLICY=-LockWallpaper -Usb readonly -Uwf on"
REM 加口令： set "POLICY=-LockWallpaper -Usb readonly -Uwf on -Passphrase lab-2016"
```

也可直接用 PowerShell（管理员）精细控制：
```powershell
.\students\install-student.ps1 -LockWallpaper -Usb readonly -Uwf on -Passphrase "lab-2016"
```
参数：`-Usb allow|readonly|deny`、`-Uwf on|off|unmanaged`、`-WallpaperPath <图片>`、
`-InstallDir <目录>`、`-Passphrase <口令>`。

### B. 整机房批量部署（从教师机推送）
前提：目标机开启 `admin$` 共享、账号有管理员权限、已装 [PsExec](https://learn.microsoft.com/sysinternals/downloads/psexec)。
把所有教学机的主机名/IP 写进 `pcs.txt`（每行一个），然后：
```powershell
.\students\deploy-lab.ps1 -HostsFile .\pcs.txt -ExePath .\students\labctl-student.exe `
    -PolicyArgs "-LockWallpaper -Usb readonly -Uwf on -Passphrase lab-2016"
```
> 校内也可改用**组策略（GPO）启动脚本**或 SCCM/Intune 分发 `install-student.ps1`，原理相同。

### C. 卸载
```powershell
.\students\uninstall-student.ps1          # 停止服务、删除程序与数据
.\students\uninstall-student.ps1 -KeepData # 保留策略/日志
```
> 卸载**不会**自动回退壁纸/U盘等注册表策略；如需恢复，先在控制台下发
> “解锁壁纸 + U盘允许 + UWF不管理”的空策略，再卸载。

### 排错：GUI 提示"计算机积极拒绝" / 教师机看不到学生机

GUI 客户端只是界面，真正监听端口、被教师端连接的是后台服务 `labctl-student`。
"积极拒绝"(错误 10061)= 本机 47800 端口没人监听，即**服务没在运行**。

在学生机（管理员）依次检查：
```bash
"C:\Program Files\LabCtl\labctl-student.exe" doctor
```
```bash
sc query LabCtlStudent
netstat -ano | findstr 47800
```
- 服务不是 `RUNNING` → 多半是**没用一键脚本/没用管理员装**，只运行了 GUI。
  重新以管理员运行 `install-student.bat`（或 `labctl-student install`）。
- 服务在跑、但教师机发现不到 → **防火墙**。新版安装器与服务启动时会自动放行
  TCP 47800 / UDP 47801；若仍不行，手动执行：
  ```bash
  netsh advfirewall firewall add rule name="LabCtl Agent (TCP 47800)" dir=in action=allow protocol=TCP localport=47800 profile=any
  netsh advfirewall firewall add rule name="LabCtl Discovery (UDP 47801)" dir=in action=allow protocol=UDP localport=47801 profile=any
  ```
- 教师机与学生机需在**同一子网**且允许 UDP 广播（跨 VLAN/AP 隔离会导致发现失败）。

### 日常管理
- 按需在控制台“班级策略”中调整并下发（UWF 变更需学生机重启后生效）。
- 教师机运行 `labctl-teacher`，浏览器打开控制台进行巡览与管理。

> 生产部署建议：为 TCP/UDP 通信增加共享密钥/ TLS 与来源校验，并将控制台绑定在教师机本机。
> 本仓库聚焦功能骨架，传输加密留作部署期加固项。
