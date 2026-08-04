<div align="center">

# CodexUsageBar

**一个隐私优先、紧凑的 Windows 与 macOS Codex 用量悬浮卡片。**

使用 Tauri 2、Rust、React、TypeScript、Material UI 与 Vite 构建。

[下载最新版本](https://github.com/creamtea47/codex-usage-bar/releases/latest) · [查看截图](#截图) · [隐私与只读边界](#隐私与只读边界) · [开发指南](#开发)

[![Build](https://github.com/creamtea47/codex-usage-bar/actions/workflows/build.yml/badge.svg)](https://github.com/creamtea47/codex-usage-bar/actions/workflows/build.yml)
[![Latest Release](https://img.shields.io/github/v/release/creamtea47/codex-usage-bar?display_name=tag)](https://github.com/creamtea47/codex-usage-bar/releases/latest)
[![Downloads](https://img.shields.io/github/downloads/creamtea47/codex-usage-bar/total)](https://github.com/creamtea47/codex-usage-bar/releases)
[![Stars](https://img.shields.io/github/stars/creamtea47/codex-usage-bar?style=flat)](https://github.com/creamtea47/codex-usage-bar/stargazers)

[English](README.md)

</div>

> CodexUsageBar 只在本机读取已登录 Codex 的用量信息；它不上传凭据、不刷新 Token，也不会操作重置卡。

## 截图

### macOS 深色主题下的紧凑用量悬浮卡

![CodexUsageBar 在 macOS 深色主题下的紧凑主界面：原生透明圆角、动态额度窗口、剩余额度和右上角控件](docs/images/dashboard-light.png)

> 正常状态不会占用额外位置；发现新版本时，刷新按钮左侧会出现绿色小更新图标。点击图标会打开“关于与更新”和安装确认框，不会直接下载或安装。

### 跟随深色主题的 macOS 设置窗口与更新页

![CodexUsageBar macOS 设置窗口：原生标题栏跟随深色主题，关于与更新页显示 v0.2.7、自动检测开关与手动更新入口](docs/images/settings-update.png)

## 功能

- 一个额度窗口时，默认以 `460 × 260` 的紧凑卡片打开，消除底部无效留白；新增额度窗口时仅按内容增高，最高 `560px`，超出后只滚动额度区域。手动调整大小后会保持用户选择的尺寸。
- 动态显示接口返回的全部额度窗口、剩余百分比、重置时间和本地倒计时。
- 成功刷新后展示如 `j***@example.com · Pro` 的掩码账号摘要，以及下次自动刷新倒计时和最近刷新时间。
- 启动立即刷新，并可按 1 / 3 / 5 / 10 / 30 分钟自动刷新；两次请求之间只更新本地倒计时。
- 右上角提供“刷新、设置、关闭”按钮；发现新版本时显示绿色小更新图标，点击后再由你确认安装；请求进行中会禁用刷新，避免重复请求。
- 长周期额度显示控量线；悬停提示会说明它是按额度窗口已过去的时间计算出的“建议最低剩余额度”，仅用于本地节奏参考，不是官方阈值。
- 无边框、可拖动、可缩放的桌面悬浮卡，支持跟随系统、浅色、深色主题、置顶和位置 / 大小锁定；在 macOS 失焦状态下首次点击也会直接响应。
- 关闭窗口即退出，不创建托盘常驻进程；可选当前用户登录时自启。
- 设置以独立、不透明的原生窗口打开，左侧分为“显示”“数据与刷新”“启动”“关于与更新”四类。
- 默认在启动后自动检查一次、之后最多每 6 小时检查一次；可在“关于与更新”中关闭。自动检查只读取包含各平台更新包签名的公开 HTTPS `latest.json` 清单，不会下载、安装或重启应用。
- 发现新版时主卡右上角会显示绿色小更新图标；点击后会直接打开“关于与更新”和安装确认框。只有确认“下载并安装”后，Rust 后端才会下载并验证 Tauri 签名。Windows 会交接当前用户 NSIS 安装器，macOS 会在必要时请求系统授权、替换 App 后重启。

## 安装

1. 前往 [Releases](https://github.com/creamtea47/codex-usage-bar/releases/latest)，按设备选择资产：

   | 设备 | 下载文件 | 安装方式 |
   | --- | --- | --- |
   | Windows x64（大多数电脑） | `CodexUsageBar-x64-setup.exe` | 运行 NSIS 安装包；按当前用户安装，通常无需管理员权限。 |
   | Windows on ARM | `CodexUsageBar-arm64-setup.exe` | 运行 NSIS 安装包；按当前用户安装，通常无需管理员权限。 |
   | Intel Mac（macOS 11+） | `CodexUsageBar-macos-x64.dmg` | 打开 DMG，将 App 拖入“应用程序”。 |
   | Apple 芯片 Mac（macOS 11+） | `CodexUsageBar-macos-arm64.dmg` | 打开 DMG，将 App 拖入“应用程序”。 |

2. 确保 Codex 已在本机登录，再启动 **CodexUsageBar**。

> 从 v0.2.5 或更早版本升级时，请先手动安装首个启用应用内更新器的版本（v0.2.6 或更高）。安装完成后，后续版本才可以通过应用自动检测并由你确认安装。

当前 macOS DMG 采用临时代码签名，尚未经过 Apple 公证；首次打开若被 Gatekeeper 拦截，请在“系统设置 → 隐私与安全性”允许打开，或按住 Control 点击 App 后选择“打开”。应用内更新的 Tauri 签名用于验证更新包完整性，不能替代 Apple Developer ID 签名与公证。

安装后的新版本会使用新的应用标识和配置目录，不依赖原来的 `luodaoyi/codex-useage-win` 仓库，也不会迁移或清理旧版设置、自启项。

### Windows 旧版升级提示

Windows 安装包的发布者为 `creamtea47`。为兼容旧版本升级，早期版本已将同一安装目录同时写入旧的 `luodaoyi` 键和新的 `creamtea47` 键，因此当前安装器可正确定位已有卸载器。它只影响 Windows 安装兼容性，不影响 Git 仓库、更新地址、应用标识或数据目录。

若当前正停在 v0.2.1 的该报错窗口，直接退出安装器并下载当前正式版本；无需手动删除注册表或 `auth.json`。

## 使用方式

打开后，卡片会立即尝试读取用量。右上角“刷新”可手动重试；旁边的“设置”会打开独立窗口，其中按“显示”“数据与刷新”“启动”“关于与更新”分类。

“显示”页可切换主题、置顶、位置 / 大小锁定；“数据与刷新”页设置刷新间隔；“启动”页控制开机自启；“关于与更新”页显示当前版本、自动检查开关和更新操作。关闭设置窗口只会隐藏设置页，悬浮卡会继续运行；关闭悬浮卡才会退出应用。

应用默认会在启动后自动检查一次、之后最多每 6 小时检查一次，也可以在设置中关闭。需要立即检查时，打开设置并点击“检查更新”：

- 已有新版本时，主卡右上角会显示绿色小更新图标；点击图标会直接打开更新页和确认框。你点击“下载并安装”后，应用才会下载并验证更新包签名，再交接原生安装流程。
- Windows 开始安装时会关闭 CodexUsageBar；macOS 替换 App 后会重启。应用不会在未确认时静默下载、替换或重启。
- 已是最新版本、网络不可用或签名验证失败时，不会影响已显示的用量数据；签名失败会取消安装。

## 隐私与只读边界

认证文件只由 Rust 后端在本地请求期间读取。React 前端仅收到经过过滤的状态、套餐标签、刷新时间、错误提示、额度窗口数据，以及成功用量响应中存在时的掩码邮箱。原始邮箱会先在 Rust 中转换为如 `j***@example.com` 的形式再发送到界面；前端从不接收 `auth.json`、access token、请求头、接口原始响应或未掩码邮箱。

`auth.json` 的查找优先级如下：

1. 正在运行的应用可执行文件同目录。
2. `%CODEX_HOME%\auth.json`。
3. Windows：`%USERPROFILE%\.codex\auth.json`；macOS：`~/.codex/auth.json`。

用量功能仅读取 `GET https://chatgpt.com/backend-api/wham/usage` 的用量结果；更新功能独立读取公开 HTTPS `latest.json` 清单，并且：

- 不刷新 OAuth Token，不写回或修改 `auth.json`。
- 不查询或消耗重置卡，不发起 OAuth 登录流程。
- 不安装前端文件系统或 HTTP 权限插件；敏感 I/O 保持在 Rust 后端。
- 掩码账号摘要仅在用量请求成功且接口提供账号邮箱时生成；不会从 `auth.json` 读取邮箱，不持久化未掩码邮箱，也不会将任一种邮箱写入运行日志。
- 更新检查只访问 `creamtea47/codex-usage-bar` 的包含各平台更新包签名的公开 HTTPS `latest.json` 清单，不携带 `auth.json`、Token、邮箱或用量数据。下载包必须与内置公钥匹配才会安装。

认证文件缺失、Token 失效或接口返回未授权时，最后一次成功数据会保留并标记为“已过期”。请在 Codex 中重新登录后，再点击右上角“刷新”。

## 技术架构

| 层 | 技术 | 作用 |
| --- | --- | --- |
| 桌面界面 | React + TypeScript + Material UI + Vite | 紧凑中文用量卡、主题、独立侧栏设置窗口、拖动与缩放交互 |
| 原生边界 | Tauri 2 command / capability | 最小化 IPC，限定窗口与打开 Release 链接的能力 |
| 数据与持久化 | Rust + Tokio + Reqwest + Serde | 只读认证、用量请求去重、内存快照、设置、自启、脱敏日志 |

对前端开放的 IPC 为 `get_dashboard`、`refresh_dashboard`、`get_settings`、`save_settings`、`get_autostart`、`set_autostart`、`open_settings_window`、`mark_main_size_manual`、`get_app_update_info`、`check_app_update` 与 `install_app_update`。更新 IPC 只返回版本摘要和下载进度；URL、签名、原始清单、凭据和未掩码账号信息永不跨越 Rust 边界。

## 日志与排查

- 设置及主卡 / 设置窗口的位置：Windows 为 `%APPDATA%\com.creamtea47.codexusagebar\settings.json`；macOS 为 `~/Library/Application Support/com.creamtea47.codexusagebar/settings.json`。
- 运行日志：Windows 为 `%LOCALAPPDATA%\com.creamtea47.codexusagebar\logs\codex-usage-bar.log`；macOS 为 `~/Library/Logs/com.creamtea47.codexusagebar/codex-usage-bar.log`；均保留 14 天。
- 日志只记录时间、操作结果、HTTP 状态或传输类别和脱敏错误；更新日志仅记录版本号及“发现版本 / 网络 / 签名 / 安装”等脱敏类别，绝不记录 Token、Authorization 请求头、认证文件内容、更新 URL 或签名内容。
- 受代理网络限制时，原生请求会遵循 Windows/macOS 系统代理与 `HTTP_PROXY` / `HTTPS_PROXY`；代理地址和凭据不会写入日志。
- 无法读取用量时，先确认 Codex 已登录，再点击右上角“刷新”。请勿将认证信息粘贴到 Issue、截图或日志中。
- 若旧 Win32 版开通过自启，旧的 `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` 项不会被新版迁移或删除；确认新版可用后请自行关闭旧项，避免两个悬浮窗同时启动。

## 开发

开发环境需要 Node.js 22.13+ 或 24+、pnpm 10 和稳定版 Rust。Windows 还需要 MSVC 工具链与 WebView2 Runtime；macOS 需要 Xcode Command Line Tools。不需要全局安装 Tauri CLI。

主悬浮卡依赖真正的原生透明窗口来呈现 CSS 圆角。Tauri 在 macOS 上需要 `app.macOSPrivateApi: true` 才能透明化 WKWebView 背景；缺少该项会在四角露出白色矩形底层。该私有 API 不适用于 Mac App Store，但不影响当前 GitHub Release 的 DMG 分发方式。

macOS 的应用内更新只允许从标准 `.app/Contents/MacOS` 包内执行。`tauri dev` 的裸调试二进制会在下载前拒绝安装，避免更新器把 `target/debug` 误当成需要替换的 App 目录；请使用本地 DMG 验证完整安装流程。

```powershell
pnpm install --frozen-lockfile
pnpm lint
pnpm test
cargo test --locked --manifest-path src-tauri/Cargo.toml
pnpm tauri dev
```

构建当前系统的发行包：

仓库会自动合并 `tauri.windows.conf.json` 或 `tauri.macos.conf.json`，为 Tauri 构建选择当前系统的原生安装包。普通本地构建没有发布私钥，因此应使用下列 CI 等价命令关闭发布签名与更新产物：

```powershell
# Windows：普通本地包（不需要发布私钥）
pnpm tauri build --bundles nsis --no-sign --config src-tauri/tauri.unsigned.conf.json

# macOS：普通本地包（不需要发布私钥）
pnpm tauri build --bundles dmg --no-sign --config src-tauri/tauri.unsigned.conf.json
```

产物位于 `src-tauri\target\release\bundle\nsis\` 或 `src-tauri/target/release/bundle/dmg/`。

## CI 与发布

推送到 `main` / `master` 或创建 Pull Request 时，GitHub Actions 会在原生 Windows x64、Windows ARM64、Intel Mac 与 Apple 芯片 Mac 运行器上执行前端 lint、前端测试、Rust 单测和未签名的原生打包。推送 `v*` 标签后，工作流只在 tag 构建步骤读取 GitHub Actions Secret 中的 `TAURI_SIGNING_PRIVATE_KEY`，为四个应用内更新载荷生成 Tauri 签名，再先创建草稿 Release、最后原子发布完整更新集。macOS DMG 仅用于手动安装，尚未经过 Apple 公证：

当前签名私钥不带口令；若未来改用带口令的私钥，还需在 GitHub Actions 配置 `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`。稳定更新标签必须使用 `vX.Y.Z` 格式，避免预发布版本进入稳定更新通道。

- `CodexUsageBar-x64-setup.exe`
- `CodexUsageBar-arm64-setup.exe`
- `CodexUsageBar-macos-x64.dmg`
- `CodexUsageBar-macos-arm64.dmg`
- 两个 Windows 安装包对应的 `.sig`
- 两个 macOS `.app.tar.gz` 自动更新包及对应 `.sig`
- `latest.json`（包含四个平台更新包签名的公开更新清单）

维护者发布示例：

```powershell
git tag -a v0.2.7 -m "v0.2.7 macOS 兼容与更新提示"
git push origin master
git push origin v0.2.7
```

## 不包含的能力

不提供未确认的静默下载或更新、任务栏停靠模式、通用旧布局迁移、界面中英文切换、OAuth Token 续期或重置卡操作。已有安装仅会按上述规则进行一次紧凑高度调整。
