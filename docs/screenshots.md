# Screenshot notes / 截图记录

[English README](../README.md) · [中文 README](../README-zh.md)

## v0.7.0 additions / 新增截图

2026-09-23：直接复用 v0.7.0 React 组件，通过隔离的浏览器入口使用合成账号和额度数据。未读取真实凭证或发送上游请求。这些图片展示界面，不替代原生窗口或接口验收。

- [紧凑主窗 / Chinese compact card](images/compact-main-zh.png)：340 × 296，中文浅色。
- [Compact card / 英文紧凑主窗](images/compact-main-en.png)：460 × 296，英文深色。
- [账号与重置卡有效期 / Accounts and credit expiry](images/accounts-credits-zh.png)：1000 × 840，中文浅色。

下方保留 v0.6.2 的设置页及历史界面示例。

## Capture / 拍摄来源

- Date / 日期：2026-09-12（Asia/Shanghai）。
- UI version / 界面版本：v0.6.2，source commit `360b3aa`。
- Method / 方法：Playwright + Chromium，运行本地 Vite 页面，直接复用仓库的 React 界面组件。使用隔离的文档入口，以合成数据替换 Rust IPC；不读取认证文件、不访问账户接口、不触发真实通知或模型请求。
- Data / 数据：`d***@example.com`、84% / 68% 剩余额度、趋势曲线及排期均为示例；通知与自动接续开关展示的是示例配置，不是首次安装默认值。
- Scope / 范围：browser-rendered UI screenshots, not native desktop or delivery acceptance. 这些图片用于展示界面，不作为 Windows/macOS 原生窗口、通知投递或自动接续成功的验证证据。
- Viewport / 视口：主卡 520 × 440；设置页宽 1120，高度按内容调整，完整展示页面。

## Gallery / 文件清单

Each view has English (`-en`) and Simplified Chinese (`-zh`) captures. 每组各含中英文两张，共 14 张。

| View / 页面 | English | 中文 |
| --- | --- | --- |
| Light card / 浅色卡片 | [PNG](images/dashboard-light-en.png) | [PNG](images/dashboard-light-zh.png) |
| Dark card / 深色卡片 | [PNG](images/dashboard-dark-en.png) | [PNG](images/dashboard-dark-zh.png) |
| Display / 显示 | [PNG](images/display-en.png) | [PNG](images/display-zh.png) |
| Notifications / 通知 | [PNG](images/notifications-en.png) | [PNG](images/notifications-zh.png) |
| Trends / 趋势 | [PNG](images/trends-en.png) | [PNG](images/trends-zh.png) |
| Auto-continuation / 自动接续 | [PNG](images/auto-continue-en.png) | [PNG](images/auto-continue-zh.png) |
| About / 关于与更新 | [PNG](images/about-en.png) | [PNG](images/about-zh.png) |

## UI sources / 对应源码

- [App.tsx](../src/App.tsx)：主卡。
- [SettingsWindow.tsx](../src/SettingsWindow.tsx)：设置导航、显示与更新。
- [NotificationsPage.tsx](../src/NotificationsPage.tsx)：通知规则。
- [TrendsPage.tsx](../src/TrendsPage.tsx)：趋势与采集控制。
- [QuotaAutoContinuePage.tsx](../src/QuotaAutoContinuePage.tsx)：接续状态与确认界面。

## README structure references / 文档结构参考

参考 [CodexBar](https://github.com/steipete/CodexBar/blob/main/README.md) 的用途、安装与功能组织方式，以及 [PowerToys](https://github.com/microsoft/PowerToys/blob/main/README.md) 的下载入口和详细文档分层。仅借鉴信息组织，功能说明依据本项目源码。
