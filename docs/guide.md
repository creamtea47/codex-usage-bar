# Technical guide

[Back to README](../README.md)

## Usage

The card tries to load usage data immediately after launch. Use the top-right **Refresh** button to retry immediately, including while an automatic failure-backoff deadline is pending. The adjacent **Settings** button opens a separate window organized into **Display**, **Data & refresh**, **Notifications**, **Trends**, **Quota Auto-Continuation**, **Startup**, and **About & updates**.

**Display** controls language, theme, always-on-top, and position / size lock. **Data & refresh** contains the fixed refresh interval and privacy explanation. **Notifications** contains OS permission, alert rules, quiet hours, and the test action. **Trends** switches among 24 hours, 7 days, rolling 30 days, all history, or a custom range covering any past dates; it also controls local collection and confirmed deletion for the current account. **Quota Auto-Continuation** shows the target reset, next attempt, consumed slots, the latest trigger reason, and separate sanitized results for automatic actions and manual tests. **Startup** controls autostart and close-to-tray behavior. Closing Settings only hides it; closing the main window hides to the tray by default, while tray **Quit** always exits.

Auto-continuation requires the app to remain running in the tray while the computer is awake. Resuming or restarting more than 30 minutes after the target marks that cycle missed; only the latest due slot runs, so earlier slots are never replayed back-to-back. **Test now** always requires a second confirmation, sends once, and never retries.

System notifications remain disabled until you turn them on and grant operating-system permission. The first successful snapshot establishes quota-rule baselines rather than immediately warning about an already-low window; the first observed reset-credit count and every account switch likewise establish a count baseline, and only a later increase for the same account alerts. Within one quota cycle, each enabled low-quota or pace event is sent once; a later `resetAt` starts a new cycle and can produce one reset notification. A reset-credit arrival can be merged with same-refresh quota events into one account-free notification with at most three window entries. Quiet hours use local time in the half-open `[start, end)` interval, including cross-midnight ranges, and suppressed events are not replayed. On Windows, notification name and icon behavior must be judged from an installed NSIS package; development notifications are not a release acceptance result.

Trend collection starts enabled for both new and upgraded installations. Only successful snapshots are sampled, with no age, point-count, or stream-count eviction: the newest 24 hours keep collected samples, days 1–7 use 15-minute buckets, days 7–32 use hourly buckets, and older history keeps the first and last point in each UTC-day bucket while preserving reset-cycle boundaries. **Today's consumption** always covers 00:00 through now in the system's local calendar day, independently of the selected chart range. Forecasting uses recent samples from the current cycle and reports **Collecting**, **Stable**, **Expected to run out before reset**, or **Expected to last until reset**. A reliable exhaustion time is rounded to 15 minutes and is never extrapolated past the quota reset. Turning collection off stops new samples without hiding or deleting existing charts and forecast details; **Clear history** permanently removes the current account's points after confirmation.

**GitHub repository** opens only `https://github.com/creamtea47/codex-usage-bar`. **Copy sanitized diagnostics** writes a whitelist summary to the clipboard. **Report an issue** opens a separate GitHub Issue page and never appends diagnostics; inspect the copied summary before deciding whether to paste it. **View release notes** opens the tag page for a validated `X.Y.Z` application version and otherwise falls back to the repository's Releases page.

The app checks once shortly after launch and then at most every six hours by default; you can turn this off in Settings. Choose **Check for updates** when you want to check immediately:

- When an update is available, a small green update icon appears in the card's upper-right corner. Clicking it opens the update page and confirmation dialog. Only clicking **Download and install** downloads the payload, verifies its signature, and hands it to the native installer.
- Windows closes CodexUsageBar when installation starts; macOS restarts after replacing the app. The app never silently downloads, replaces, or restarts without confirmation.
- A current build, network error, or signature-verification failure never affects the quota data already on screen. A signature failure cancels installation.

## Privacy and feature boundary

Only the Rust backend reads local credentials during a request. Usage, trends, and notifications remain read-only; auto-continuation sends a real request only after explicit opt-in or test confirmation. React receives filtered usage and sanitized continuation status, never `auth.json`, access tokens, request headers, raw API responses, model replies, raw errors, or an unmasked email address.

