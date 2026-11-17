# 工程架构与模块迁移

## 窗口和前端

`src/main.tsx` 按 Tauri 标签动态导入对应窗口。设置窗口注册事件后读取初始快照。趋势页在进入趋势导航后按需加载。新功能的页面、数据服务、视图状态和测试放在同一业务目录。

| 重构前 | 当前职责与入口 |
|---|---|
| `App.tsx`、`SettingsWindow.tsx` | `src/app/` 装配窗口、导航与设置页初始订阅。独立的设置面板在 `features/settings` 和 `features/updates`。 |
| `AccountsPage.tsx`、`accountsBridge.ts` | `features/accounts/accountsApi.ts` 定义原生调用，`accountStore.ts` 发布账号快照，`accountActions.ts` 在写操作完成后重读。两个窗口直接复用 `AccountSelector.tsx`。 |
| `bridge.ts`、`types.ts`、`accountTypes.ts` | 按业务放入 `features/*/*Api.ts` 与 `contracts/`。原生 SDK 由 `platform/` 适配。传输类型不依赖 React。 |
| `TrendsPage.tsx`、`HistoryDetails.tsx` | `features/history/` 分别处理查询、图表转换、区间选择和周期明细。额度预测文案和额外额度展示由 `features/usage/` 复用。 |
| 主题、格式化与翻译 | `shared/theme.ts`、`shared/format.ts`、`shared/i18n/`。两种语言使用对应的业务键。 |

跨模块导入使用 `@/`，模块内部使用相对路径。页面由 app 组合。依赖环、跨功能引用其他页面、绕过平台层直接调用 Tauri 均由 `pnpm lint` 检查。测试跟随实现，跨窗口装配用例放在 `app/`。

## Rust 运行时

| 重构前 | 当前职责与入口 |
|---|---|
| `lib.rs` | `app/bootstrap.rs` 装配进程，`ipc/` 注册的 36 个命令按业务拆分。窗口权限由 `ipc/access.rs` 统一校验。 |
| 账号初始化和刷新 | `app/state.rs` 创建共享设置、历史、更新器与窗口几何状态。`app/account_hub.rs` 为每个账号构造刷新、凭据、通知基线及接续状态，复用同一份共享资源。 |
| `usage_history.rs` | `domain/history/` 处理采集、查询、预测、分析与旧格式解码。`infra/history_store.rs` 负责备份、替换和恢复。`app/history.rs` 负责锁与应用服务。 |
| `quota_auto_continue.rs` | `domain/continuation.rs` 定义周期状态与转换。`app/continuation/` 处理观察、请求交付及协议，`app/continuation_scheduler.rs` 驱动任务，`infra/continuation_store.rs` 管理迁移与落盘。 |
| 账号、用量、设置、通知 | `domain/` 定义模型和规则，`infra/` 读写托管凭据、配置、通知基线并发起 HTTP 请求。平台代理观察在 `infra/network.rs`，账号唤醒在 `app/network.rs`。 |
| 托盘、窗口与轻量模式 | `app/tray.rs`、`app/windows.rs`、`app/lightweight.rs` 协调生命周期与后台资源。平台窗口激活与显示策略位于 `infra/`。 |

账号列表监听的每次注册拥有自己的清理句柄。退订后才返回的句柄会立即释放。刷新及历史读取绑定请求发起时的账号，过期响应不能替换新选择。Rust 在网络修订变化后丢弃旧读取。接续对交付状态不确定的请求保留原有停止重发语义。

## 数据和产物

现有账号目录、文件名、序列化格式、历史迁移备份及凭据权限由各自存储模块维护。账号注册表、旧版设置、历史 v1 和接续 v1 的合成文件覆盖启动、写入与再次启动。命令、窗口访问范围和事件列于 [IPC 基线](ipc.md)。

`dist/` 是 Vite 资源，Cargo 在 `target/` 下生成可执行文件和安装包。`artifacts/` 和 `output/` 用于本地验收，均由 Git 忽略。`docs/images/` 保留已发布截图，`docs/releases/` 保留版本说明。Rust 的 TCP 模拟服务位于仅在测试编译时启用的 `test_support/`。
