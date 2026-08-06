<div align="center">

# CodexUsageBar

**A privacy-first, compact floating Codex usage card for Windows and macOS.**

Built with Tauri 2, Rust, React, TypeScript, Material UI, and Vite.

[Download](https://github.com/creamtea47/codex-usage-bar/releases/latest) · [Screenshots](#screenshots) · [Privacy](#privacy-and-read-only-boundary) · [Development](#development)

[![Build](https://github.com/creamtea47/codex-usage-bar/actions/workflows/build.yml/badge.svg)](https://github.com/creamtea47/codex-usage-bar/actions/workflows/build.yml)
[![Latest Release](https://img.shields.io/github/v/release/creamtea47/codex-usage-bar?display_name=tag)](https://github.com/creamtea47/codex-usage-bar/releases/latest)
[![Downloads](https://img.shields.io/github/downloads/creamtea47/codex-usage-bar/total)](https://github.com/creamtea47/codex-usage-bar/releases)
[![Stars](https://img.shields.io/github/stars/creamtea47/codex-usage-bar?style=flat)](https://github.com/creamtea47/codex-usage-bar/stargazers)

[简体中文](README-zh.md)

</div>

> CodexUsageBar reads usage data from a locally signed-in Codex session. It never uploads credentials, refreshes tokens, or operates reset cards.

## Screenshots

### Compact floating usage card in macOS dark mode

![CodexUsageBar compact dashboard on macOS showing native transparent corners, dynamic quota windows, remaining usage, and top-right controls](docs/images/dashboard-light.png)

> The normal state uses no extra space. When an update is available, a small green update icon appears to the left of Refresh. It opens About & updates and the confirmation dialog; it does not immediately download or install anything.

### Theme-aware macOS Settings and updates page

![CodexUsageBar macOS Settings window with a dark native title bar and the About and updates page showing automatic checks and the manual update entry point](docs/images/settings-update.png)

## Features

- Opens as a compact `460 × 260` floating card for one quota window, removing unused bottom space. It expands only when additional windows need room (up to `560px`), and scrolls only the quota area beyond that limit. A manually resized card keeps its chosen size.
- Displays every quota window returned by the usage API, its remaining percentage, reset time, and a local countdown that ticks every second from the absolute reset deadline. At zero it shows **Resetting…** until a successful refresh supplies the next cycle.
- Shows a masked account summary such as `j***@example.com · Pro` after a successful refresh, together with the next automatic-refresh countdown and most recent refresh time.
- Refreshes at launch and on a fixed, configurable 1 / 3 / 5 / 10 / 30 minute interval. New installations default to 1 minute, while upgrades keep their saved interval. Consecutive failures retry after 1 / 3 / 5 / 10 / 30 minutes and remain at 30 minutes until a request succeeds; manual refresh bypasses the wait and reschedules from completion.
- Keeps **Refresh**, **Settings**, and **Close** in the upper-right corner. A green update icon appears when a new version is available and opens the user-confirmed install flow. Refresh is disabled while a request is running to prevent duplicate requests.
- Draws a pace marker for long quota windows. Its tooltip explains that the marker is the suggested minimum remaining quota calculated from elapsed window time; it is a local pacing estimate, not an official limit. When enough local samples exist, a reliable exhaustion forecast can replace the pace hint.
- Provides a complete Simplified Chinese and English interface. **Display** can follow the system language or override it; any `zh-*` system locale resolves to Simplified Chinese and other locales fall back to English.
- Uses a borderless, draggable, resizable desktop card with system, light, and dark themes; supports always-on-top and position / size lock, and responds to the first click while unfocused on macOS.
- Closes completely with its window—no tray-resident process. Autostart for the current account is optional.
- Opens Settings in a separate opaque native window with sidebar categories: **Display**, **Data & refresh**, **Trends**, **Startup**, and **About & updates**.
- Offers opt-in system notifications for low remaining quota, usage running ahead of elapsed-time pace, and a newly reset quota cycle. The master switch is off by default while all three rules default on: low quota starts at 20%, pace deficit at 10 percentage points, and both thresholds accept 0–100. Quiet hours default off with an initial 22:00–08:00 range, support crossing midnight, and never replay suppressed alerts later.
- Keeps a lightweight 24-hour / 7-day trend view in Settings. Collection is enabled by default, stored only in `usage-history.json`, can be paused or cleared, and is immediately cleared when the local account fingerprint changes. Each quota window has a fixed 0–100% chart with reset-cycle breaks, recent consumption, and a four-state local forecast.
- Adds **Diagnostics & feedback** with a whitelist-only diagnostic summary, a write-only clipboard action, a separate new-Issue link, and a validated current-version Release-notes link. Diagnostic text is never inserted into the Issue URL.
- Checks once shortly after launch and then at most every six hours by default; it can be disabled in **About & updates**. Automatic checks read only the public HTTPS `latest.json` manifest containing signatures for each platform update payload; they never download, install, or restart the app.
- When a new version is found, a small green update icon appears in the card's upper-right corner. It opens **About & updates** and the confirmation dialog. Only after confirming **Download and install** does Rust download and verify the Tauri signature. Windows hands off to the current-user NSIS installer; macOS may request authorization, replaces the app, and restarts.

## Install

1. Open [Releases](https://github.com/creamtea47/codex-usage-bar/releases/latest) and choose the matching asset:

   | Device | Asset | Install |
   | --- | --- | --- |
   | Windows x64 (most PCs) | `CodexUsageBar-x64-setup.exe` | Run the NSIS installer. It installs for the current user and normally needs no administrator privileges. |
   | Windows on ARM | `CodexUsageBar-arm64-setup.exe` | Run the NSIS installer. It installs for the current user and normally needs no administrator privileges. |
   | Intel Mac (macOS 11+) | `CodexUsageBar-macos-x64.dmg` | Open the DMG and drag the app to Applications. |
   | Apple silicon Mac (macOS 11+) | `CodexUsageBar-macos-arm64.dmg` | Open the DMG and drag the app to Applications. |

2. Sign in to Codex on this computer, then start **CodexUsageBar**.

> When upgrading from v0.2.5 or earlier, manually install the first updater-enabled version (v0.2.6 or newer). Automatic detection and user-confirmed installation are available only after that bootstrap upgrade.

The current macOS DMGs are ad-hoc signed and not Apple-notarized. If Gatekeeper blocks the first launch, allow the app in **System Settings → Privacy & Security**, or Control-click it and choose **Open**. The Tauri updater signature verifies update payload integrity; it does not replace Apple Developer ID signing or notarization.

The application uses a new application identifier and configuration directory. It is independent of the former `luodaoyi/codex-useage-win` repository and deliberately does not migrate or remove old settings or autostart entries.

### Windows legacy-upgrade note

Windows builds identify `creamtea47` as the publisher. For legacy-upgrade compatibility, earlier releases already wrote the same install path under both the legacy `luodaoyi` key and the new `creamtea47` key, so current installers can find an existing uninstaller correctly. This affects only Windows installer compatibility—not the Git repository, update endpoint, application identifier, or data directory.

If an old v0.2.1 installer is currently showing that error, exit it and download a current release. Do not manually delete the registry entry or `auth.json`.

## Usage

The card tries to load usage data immediately after launch. Use the top-right **Refresh** button to retry immediately, including while an automatic failure-backoff deadline is pending. The adjacent **Settings** button opens a separate window where settings are organized into **Display**, **Data & refresh**, **Trends**, **Startup**, and **About & updates**.

**Display** controls language, theme, always-on-top, and position / size lock. **Data & refresh** controls the fixed refresh interval and system-notification rules. **Trends** switches between 24 hours and 7 days, controls local collection, and provides the confirmed clear action. **Startup** controls autostart. **About & updates** contains the current version, update actions, sanitized diagnostics, feedback, and Release notes. Closing Settings only hides that window—the floating card remains running. Closing the floating card exits the application.

System notifications remain disabled until you turn them on and grant operating-system permission. The first successful snapshot establishes a baseline rather than immediately warning about an already-low window. Within one quota cycle, each enabled low-quota or pace event is sent once; a later `resetAt` starts a new cycle and can produce one reset notification. Events from several windows are merged into one account-free notification with at most three window entries. Quiet hours use local time in the half-open `[start, end)` interval, including cross-midnight ranges, and suppressed events are not replayed. On Windows, notification name and icon behavior must be judged from an installed NSIS package; development notifications are not a release acceptance result.

Trend collection starts enabled for both new and upgraded installations. Only successful snapshots are sampled, retained for at most seven days, and shown with reset-cycle discontinuities. Forecasting uses recent samples from the current cycle and reports **Collecting**, **Stable**, **Expected to run out before reset**, or **Expected to last until reset**. A reliable exhaustion time is rounded to 15 minutes and is never extrapolated past the quota reset. Turning collection off stops new samples without uploading anything; **Clear history** permanently removes existing points after confirmation.

**Copy sanitized diagnostics** writes a whitelist summary to the clipboard. **Report an issue** opens a separate GitHub Issue page and never appends diagnostics; inspect the copied summary before deciding whether to paste it. **View release notes** opens the tag page for a validated `X.Y.Z` application version and otherwise falls back to the repository's Releases page.

The app checks once shortly after launch and then at most every six hours by default; you can turn this off in Settings. Choose **Check for updates** when you want to check immediately:

- When an update is available, a small green update icon appears in the card's upper-right corner. Clicking it opens the update page and confirmation dialog. Only clicking **Download and install** downloads the payload, verifies its signature, and hands it to the native installer.
- Windows closes CodexUsageBar when installation starts; macOS restarts after replacing the app. The app never silently downloads, replaces, or restarts without confirmation.
- A current build, network error, or signature-verification failure never affects the quota data already on screen. A signature failure cancels installation.

## Privacy and read-only boundary

Only the Rust backend reads local credentials during a request. React receives a filtered snapshot containing status, plan label, refresh time, a stable error code, quota windows, and—when the successful usage response provides it—a masked account email. The frontend translates known error codes for authentication missing / invalid, network, rate limit, service unavailable, invalid response, and local bridge failures, with a safe generic fallback for unknown codes. The raw email is transformed in Rust (for example, `j***@example.com`) before it reaches the UI. React never receives `auth.json`, access tokens, request headers, raw API responses, raw errors, or an unmasked email address.

`auth.json` is searched in this order:

1. Next to the running application executable.
2. `%CODEX_HOME%\auth.json`.
3. Windows: `%USERPROFILE%\.codex\auth.json`; macOS: `~/.codex/auth.json`.

The usage feature only reads `GET https://chatgpt.com/backend-api/wham/usage`; the update feature separately reads the public HTTPS `latest.json` manifest, and the app:

- Does not refresh OAuth tokens or write to `auth.json`.
- Does not query or consume reset cards and never begins an OAuth flow.
- Does not install frontend filesystem or HTTP permissions; sensitive I/O stays in Rust.
- Derives the optional masked account summary only from a successful usage response. It does not read an email from `auth.json`, persist an unmasked email, or write either form of the email to runtime logs.
- Stores trend history in a separate, versioned `usage-history.json` beside `settings.json`. The file contains a local random salt, an irreversible salted account fingerprint, anonymous locally salted quota-window stream keys / durations, reset-cycle timestamps, sample times, and remaining percentages. Raw upstream window IDs are hash inputs only. It never stores an account ID, Token, email, upstream label, raw response, proxy, URL, or authentication path. An account-fingerprint change clears all previous samples before the new account is recorded.
- Exposes only sanitized percentages, timestamps, forecast metadata, and fallback-label metadata through the Settings-only history IPC. The Settings window cannot call the main card's `get_dashboard`, so it never receives the masked account, plan, or live raw snapshot.
- Builds system-notification text from localized fallback window names and quota values only; it never includes the masked account, upstream window label, Token, or raw response.
- Generates diagnostics from a fixed whitelist: schema and app version, platform / architecture, language, theme, refresh interval, consecutive failures, notification status / permission, dashboard status / error code, window count, refresh and update status, and history counts / storage state. It excludes Tokens, accounts, email, paths, proxy settings, URLs, quota percentages, reset times, curve points, raw errors, and logs.
- Grants the Settings WebView clipboard **write-text only**—never clipboard read—and restricts external opening to this repository's Release and new-Issue pages. The main WebView has neither clipboard nor notification-plugin permission; notification permission and test actions go through Settings-only Rust commands.
- Checks updates only against the public HTTPS `latest.json` manifest containing signatures for each platform update payload for `creamtea47/codex-usage-bar`, without sending `auth.json`, tokens, email, or usage data. A payload must match the embedded public key before it can be installed.

If credentials are missing, expired, or rejected, the last successful snapshot remains visible as stale. Sign in to Codex again, then choose top-right **Refresh**.

## Architecture

| Layer | Technology | Responsibility |
| --- | --- | --- |
| Desktop UI | React 19 + TypeScript + Material UI + i18next + Recharts 3 + Vite | Compact bilingual card, lazy-loaded Trends page, themes, sidebar Settings, accessibility, drag and resize interactions |
| Native boundary | Tauri 2 commands, events, and capabilities | Window-scoped IPC, OS notification permission / delivery, write-only clipboard, and repository-only Issue / Release links |
| Data and persistence | Rust + Tokio + Reqwest + Serde | Read-only credentials, fixed refresh and failure backoff, request de-duplication, cached snapshots, versioned local history, forecasting, settings, autostart, and redacted logs |

The complete frontend IPC surface is `get_dashboard`, `refresh_dashboard`, `get_settings`, `save_settings`, `set_notifications_enabled`, `send_test_notification`, `get_usage_history`, `set_history_enabled`, `clear_usage_history`, `get_diagnostics`, `get_autostart`, `set_autostart`, `open_settings_window`, `mark_main_size_manual`, `get_app_update_info`, `check_app_update`, and `install_app_update`. `get_dashboard` and manual refresh are main-window only; notification, history, diagnostics, autostart, and update actions are Settings-window only; `get_settings` is the minimal shared preference read. `dashboard-updated` is emitted only to the main window and `usage-history-updated` only to Settings. Update IPC returns only version summaries and progress; URLs, signatures, raw manifests, credentials, and unmasked account details never cross the Rust boundary.

## Logs and troubleshooting

- Settings and independent main/settings-window placement: `%APPDATA%\com.creamtea47.codexusagebar\settings.json` on Windows; `~/Library/Application Support/com.creamtea47.codexusagebar/settings.json` on macOS.
- Local trend samples: `usage-history.json` beside `settings.json`. It is retained for seven days, capped at 2,500 points per stream and 16 streams, can be paused or cleared in **Trends**, and is never uploaded.
- Runtime logs: `%LOCALAPPDATA%\com.creamtea47.codexusagebar\logs\codex-usage-bar.log` on Windows; `~/Library/Logs/com.creamtea47.codexusagebar/codex-usage-bar.log` on macOS; retained for 14 days.
- Logs contain timestamps, operation results, HTTP status or transport categories, and redacted errors only. Updater logs retain only version numbers and redacted categories such as available version, network, signature, or install—never tokens, Authorization headers, authentication-file contents, update URLs, or signature data.
- On proxy-restricted networks, native requests follow the Windows/macOS system proxy and `HTTP_PROXY` / `HTTPS_PROXY`; proxy addresses and credentials are never written to logs.
- If usage cannot be loaded, first confirm that Codex is signed in, then choose top-right **Refresh**. Automatic failures use the visible 1 / 3 / 5 / 10 / 30 minute retry schedule and return to the selected fixed interval after success.
- Use **Copy sanitized diagnostics** for the whitelist summary. Never submit `auth.json`, a Token, unreviewed logs, credentials, or private account details to an Issue, screenshot, or comment.
- Autostart from the legacy Win32 app is neither migrated nor removed. Disable its old `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` entry after confirming the new app works, otherwise two widgets may start together.

## Development

Development requires Node.js 22.13+ or 24+, pnpm 10, and stable Rust. Windows additionally needs the MSVC toolchain and WebView2 Runtime; macOS needs Xcode Command Line Tools. A global Tauri CLI is not required.

The main card relies on a genuinely transparent native window for its CSS-rounded corners. On macOS, Tauri requires `app.macOSPrivateApi: true` to make the WKWebView background transparent; without it, an opaque white rectangle shows through at all four corners. This private API is not compatible with Mac App Store distribution, but it is compatible with the project's current GitHub Release DMGs.

In-app updates on macOS are allowed only when the executable is running from a standard `.app/Contents/MacOS` bundle. The raw `tauri dev` binary rejects installation before downloading so the updater cannot mistake `target/debug` for the App directory; use a local DMG to test the complete install flow.

On Windows, validate notification app name and icon only from an installed NSIS package. The development executable's notification identity is not considered a release test result.

```powershell
pnpm install --frozen-lockfile
pnpm lint
pnpm test
cargo test --locked --manifest-path src-tauri/Cargo.toml
pnpm tauri dev
```

Build the native package for the current platform with:

The repository automatically merges `tauri.windows.conf.json` or `tauri.macos.conf.json` to select the native installer for the current platform. Ordinary local builds do not have the release key, so use the following CI-equivalent commands to disable release signing and updater artifacts:

```powershell
# Windows: ordinary local bundle (no release key required)
pnpm tauri build --bundles nsis --no-sign --config src-tauri/tauri.unsigned.conf.json

# macOS: ordinary local bundle (no release key required)
pnpm tauri build --bundles dmg --no-sign --config src-tauri/tauri.unsigned.conf.json
```

Output is written below `src-tauri\target\release\bundle\nsis\` or `src-tauri/target/release/bundle/dmg/`.

## CI and releases

Pushes to `main` / `master` and pull requests run frontend lint/tests, Rust tests, and unsigned native bundles on Windows x64, Windows ARM64, Intel Mac, and Apple silicon Mac runners. A `v*` tag reads `TAURI_SIGNING_PRIVATE_KEY` only in tag-build steps from GitHub Actions Secrets, generates Tauri signatures for the four in-app updater payloads, creates a draft Release, and only then publishes the complete update set. macOS DMGs remain manual-install artifacts and are not Apple-notarized:

The current signing key has no password. If a future encrypted key is used, configure `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` in GitHub Actions as well. Stable updater releases must use `vX.Y.Z` tags so prereleases cannot enter the stable update channel.

- `CodexUsageBar-x64-setup.exe`
- `CodexUsageBar-arm64-setup.exe`
- `CodexUsageBar-macos-x64.dmg`
- `CodexUsageBar-macos-arm64.dmg`
- `.sig` files for both Windows installers
- macOS `.app.tar.gz` updater payloads and their `.sig` files
- `latest.json`, the public manifest containing signatures for all four platform update payloads

Example maintainer release:

```powershell
git tag -a v0.3.0 -m "v0.3.0 notifications and local trends"
git push origin master
git push origin v0.3.0
```

## Intentionally excluded

There are no unconfirmed silent downloads or updates, taskbar-docked mode, general legacy-layout migration, cloud history sync, custom trend date ranges, OAuth token refresh, or reset-card operations. Existing installations receive only the one-time compact-height adjustment described above.
