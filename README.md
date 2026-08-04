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

![CodexUsageBar macOS Settings window with a dark native title bar and the v0.2.7 About and updates page showing automatic checks and the manual update entry point](docs/images/settings-update.png)

## Features

- Opens as a compact `460 × 260` floating card for one quota window, removing unused bottom space. It expands only when additional windows need room (up to `560px`), and scrolls only the quota area beyond that limit. A manually resized card keeps its chosen size.
- Displays every quota window returned by the usage API, its remaining percentage, reset time, and local countdown.
- Shows a masked account summary such as `j***@example.com · Pro` after a successful refresh, together with the next automatic-refresh countdown and most recent refresh time.
- Refreshes at launch and on a configurable 1 / 3 / 5 / 10 / 30 minute interval; only the local countdown changes between requests.
- Keeps **Refresh**, **Settings**, and **Close** in the upper-right corner. A green update icon appears when a new version is available and opens the user-confirmed install flow. Refresh is disabled while a request is running to prevent duplicate requests.
- Draws a pace marker for long quota windows. Its tooltip explains that the marker is the suggested minimum remaining quota calculated from elapsed window time; it is a local pacing estimate, not an official limit.
- Uses a borderless, draggable, resizable desktop card with system, light, and dark themes; supports always-on-top and position / size lock, and responds to the first click while unfocused on macOS.
- Closes completely with its window—no tray-resident process. Autostart for the current account is optional.
- Opens Settings in a separate opaque native window with sidebar categories: **Display**, **Data & refresh**, **Startup**, and **About & updates**.
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

The card tries to load usage data immediately after launch. Use the top-right **Refresh** button to retry. The adjacent **Settings** button opens a separate window where settings are organized into **Display**, **Data & refresh**, **Startup**, and **About & updates**.

The Display page controls the theme, always-on-top, and position / size lock. The Data & refresh page controls the refresh interval; Startup controls autostart; About & updates contains the current version, automatic-check preference, and update actions. Closing Settings only hides that window—the floating card remains running. Closing the floating card exits the application.

The app checks once shortly after launch and then at most every six hours by default; you can turn this off in Settings. Choose **Check for updates** when you want to check immediately:

- When an update is available, a small green update icon appears in the card's upper-right corner. Clicking it opens the update page and confirmation dialog. Only clicking **Download and install** downloads the payload, verifies its signature, and hands it to the native installer.
- Windows closes CodexUsageBar when installation starts; macOS restarts after replacing the app. The app never silently downloads, replaces, or restarts without confirmation.
- A current build, network error, or signature-verification failure never affects the quota data already on screen. A signature failure cancels installation.

## Privacy and read-only boundary

Only the Rust backend reads local credentials during a request. React receives a filtered snapshot containing status, plan label, refresh time, a safe error message, quota windows, and—when the successful usage response provides it—a masked account email. The raw email is transformed in Rust (for example, `j***@example.com`) before it reaches the UI. React never receives `auth.json`, access tokens, request headers, raw API responses, or an unmasked email address.

`auth.json` is searched in this order:

1. Next to the running application executable.
2. `%CODEX_HOME%\auth.json`.
3. Windows: `%USERPROFILE%\.codex\auth.json`; macOS: `~/.codex/auth.json`.

The usage feature only reads `GET https://chatgpt.com/backend-api/wham/usage`; the update feature separately reads the public HTTPS `latest.json` manifest, and the app:

- Does not refresh OAuth tokens or write to `auth.json`.
- Does not query or consume reset cards and never begins an OAuth flow.
- Does not install frontend filesystem or HTTP permissions; sensitive I/O stays in Rust.
- Derives the optional masked account summary only from a successful usage response. It does not read an email from `auth.json`, persist an unmasked email, or write either form of the email to runtime logs.
- Checks updates only against the public HTTPS `latest.json` manifest containing signatures for each platform update payload for `creamtea47/codex-usage-bar`, without sending `auth.json`, tokens, email, or usage data. A payload must match the embedded public key before it can be installed.

If credentials are missing, expired, or rejected, the last successful snapshot remains visible as stale. Sign in to Codex again, then choose top-right **Refresh**.

## Architecture

| Layer | Technology | Responsibility |
| --- | --- | --- |
| Desktop UI | React + TypeScript + Material UI + Vite | Compact Chinese usage card, themes, standalone sidebar settings window, drag and resize interactions |
| Native boundary | Tauri 2 commands and capabilities | Narrow IPC and scoped window / release-link permissions |
| Data and persistence | Rust + Tokio + Reqwest + Serde | Read-only credentials, request de-duplication, cached snapshots, settings, autostart, redacted logs |

The complete frontend IPC surface is `get_dashboard`, `refresh_dashboard`, `get_settings`, `save_settings`, `get_autostart`, `set_autostart`, `open_settings_window`, `mark_main_size_manual`, `get_app_update_info`, `check_app_update`, and `install_app_update`. Update IPC returns only version summaries and progress; URLs, signatures, raw manifests, credentials, and unmasked account details never cross the Rust boundary.

## Logs and troubleshooting

- Settings and independent main/settings-window placement: `%APPDATA%\com.creamtea47.codexusagebar\settings.json` on Windows; `~/Library/Application Support/com.creamtea47.codexusagebar/settings.json` on macOS.
- Runtime logs: `%LOCALAPPDATA%\com.creamtea47.codexusagebar\logs\codex-usage-bar.log` on Windows; `~/Library/Logs/com.creamtea47.codexusagebar/codex-usage-bar.log` on macOS; retained for 14 days.
- Logs contain timestamps, operation results, HTTP status or transport categories, and redacted errors only. Updater logs retain only version numbers and redacted categories such as available version, network, signature, or install—never tokens, Authorization headers, authentication-file contents, update URLs, or signature data.
- On proxy-restricted networks, native requests follow the Windows/macOS system proxy and `HTTP_PROXY` / `HTTPS_PROXY`; proxy addresses and credentials are never written to logs.
- If usage cannot be loaded, first confirm that Codex is signed in, then choose top-right **Refresh**. Never paste credentials into an issue, screenshot, or log.
- Autostart from the legacy Win32 app is neither migrated nor removed. Disable its old `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` entry after confirming the new app works, otherwise two widgets may start together.

## Development

Development requires Node.js 22.13+ or 24+, pnpm 10, and stable Rust. Windows additionally needs the MSVC toolchain and WebView2 Runtime; macOS needs Xcode Command Line Tools. A global Tauri CLI is not required.

The main card relies on a genuinely transparent native window for its CSS-rounded corners. On macOS, Tauri requires `app.macOSPrivateApi: true` to make the WKWebView background transparent; without it, an opaque white rectangle shows through at all four corners. This private API is not compatible with Mac App Store distribution, but it is compatible with the project's current GitHub Release DMGs.

In-app updates on macOS are allowed only when the executable is running from a standard `.app/Contents/MacOS` bundle. The raw `tauri dev` binary rejects installation before downloading so the updater cannot mistake `target/debug` for the App directory; use a local DMG to test the complete install flow.

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
git tag -a v0.2.7 -m "v0.2.7 macOS compatibility and update indicator"
git push origin master
git push origin v0.2.7
```

## Intentionally excluded

There are no unconfirmed silent downloads or updates, taskbar-docked mode, general legacy-layout migration, UI language switching, OAuth token refresh, or reset-card operations. Existing installations receive only the one-time compact-height adjustment described above.
