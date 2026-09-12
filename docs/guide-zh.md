# 技术与使用指南

[返回首页](../README-zh.md)

## 使用方式

打开后，卡片会立即尝试读取用量。右上角“刷新”可立即手动重试，即使当前正等待失败退避也不受影响；旁边的“设置”会打开独立窗口，其中按“显示”“数据与刷新”“通知”“趋势”“额度自动接续”“启动”“关于与更新”分类。

“显示”页可切换语言、主题、置顶和位置 / 大小锁定；“数据与刷新”页只包含固定刷新间隔和隐私说明；“通知”页包含系统权限、提醒规则、静默时段和测试操作；“趋势”页可切换 24 小时、7 天、滚动最近 30 天、全部历史或任意过去日期的自定义范围，并控制本地采集及提供二次确认的当前账号清除操作；“额度自动接续”页显示目标重置、下一尝试、尝试槽、最近触发原因，并分开显示最近自动结果与最近手动测试；“启动”页控制开机自启及关闭时托盘行为；“关于与更新”页显示当前版本、更新操作、仓库精确外链、脱敏诊断、问题反馈和更新说明。关闭设置窗口只会隐藏设置页；主窗口默认关闭到托盘，托盘“退出”始终直接退出。

自动接续依赖应用在托盘中保持运行且电脑处于唤醒状态。睡眠或退出超过目标重置点 30 分钟会把本周期标记为“已错过”，恢复时只占用最近一个到期槽，不会连续补跑。手动“立即测试”必须二次确认，只发送一次且不自动重试。

系统通知只有在你手动开启并授予操作系统权限后才生效。首次成功快照只建立额度规则基线，不会立即提醒当时已经处于低额度的窗口；首次取得重置卡计数或账号切换也只建立卡数基线，只有同一账号的卡片总数增加才提醒。同一额度周期内，每类已开启的低额度或超速事件只提醒一次；`resetAt` 向后进入新周期时可提醒一次重置。重置卡到账可与同次额度事件合并为一条不含账号的通知，额度窗口最多列出三个。静默时段按本地时间的半开区间 `[开始, 结束)` 判断，支持跨午夜，期间被抑制的事件不会补发。Windows 上的通知名称和图标必须以安装后的 NSIS 包为准；开发环境通知不作为正式验收结果。

趋势采集对新安装和升级用户都默认开启，只记录成功快照且不按日期、点数或流数量淘汰：最新 24 小时保留采集样本，1–7 天按 15 分钟桶压缩，7–32 天按小时桶压缩，32 天以上按 UTC 日桶保留每日首尾点；所有层级都保留重置周期边界。“今日消耗”始终统计系统本地当天 00:00 到当前时刻，与当前选择的图表范围相互独立。预测仅使用当前周期的近期样本，显示“正在采集”“近期稳定”“预计在重置前耗尽”或“预计可撑到重置”；可靠的耗尽时刻按 15 分钟取整，并且不会外推到重置之后。关闭采集只停止新增样本，不会隐藏、删除或上传已有图表与预测明细；“清除历史”会在二次确认后永久删除当前账号已有曲线点。

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
- 自动接续先只读刷新用量，校验仍是同一重置事件和账号；`reset_at` 已推进不再单独构成跳过理由，只有同一事件已经记录手动或自动成功时才阻止后续发送。没有同事件成功记录时，先读取账号实时 Codex 模型清单，再向 `POST https://chatgpt.com/backend-api/codex/responses` 发送固定 `hi`，强制 `stream: true`、`store: false`，且只在 SSE 收到 `response.completed` 时判定成功。模型回复正文不展示、不保存。
- 自动接续运行状态保存在 `settings.json` 同目录下独立、版本化的 `quota-auto-continue.json`。schema v2 只包含本机随机盐、加盐账号 / 周窗口指纹、当前周期与 pending 下一周期时间、最近额度观测、脱敏 `eventId` / `generationId`、通知处置、30 分钟事件锁、已占用尝试槽、完成标记、时间戳、触发原因，以及分离的自动 / 手动脱敏结果摘要；不保存 Token、账号 ID、邮箱、请求头、响应正文或模型回复。同一尝试槽会在发请求前原子落盘，崩溃恢复后不会重复执行。
- 在 `settings.json` 同目录按日写入脱敏的 `quota-audit-YYYY-MM-DD.jsonl`，保留 14 天。每条记录仅含 UTC 时间、固定级别 / 动作码、不透明事件 / 代次 ID、触发原因、重置前后整数剩余比例与 `reset_at`、从 0 开始的尝试槽和固定错误码；不含凭据、账号 ID 或邮箱、路径或 URL、用量原始响应、提示词 / 回复正文或任意错误文本。
- 不额外请求重置卡，也不调用重置卡消耗或其他写入接口；只从现有 `wham/usage` 用量响应读取附带的重置卡总数和当前可使用数摘要，不发起 OAuth 登录流程。
- 重置卡通知基线保存在 `settings.json` 同目录下独立、版本化的 `reset-credit-notification.json`。文件只包含 schema 版本、本机随机盐、加盐账号指纹和最近一次卡片总数；不保存 Token、账号 ID、邮箱、当前可使用数或原始响应。
- 不安装前端文件系统或 HTTP 权限插件；敏感 I/O 保持在 Rust 后端。
- 掩码账号摘要仅在用量请求成功且接口提供账号邮箱时生成；不会从 `auth.json` 读取邮箱，不持久化未掩码邮箱，也不会将任一种邮箱写入运行日志。
- 趋势历史保存在 `settings.json` 同目录下独立、版本化的 `usage-history.json`。文件只包含本机随机盐、不可逆的加盐账号指纹、匿名账号分区、匿名的本机加盐额度流键 / 周期长度、重置周期时间、采样时间和剩余百分比；上游窗口 ID 也只参与哈希，不会原样落盘。不保存账号 ID、Token、邮箱、上游窗口名称、原始响应、代理、URL 或认证路径。账号切换只更换当前分区，不会删除其他账号历史。
- 设置窗口只能通过专属历史 IPC 取得脱敏后的百分比、时间、预测和 fallback 标签元数据；它不能调用主卡的 `get_dashboard`，因此不会取得掩码账号、套餐或实时原始快照。
- 系统通知只使用本地化 fallback 窗口名、额度数值，以及重置卡新增数、总数和当前可使用数；不包含掩码账号、上游窗口名称、Token 或原始响应。
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
- 重置卡通知基线：同目录下的 `reset-credit-notification.json`；只保存脱敏账号指纹和最近卡片总数，用于跨重启识别新增卡片，不保存当前可使用数或账号原文。
- 本地趋势样本：位于 `settings.json` 同目录的 `usage-history.json`；不按日期、点数或流数量淘汰，每次查询每个系列最多展示 1,000 点。最新 24 小时保留已采集样本，1–7 天按 15 分钟桶压缩，7–32 天按小时桶压缩，32 天以上按 UTC 日桶保留每日首尾点，并始终保留重置周期边界。schema v1 会无损迁移到多账号 schema v2，并在首次改写前创建一次迁移备份。可在“趋势”中暂停或清除当前账号历史，绝不上传；升级前没有样本的时段会显示为“部分覆盖”。
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
git tag -a v0.6.2 -m "v0.6.2 永久历史、多账号分区与单实例修复"
git push origin master
git push origin v0.6.2
```

## 不包含的能力

不提供未确认的静默下载或更新、任务栏停靠模式、通用旧布局迁移、云端历史同步、跨账号历史选择器、自定义接续提示词、OAuth Token 续期或系统级定时唤醒。已有安装仅会按上述规则进行一次紧凑高度调整。