`auth.json` is searched in this order:

1. Next to the running application executable.
2. `%CODEX_HOME%\auth.json`.
3. Windows: `%USERPROFILE%\.codex\auth.json`; macOS: `~/.codex/auth.json`.

The usage feature only reads `GET https://chatgpt.com/backend-api/wham/usage`; the update feature separately reads the public HTTPS `latest.json` manifest, and the app:

- Never reads refresh tokens, refreshes OAuth tokens, or writes to `auth.json`; every attempt rereads the latest access token.
- Auto-continuation first refreshes usage read-only to verify the same reset event and account. An advanced `reset_at` alone is no longer a reason to skip; only an automatic or manual success already recorded for the same event prevents a later send. When no same-event success exists, it reads the live Codex model manifest and sends fixed `hi` to `POST https://chatgpt.com/backend-api/codex/responses` with `stream: true` and `store: false`. Only an SSE `response.completed` event counts as success; response text is neither displayed nor stored.
- Stores auto-continuation runtime state in a separate versioned `quota-auto-continue.json` beside `settings.json`. Schema v2 contains only a local salt, salted account/window fingerprints, the active cycle and pending next-cycle time, the latest quota observation, opaque `eventId` / `generationId`, notification disposition, the 30-minute event lock, consumed slots, completion marker, timestamps, trigger reason, and separate sanitized automatic/manual result summaries. It never stores tokens, account IDs, email, request headers, response bodies, or model replies. A slot is atomically persisted before sending, so crash recovery cannot repeat it.
- Writes a daily sanitized `quota-audit-YYYY-MM-DD.jsonl` beside `settings.json` and retains it for 14 days. Each record contains only a UTC timestamp, fixed level/action codes, opaque event/generation IDs, trigger reason, old/new integer remaining percentages and `reset_at` values, a zero-based slot index, and a fixed error code; it excludes credentials, account IDs or email, paths or URLs, raw quota responses, prompt/reply text, and arbitrary error text.
- Makes no additional reset-credit request and never calls a reset-credit consumption or other write endpoint. It only reads the total and currently usable reset-credit summary already attached to the existing `wham/usage` response, and never begins an OAuth flow.
- Stores the reset-credit notification baseline in a separate versioned `reset-credit-notification.json` beside `settings.json`. It contains only the schema version, a local random salt, a salted account fingerprint, and the latest total count; it never stores tokens, account IDs, email, the currently usable count, or the raw response.
- Does not install frontend filesystem or HTTP permissions; sensitive I/O stays in Rust.
- Derives the optional masked account summary only from a successful usage response. It does not read an email from `auth.json`, persist an unmasked email, or write either form of the email to runtime logs.
- Stores trend history in a separate, versioned `usage-history.json` beside `settings.json`. The file contains a local random salt, irreversible salted account fingerprints, anonymous account partitions, anonymous locally salted quota-window stream keys / durations, reset-cycle timestamps, sample times, and remaining percentages. Raw upstream window IDs are hash inputs only. It never stores an account ID, Token, email, upstream label, raw response, proxy, URL, or authentication path. Switching accounts changes the active partition without deleting other account history.
- Exposes only sanitized percentages, timestamps, forecast metadata, and fallback-label metadata through the Settings-only history IPC. The Settings window cannot call the main card's `get_dashboard`, so it never receives the masked account, plan, or live raw snapshot.
- Builds system-notification text only from localized fallback window names, quota values, and reset-credit gained/total/currently-usable counts; it never includes the masked account, upstream window label, Token, or raw response.
- Generates diagnostics from a fixed whitelist: schema/app/platform fields, refresh and notification status, dashboard/update/history summaries, the close-to-tray setting, and the continuation enabled flag/fixed enum phase. It excludes tokens, accounts, email, paths, proxy settings, URLs, quota percentages, reset or attempt times, curve points, raw errors, and logs.
- Grants the Settings WebView clipboard **write-text only**—never clipboard read—and restricts external opening to the exact repository root, its Release pages, and its new-Issue page. No wildcard is granted for the repository root. The main WebView has neither clipboard nor notification-plugin permission; notification permission and test actions go through Settings-only Rust commands.
- Checks updates only against the public HTTPS `latest.json` manifest containing signatures for each platform update payload for `creamtea47/codex-usage-bar`, without sending `auth.json`, tokens, email, or usage data. A payload must match the embedded public key before it can be installed.

