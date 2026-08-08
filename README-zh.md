<div align="center">

# CodexUsageBar

**一个隐私优先、紧凑的 Windows 与 macOS Codex 用量悬浮卡片。**

使用 Tauri 2、Rust、React、TypeScript、Material UI 与 Vite 构建。

[下载最新版本](https://github.com/creamtea47/codex-usage-bar/releases/latest) · [查看截图](#截图) · [隐私与功能边界](#隐私与功能边界) · [开发指南](#开发)

[![Build](https://github.com/creamtea47/codex-usage-bar/actions/workflows/build.yml/badge.svg)](https://github.com/creamtea47/codex-usage-bar/actions/workflows/build.yml)
[![Latest Release](https://img.shields.io/github/v/release/creamtea47/codex-usage-bar?display_name=tag)](https://github.com/creamtea47/codex-usage-bar/releases/latest)
[![Downloads](https://img.shields.io/github/downloads/creamtea47/codex-usage-bar/total)](https://github.com/creamtea47/codex-usage-bar/releases)
[![Stars](https://img.shields.io/github/stars/creamtea47/codex-usage-bar?style=flat)](https://github.com/creamtea47/codex-usage-bar/stargazers)

[English](README.md)

</div>

> CodexUsageBar 默认只在本机读取已登录 Codex 的用量信息。只有用户明确开启“额度自动接续”或二次确认“立即测试”后，才会发送固定的最小 `hi` 请求；应用不上传凭据、不刷新 Token，也不修改认证文件。

## 截图

### macOS 深色主题下的紧凑用量悬浮卡

![CodexUsageBar 在 macOS 深色主题下的紧凑主界面：原生透明圆角、动态额度窗口、剩余额度和右上角控件](docs/images/dashboard-light.png)

> 正常状态不会占用额外位置；发现新版本时，刷新按钮左侧会出现绿色小更新图标。点击图标会打开“关于与更新”和安装确认框，不会直接下载或安装。

### 跟随深色主题的 macOS 设置窗口与更新页

![CodexUsageBar macOS 设置窗口：原生标题栏跟随深色主题，关于与更新页显示自动检测开关与手动更新入口](docs/images/settings-update.png)

## 功能

- 一个额度窗口时，默认以 `460 × 260` 的紧凑卡片打开，消除底部无效留白；新增额度窗口或出现可靠建议时仅按内容增高，最高 `560px`。额度卡和超过三行的建议分别在各自区域滚动；手动调整大小后会保持用户选择的尺寸。
- 动态显示接口返回的全部额度窗口、剩余百分比、重置时间，以及按绝对重置时刻逐秒递减的本地倒计时。归零后显示“正在重置…”，直到下次成功刷新提供新周期。
- 成功刷新后展示如 `j***@example.com · Pro` 的掩码账号摘要，以及下次自动刷新倒计时和最近刷新时间。
- 启动立即刷新，并按用户选择的固定 1 / 3 / 5 / 10 / 30 分钟间隔自动刷新；新安装默认 1 分钟，升级用户保留原间隔。连续失败后依次等待 1 / 3 / 5 / 10 / 30 分钟重试，之后保持 30 分钟，任意一次成功即恢复固定间隔；手动刷新可绕过等待，并从完成时刻重新排期。
- 右上角提供“刷新、设置、关闭”按钮；发现新版本时显示绿色小更新图标，点击后再由你确认安装；请求进行中会禁用刷新，避免重复请求。
- 长周期额度显示控量线，并始终在卡片右下角重复展示其四舍五入后的“建议最低剩余额度”；悬停提示会说明它仅用于本地节奏参考，不是官方阈值。本地样本足够时，可靠预测按额度卡顺序单独显示在底部建议区，不会替换控量建议。
- 提供完整的简体中文和英文界面；“显示”页可跟随系统语言或手动覆盖。任意 `zh-*` 系统语言解析为简体中文，其余语言回落英文。
- 无边框、可拖动、可缩放的桌面悬浮卡，支持跟随系统、浅色、深色主题、置顶和位置 / 大小锁定；在 macOS 失焦状态下首次点击也会直接响应。
- 默认点击主窗口关闭按钮会隐藏到系统托盘并继续后台刷新；托盘菜单可显示主窗口、打开设置或明确退出。关闭“关闭时最小化到托盘”后，主窗口关闭按钮才会直接退出。
- 设置以独立、不透明的原生窗口打开，左侧依次分为“显示”“数据与刷新”“通知”“趋势”“额度自动接续”“启动”“关于与更新”七类。
- “额度自动接续”默认关闭。开启后依据当前账号 6–8 天周窗口的 `reset_at`，在重置点发送固定 `hi`；失败最多在 `+1`、`+5`、`+30` 分钟重试，成功立即停止。由于 [OpenAI 模型目录](https://developers.openai.com/api/docs/models) 会持续迭代，模型来自账号实时清单：有 `gpt-5.4` 时优先使用，否则选首个非图片文本模型，清单不可用时才兼容回退。
- 独立“通知”页提供低剩余额度、消耗快于时间进度和额度周期重置提醒。总开关默认关闭，三个子项默认开启；低额度初值为 20%，超速差值初值为 10 个百分点，两者都支持 0–100。静默时段默认关闭，初始范围为 22:00–08:00，支持跨午夜，期间抑制的事件不会在结束后补发。
- 在设置中提供独立的 24 小时 / 7 天趋势页。采集默认开启，仅保存到本机 `usage-history.json`，可暂停或清除；检测到账号指纹变化时立即清空旧账号历史。暂停后只停止新增样本，已有图表与预测仍可查看。每个额度窗口显示固定 0–100% 图表、重置周期断线、近段消耗和四态本地预测。
- “诊断与反馈”提供仅含白名单的诊断摘要、只写剪贴板操作、独立的新建 Issue 外链，以及校验当前版本后的 Release 说明外链；诊断内容不会拼进 Issue URL。
- 默认在启动后自动检查一次、之后最多每 6 小时检查一次；可在“关于与更新”中关闭。该页还提供“GitHub 仓库”按钮，且只允许打开当前仓库的精确 HTTPS 根地址。自动检查只读取包含各平台更新包签名的公开 HTTPS `latest.json` 清单，不会下载、安装或重启应用。
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

打开后，卡片会立即尝试读取用量。右上角“刷新”可立即手动重试，即使当前正等待失败退避也不受影响；旁边的“设置”会打开独立窗口，其中按“显示”“数据与刷新”“通知”“趋势”“额度自动接续”“启动”“关于与更新”分类。

“显示”页可切换语言、主题、置顶和位置 / 大小锁定；“数据与刷新”页只包含固定刷新间隔和隐私说明；“通知”页包含系统权限、提醒规则、静默时段和测试操作；“趋势”页切换 24 小时 / 7 天、控制本地采集并提供二次确认的清除操作；“额度自动接续”页显示目标重置、下一尝试、尝试槽和脱敏结果；“启动”页控制开机自启及关闭时托盘行为；“关于与更新”页显示当前版本、更新操作、仓库精确外链、脱敏诊断、问题反馈和更新说明。关闭设置窗口只会隐藏设置页；主窗口默认关闭到托盘，托盘“退出”始终直接退出。

自动接续依赖应用在托盘中保持运行且电脑处于唤醒状态。睡眠或退出超过目标重置点 30 分钟会把本周期标记为“已错过”，恢复时只占用最近一个到期槽，不会连续补跑。手动“立即测试”必须二次确认，只发送一次且不自动重试。

系统通知只有在你手动开启并授予操作系统权限后才生效。首次成功快照只建立基线，不会立即提醒当时已经处于低额度的窗口。同一额度周期内，每类已开启的低额度或超速事件只提醒一次；`resetAt` 向后进入新周期时可提醒一次重置。多个窗口同次触发会合并为一条不含账号的通知，最多列出三个窗口。静默时段按本地时间的半开区间 `[开始, 结束)` 判断，支持跨午夜，期间被抑制的事件不会补发。Windows 上的通知名称和图标必须以安装后的 NSIS 包为准；开发环境通知不作为正式验收结果。

趋势采集对新安装和升级用户都默认开启，只记录成功快照，最多保留七天，并在重置周期之间断线。预测仅使用当前周期的近期样本，显示“正在采集”“近期稳定”“预计在重置前耗尽”或“预计可撑到重置”；可靠的耗尽时刻按 15 分钟取整，并且不会外推到重置之后。关闭采集只停止新增样本，不会隐藏、删除或上传已有图表与预测明细；“清除历史”会在二次确认后永久删除已有曲线点。

“GitHub 仓库”只打开 `https://github.com/creamtea47/codex-usage-bar`。“复制脱敏诊断”只把白名单摘要写入剪贴板。“反馈问题”会单独打开 GitHub Issue 页面，绝不会附加诊断；请先检查复制内容，再自行决定是否粘贴。“查看更新说明”会为合法的 `X.Y.Z` 应用版本打开对应 tag 页面，版本异常时回退到仓库 Releases 总页。

应用默认会在启动后自动检查一次、之后最多每 6 小时检查一次，也可以在设置中关闭。需要立即检查时，打开设置并点击“检查更新”：

- 已有新版本时，主卡右上角会显示绿色小更新图标；点击图标会直接打开更新页和确认框。你点击“下载并安装”后，应用才会下载并验证更新包签名，再交接原生安装流程。
- Windows 开始安装时会关闭 CodexUsageBar；macOS 替换 App 后会重启。应用不会在未确认时静默下载、替换或重启。
- 已是最新版本、网络不可用或签名验证失败时，不会影响已显示的用量数据；签名失败会取消安装。

## 隐私与功能边界

认证文件只由 Rust 后端在请求期间读取。常规用量、趋势和通知功能保持只读；自动接续只有在用户明确开启或确认测试后才会发送真实请求。React 前端仅收到经过过滤的用量快照和自动接续状态；它从不接收 `auth.json`、access token、请求头、接口原始响应、模型回复、原始错误或未掩码邮箱。

`auth.json` 的查找优先级如下：

1. 正在运行的应用可执行文件同目录。
2. `%CODEX_HOME%\auth.json`。
3. Windows：`%USERPROFILE%\.codex\auth.json`；macOS：`~/.codex/auth.json`。

用量功能仅读取 `GET https://chatgpt.com/backend-api/wham/usage` 的用量结果；更新功能独立读取公开 HTTPS `latest.json` 清单，并且：

- 不读取 refresh token、不刷新 OAuth Token、不写回或修改 `auth.json`；每次自动尝试都会重新读取最新 access token。
- 自动接续先只读刷新用量；若 `reset_at` 已推进则不发送。需要发送时先读取账号实时 Codex 模型清单，再向 `POST https://chatgpt.com/backend-api/codex/responses` 发送固定 `hi`，强制 `stream: true`、`store: false`，且只在 SSE 收到 `response.completed` 时判定成功。模型回复正文不展示、不保存。
- 自动接续运行状态保存在 `settings.json` 同目录下独立、版本化的 `quota-auto-continue.json`。它只包含本机随机盐、加盐账号 / 周窗口指纹、目标时间、已占用尝试槽、完成标记、时间戳和脱敏状态 / 错误码；不保存 Token、账号 ID、邮箱、请求头、模型名、响应正文或模型回复。同一尝试槽会在发请求前原子落盘，崩溃恢复后不会重复执行。
- 不查询或消耗重置卡，不发起 OAuth 登录流程。
- 不安装前端文件系统或 HTTP 权限插件；敏感 I/O 保持在 Rust 后端。
- 掩码账号摘要仅在用量请求成功且接口提供账号邮箱时生成；不会从 `auth.json` 读取邮箱，不持久化未掩码邮箱，也不会将任一种邮箱写入运行日志。
- 趋势历史保存在 `settings.json` 同目录下独立、版本化的 `usage-history.json`。文件只包含本机随机盐、不可逆的加盐账号指纹、匿名的本机加盐额度流键 / 周期长度、重置周期时间、采样时间和剩余百分比；上游窗口 ID 也只参与哈希，不会原样落盘。不保存账号 ID、Token、邮箱、上游窗口名称、原始响应、代理、URL 或认证路径。账号指纹变化时，会先清空全部旧样本再记录新账号。
- 设置窗口只能通过专属历史 IPC 取得脱敏后的百分比、时间、预测和 fallback 标签元数据；它不能调用主卡的 `get_dashboard`，因此不会取得掩码账号、套餐或实时原始快照。
- 系统通知只使用本地化 fallback 窗口名和额度数值，不包含掩码账号、上游窗口名称、Token 或原始响应。
- 诊断摘要采用固定白名单：schema 与应用版本、平台 / 架构、语言、主题、刷新间隔、连续失败次数、通知开关 / 权限、Dashboard 状态 / 错误代码、窗口数、最近刷新、更新状态、历史样本数 / 存储状态、托盘关闭开关，以及自动接续开关 / 固定枚举状态。明确排除 Token、账号、邮箱、路径、代理、URL、额度百分比、重置时间、尝试时间、曲线点、原始错误和日志。
- 设置 WebView 只有剪贴板**写文本**权限，没有读取权限；外链仅允许仓库精确根地址、当前仓库的 Release 页面和新建 Issue 页面，仓库根地址不使用通配符。主 WebView 没有剪贴板或通知插件权限；通知权限申请与测试都必须经过设置窗口专属 Rust 命令。
- 更新检查只访问 `creamtea47/codex-usage-bar` 的包含各平台更新包签名的公开 HTTPS `latest.json` 清单，不携带 `auth.json`、Token、邮箱或用量数据。下载包必须与内置公钥匹配才会安装。

认证文件缺失、Token 失效或接口返回未授权时，最后一次成功数据会保留并标记为“已过期”。请在 Codex 中重新登录后，再点击右上角“刷新”。

## 技术架构

| 层 | 技术 | 作用 |
| --- | --- | --- |
| 桌面界面 | React 19 + TypeScript + Material UI + i18next + Recharts 3 + Vite | 紧凑双语用量卡、动态加载趋势页、主题、侧栏设置、无障碍、拖动与缩放交互 |
| 原生边界 | Tauri 2 command / event / capability | 按窗口限制 IPC、系统通知权限与发送、只写剪贴板，以及仓库根地址 / Issue / Release 精确外链 |
| 数据与持久化 | Rust + Tokio + Reqwest + Serde | 默认只读用量、选择性最小接续请求、固定刷新与失败退避、请求去重、版本化脱敏运行状态、本地历史、预测、设置、自启和脱敏日志 |

完整前端 IPC 另包含设置窗口限定的 `get_quota_auto_continue_status`、`set_quota_auto_continue_enabled` 和 `test_quota_auto_continue`；状态通过 `quota-auto-continue-updated` 事件更新。普通 `save_settings` 会保留当前自动接续开关，不能绕过专属副作用入口。`get_dashboard` 和手动刷新仅限主窗口；通知、历史、自动接续、界面故障上报、诊断、自启和更新操作仅限设置窗口。更新 IPC 只返回版本摘要和下载进度；URL、签名、原始清单、凭据和未掩码账号信息永不跨越 Rust 边界。

## 日志与排查

- 设置及主卡 / 设置窗口的位置：Windows 为 `%APPDATA%\com.creamtea47.codexusagebar\settings.json`；macOS 为 `~/Library/Application Support/com.creamtea47.codexusagebar/settings.json`。
- 自动接续脱敏运行状态：同目录下的 `quota-auto-continue.json`；关闭功能不会删除排期状态，重新开启后会先按当前账号最新快照校验。
- 本地趋势样本：位于 `settings.json` 同目录的 `usage-history.json`；固定保留七天，每个流最多 2,500 点、最多 16 个流，可在“趋势”中暂停或清除，绝不上传。
- 运行日志：Windows 为 `%LOCALAPPDATA%\com.creamtea47.codexusagebar\logs\codex-usage-bar.log`；macOS 为 `~/Library/Logs/com.creamtea47.codexusagebar/codex-usage-bar.log`；均保留 14 天。
- 日志只记录时间、级别、任务时点、尝试序号、所选模型、操作结果和脱敏错误类别；不记录 Token、账号、Authorization 请求头、认证文件内容、原始 SSE、响应正文或模型回复。设置界面故障仅记录固定类别，更新日志仅记录版本号和脱敏结果。
- 受代理网络限制时，原生请求会遵循 Windows/macOS 系统代理与 `HTTP_PROXY` / `HTTPS_PROXY`；代理地址和凭据不会写入日志。
- 无法读取用量时，先确认 Codex 已登录，再点击右上角“刷新”。自动失败会按界面显示的 1 / 3 / 5 / 10 / 30 分钟退避，成功后恢复用户选择的固定间隔。
- 可使用“复制脱敏诊断”取得白名单摘要。请勿向 Issue、截图或评论提交 `auth.json`、Token、未经检查的日志、凭据或私人账号信息。
- 若旧 Win32 版开通过自启，旧的 `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` 项不会被新版迁移或删除；确认新版可用后请自行关闭旧项，避免两个悬浮窗同时启动。

## 开发

开发环境需要 Node.js 22.13+ 或 24+、pnpm 10 和稳定版 Rust。Windows 还需要 MSVC 工具链与 WebView2 Runtime；macOS 需要 Xcode Command Line Tools。不需要全局安装 Tauri CLI。

主悬浮卡依赖真正的原生透明窗口来呈现 CSS 圆角。Tauri 在 macOS 上需要 `app.macOSPrivateApi: true` 才能透明化 WKWebView 背景；缺少该项会在四角露出白色矩形底层。该私有 API 不适用于 Mac App Store，但不影响当前 GitHub Release 的 DMG 分发方式。

macOS 的应用内更新只允许从标准 `.app/Contents/MacOS` 包内执行。`tauri dev` 的裸调试二进制会在下载前拒绝安装，避免更新器把 `target/debug` 误当成需要替换的 App 目录；请使用本地 DMG 验证完整安装流程。

Windows 的通知应用名称与图标只能用安装后的 NSIS 包验收；开发可执行文件的通知身份不作为发布验收结果。

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
git tag -a v0.4.0 -m "v0.4.0 托盘关闭与额度自动接续"
git push origin master
git push origin v0.4.0
```

## 不包含的能力

不提供未确认的静默下载或更新、任务栏停靠模式、通用旧布局迁移、云端历史同步、自定义趋势日期范围、自定义接续提示词、OAuth Token 续期、系统级定时唤醒或多账号管理。已有安装仅会按上述规则进行一次紧凑高度调整。
