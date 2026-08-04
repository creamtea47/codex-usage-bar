<div align="center">

# CodexUsageBar

**一个隐私优先的 Windows Codex 用量悬浮卡片。**

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

### 用量悬浮卡

![CodexUsageBar 主界面：显示两个动态额度窗口、重置时间和手动刷新按钮](docs/images/dashboard-light.png)

### 设置与手动检查更新

![CodexUsageBar 设置面板：置顶、锁定、开机启动、检查更新、刷新间隔和主题](docs/images/settings-update.png)

## 功能

- 动态显示接口返回的全部额度窗口、剩余百分比、重置时间和本地倒计时。
- 启动立即刷新，并可按 1 / 3 / 5 / 10 / 30 分钟自动刷新；两次请求之间只更新本地倒计时。
- 无边框、可拖动、可缩放的 Windows 悬浮卡，支持跟随系统、浅色、深色主题、置顶和位置锁定。
- 关闭窗口即退出，不创建托盘常驻进程；可选当前用户开机自启。
- 设置面板提供**手动检查更新**：只检查本仓库公开 GitHub Release，不会在后台轮询、自动下载、替换或重启应用。

## 安装

1. 前往 [Releases](https://github.com/creamtea47/codex-usage-bar/releases/latest)。
2. 大多数 Windows 电脑下载 `CodexUsageBar-x64-setup.exe`；Windows on ARM 下载 `CodexUsageBar-arm64-setup.exe`。
3. 运行安装包。安装包按“当前用户”安装，一般不需要管理员权限。
4. 从开始菜单启动 **CodexUsageBar**；确保 Codex 已在本机登录。

安装后的新版本会使用新的应用标识和配置目录，不依赖原来的 `luodaoyi/codex-useage-win` 仓库，也不会迁移或清理旧版设置、自启项。

## 使用方式

打开后，卡片会立即尝试读取用量。右下角“立即刷新”可手动重试；右上角设置可切换主题、刷新间隔、置顶、位置锁定和开机启动。

需要查看新版本时，打开设置并点击“检查更新”：

- 已有新版本时，应用只会打开本仓库的 Release 下载页，由你自行下载和安装。
- 已是最新版本时，会显示当前版本与最新版本。
- 暂无公开 Release 或网络不可用时，不会影响已显示的用量数据。

## 隐私与只读边界

认证文件只由 Rust 后端在本地请求期间读取，React 界面从不接收 `auth.json`、access token、邮箱、请求头或接口原始响应。前端仅收到经过过滤的状态、套餐标签、刷新时间、错误提示和额度窗口数据。

`auth.json` 的查找优先级如下：

1. 正在运行的 EXE 同目录。
2. `%CODEX_HOME%\auth.json`。
3. `%USERPROFILE%\.codex\auth.json`。

应用只读取 `GET https://chatgpt.com/backend-api/wham/usage` 的用量结果，并且：

- 不刷新 OAuth Token，不写回或修改 `auth.json`。
- 不查询或消耗重置卡，不发起 OAuth 登录流程。
- 不安装前端文件系统或 HTTP 权限插件；敏感 I/O 保持在 Rust 后端。
- 更新检查只访问 `creamtea47/codex-usage-bar` 的公开 GitHub Release 信息，不携带 Codex 认证数据。

认证文件缺失、Token 失效或接口返回未授权时，最后一次成功数据会保留并标记为“已过期”。请在 Codex 中重新登录后，再点击“立即刷新”。

## 技术架构

| 层 | 技术 | 作用 |
| --- | --- | --- |
| 桌面界面 | React + TypeScript + Material UI + Vite | 中文用量卡、主题、设置弹层、拖动与缩放交互 |
| 原生边界 | Tauri 2 command / capability | 最小化 IPC，限定窗口与打开 Release 链接的能力 |
| 数据与持久化 | Rust + Tokio + Reqwest + Serde | 只读认证、用量请求去重、内存快照、设置、自启、脱敏日志 |

对前端开放的 IPC 仅为 `get_dashboard`、`refresh_dashboard`、`get_settings`、`save_settings`、`get_autostart`、`set_autostart` 与 `check_update`。凭据永不跨越 Rust 边界。

## 日志与排查

- 设置和窗口位置：`%APPDATA%\com.creamtea47.codexusagebar\settings.json`。
- 运行日志：`%LOCALAPPDATA%\com.creamtea47.codexusagebar\logs\codex-usage-bar.log`，保留 14 天。
- 日志只记录时间、操作结果、HTTP 状态类别和脱敏错误；绝不记录 Token、Authorization 请求头或认证文件内容。
- 无法读取用量时，先确认 Codex 已登录，再点击“立即刷新”。请勿将认证信息粘贴到 Issue、截图或日志中。
- 若旧 Win32 版开通过自启，旧的 `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` 项不会被新版迁移或删除；确认新版可用后请自行关闭旧项，避免两个悬浮窗同时启动。

## 开发

开发环境需要 Windows、Node.js 22+、pnpm 10、稳定版 Rust MSVC 工具链和 Windows WebView2 Runtime；不需要全局安装 Tauri CLI。

```powershell
pnpm install --frozen-lockfile
pnpm lint
pnpm test
cargo test --locked --manifest-path src-tauri/Cargo.toml
pnpm tauri dev
```

构建当前架构的 NSIS 安装包：

```powershell
pnpm tauri build --bundles nsis
```

产物位于 `src-tauri\target\release\bundle\nsis\`。

## CI 与发布

推送到 `main` / `master` 或创建 Pull Request 时，GitHub Actions 会在原生 Windows x64 与 ARM64 运行器上执行前端 lint、前端测试、Rust 单测和 NSIS 打包。推送 `v*` 标签后，工作流会发布以下资产及 SHA-256 校验文件：

- `CodexUsageBar-x64-setup.exe`
- `CodexUsageBar-arm64-setup.exe`

维护者发布示例：

```powershell
git tag -a v0.2.1 -m "v0.2.1 正式发布"
git push origin master
git push origin v0.2.1
```

## 不包含的能力

不提供静默自动更新、自动下载安装、任务栏停靠模式、旧布局迁移、界面中英文切换、OAuth Token 续期或重置卡操作。