If credentials are missing, expired, or rejected, the last successful snapshot remains visible as stale. Sign in to Codex again, then choose top-right **Refresh**.

## Architecture

| Layer | Technology | Responsibility |
| --- | --- | --- |
| Desktop UI | React 19 + TypeScript + Material UI + i18next + Recharts 3 + Vite | Compact bilingual card, lazy-loaded Trends page, themes, sidebar Settings, accessibility, drag and resize interactions |
| Native boundary | Tauri 2 commands, events, and capabilities | Window-scoped IPC, OS notification permission / delivery, write-only clipboard, and exact repository / Issue / Release links |
| Data and persistence | Rust + Tokio + Reqwest + Serde | Read-only usage by default, opt-in minimal continuation requests, fixed refresh/backoff, request de-duplication, versioned sanitized runtime state, local history, forecasting, settings, autostart, and redacted logs |

The frontend IPC also includes Settings-only `get_quota_auto_continue_status`, `set_quota_auto_continue_enabled`, and `test_quota_auto_continue`, with sanitized updates on `quota-auto-continue-updated`. Ordinary `save_settings` preserves the current continuation flag and cannot bypass the dedicated side-effect command. Dashboard/manual refresh remain main-window only; notification, history, continuation, diagnostics, autostart, and update actions are Settings-window only. URLs, signatures, raw manifests, credentials, and unmasked account details never cross the Rust boundary.

## Logs and troubleshooting

- Settings and independent main/settings-window placement: `%APPDATA%\com.creamtea47.codexusagebar\settings.json` on Windows; `~/Library/Application Support/com.creamtea47.codexusagebar/settings.json` on macOS.
- Sanitized auto-continuation state: `quota-auto-continue.json` beside `settings.json`. Disabling the feature does not delete it; re-enabling first validates it against the latest current-account snapshot.
- Reset-credit notification baseline: `reset-credit-notification.json` beside `settings.json`. It stores only a sanitized account fingerprint and the latest total count to detect arrivals across restarts, never the currently usable count or account plaintext.
- Local trend samples: `usage-history.json` beside `settings.json`. Samples are not evicted by age, point count, or stream count; each queried series still displays at most 1,000 points. The newest 24 hours keep collected samples, samples from 1–7 days use 15-minute buckets, samples from 7–32 days use hourly buckets, and older history keeps the first and last point per UTC-day bucket while preserving reset boundaries. Schema v1 migrates losslessly to multi-account schema v2 with a one-time pre-rewrite backup. Collection can be paused or the current account cleared in **Trends**, and data is never uploaded; time before the upgrade is reported as partial coverage when no samples exist.
- Runtime logs: `%LOCALAPPDATA%\com.creamtea47.codexusagebar\logs\codex-usage-bar.log` on Windows; `~/Library/Logs/com.creamtea47.codexusagebar/codex-usage-bar.log` on macOS; retained for 14 days.
- Logs contain timestamps, levels, task deadlines, attempt numbers, selected model names, operation results, and sanitized result categories. They never contain tokens, account identifiers, Authorization headers, authentication-file contents, raw SSE, response bodies, or model replies.
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
git tag -a v0.6.2 -m "v0.6.2 permanent history, account partitions, and single-instance fix"
git push origin master
git push origin v0.6.2
```

## Intentionally excluded

There are no unconfirmed silent downloads or updates, taskbar-docked mode, general legacy-layout migration, cloud history sync, cross-account history selector, custom continuation prompts, OAuth token refresh, or OS-level forced wake. Existing installations receive only the one-time compact-height adjustment described above.
