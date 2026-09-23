# Technical guide

[Back to README](../README.md)

## Usage

The card tries to load usage data immediately after launch. Use the top-right **Refresh** button to retry immediately, including while an automatic failure-backoff deadline is pending. The adjacent **Settings** button opens a separate window organized into **Accounts**, **Display**, **Data & refresh**, **Notifications**, **Trends**, **Quota Auto-Continuation**, **Startup**, and **About & updates**.

**Display** controls language, theme, always-on-top, and position / size lock. **Data & refresh** contains the fixed refresh interval and privacy explanation. **Notifications** contains OS permission, alert rules, quiet hours, and the test action. **Trends** switches among 24 hours, 7 days, rolling 30 days, all history, or a custom range covering any past dates; it also controls local collection and confirmed deletion for the current account. **Quota Auto-Continuation** shows the target reset, next attempt, consumed slots, the latest trigger reason, and separate sanitized results for automatic actions and manual tests. **Startup** controls autostart and close-to-tray behavior. Closing Settings only hides it; closing the main window hides to the tray by default, while tray **Quit** always exits.

Auto-continuation requires the app to remain running in the tray while the computer is awake. Resuming or restarting more than 30 minutes after the target marks that cycle missed; only the latest due slot runs, so earlier slots are never replayed back-to-back. **Test now** always requires a second confirmation, sends once, and never retries.

System notifications remain disabled until you turn them on and grant operating-system permission. The first successful snapshot establishes quota-rule baselines rather than immediately warning about an already-low window; the first observed reset-credit count and every account switch likewise establish a count baseline, and only a later increase for the same account alerts. Within one quota cycle, each enabled low-quota or pace event is sent once; a later `resetAt` starts a new cycle and can produce one reset notification. A reset-credit arrival can be merged with same-refresh quota events into one notification labeled with the account note with at most three window entries. Quiet hours use local time in the half-open `[start, end)` interval, including cross-midnight ranges, and suppressed events are not replayed. On Windows, notification name and icon behavior must be judged from an installed NSIS package; development notifications are not a release acceptance result.

Trend collection starts enabled for both new and upgraded installations. Only successful snapshots are sampled, with no age, point-count, or stream-count eviction: the newest 24 hours keep collected samples, days 1–7 use 15-minute buckets, days 7–32 use hourly buckets, and older history keeps the first and last point in each UTC-day bucket while preserving reset-cycle boundaries. **Today's consumption** always covers 00:00 through now in the system's local calendar day, independently of the selected chart range. Forecasting uses recent samples from the current cycle and reports **Collecting**, **Stable**, **Expected to run out before reset**, or **Expected to last until reset**. A reliable exhaustion time is rounded to 15 minutes and is never extrapolated past the quota reset. Turning collection off stops new samples without hiding or deleting existing charts and forecast details; **Clear history** permanently removes the current account's points after confirmation.

**GitHub repository** opens only `https://github.com/creamtea47/codex-usage-bar`. **Copy sanitized diagnostics** writes a whitelist summary to the clipboard. **Report an issue** opens a separate GitHub Issue page and never appends diagnostics; inspect the copied summary before deciding whether to paste it. **View release notes** opens the tag page for a validated `X.Y.Z` application version and otherwise falls back to the repository's Releases page.

The app checks once shortly after launch and then at most every six hours by default; you can turn this off in Settings. Choose **Check for updates** when you want to check immediately:

- When an update is available, a small green update icon appears in the card's upper-right corner. Clicking it opens the update page and confirmation dialog. Only clicking **Download and install** downloads the payload, verifies its signature, and hands it to the native installer.
- Windows closes CodexUsageBar when installation starts; macOS restarts after replacing the app. The app never silently downloads, replaces, or restarts without confirmation.
- A current build, network error, or signature-verification failure never affects the quota data already on screen. A signature failure cancels installation.

## Account cards and viewing selection

Cards use the full email as identity, with a plan chip, name/note, quota progress, and subscription date labeled as a credential record. Past subscription snapshots require verification. Expand account details for login method, user/account IDs, token expiry and the managed path; IDs and paths can be copied.

The viewing selector stays at the bottom of Settings navigation. The main window has an account icon between Refresh and Settings, with a masked-email menu. Neither selector changes the Codex login file. Codex login source re-reads the file and matches its identity independently of saved bindings or viewed accounts. This is a disk-file observation, not proof of a running session cache.

Full emails, IDs and paths are returned only to the Settings window through get_accounts. Other windows omit details and codexLogin, and logs exclude this information. Existing usage responses and credentials supply the data without extra network requests; failures retain the latest successful information.

Reset credits show held and currently usable counts separately. The Accounts page loads their expiry details through a separate Settings-only GET, cached per account for five minutes and invalidated by held-count changes. The nearest expiry is always visible; the tooltip lists each expiry in local time. Missing dates stay unknown, and failed reads preserve previous details with a retry action. No credit redemption operation is provided.

## Accounts, range statistics and responses

Use **Accounts** to import multiple auth.json files, edit notes, pause monitoring and remove managed copies. Every enabled account refreshes independently; the shared selector chooses the account shown in the card and Settings. Removing managed credentials preserves history and does not sign Codex out.

Click two chart points for interval duration and consumption; reverse selection works. Arrow keys move, Enter selects, Escape clears. Consumption is calculated from retained backend observations separately for each reset cycle: 80→20 followed by 100→70 after reset totals 90 percentage points. It is not token or monetary usage. Expand each cycle to page through 50 retained samples; partial coverage and historical precision are labeled. Current forecasts show time to exhaustion and time before reset. Past exhaustion is shown only when zero was observed, with the first-zero summary preserved during compaction.

Each account selects an automatic or explicit model using a list or editable ID. There is no fixed old-model fallback. The latest automatic and manual results separately retain requested/reported model, HTTP status, duration, up to 8192 characters of text and sanitized errors. Only a completed event counts as success; uncertain delivery stops automatic replay. Legacy results indicate that text was not saved.

The main window uses a short forecast line, with relative exhaustion and early-exhaustion durations in the tooltip. Trends and cycle rows retain the detailed forecast. Automatic height uses a 260px base plus 20px per visible advice row and section spacing; manually adjusted sizes retain control.

## Privacy and feature boundary

- Rust handles credential files, native import dialogs, requests and writes. WebViews never receive tokens, authentication files, request headers or full upstream JSON.
- Initial discovery uses the executable directory, CODEX_HOME/auth.json, then home .codex/auth.json. Imports copy credentials into the restricted application configuration directory at `accounts/<local-account-key>/auth.json`; ordinary imports leave the source untouched. Windows uses a restricted ACL; Unix uses 700/600 directory/file permissions.
- Managed accounts refresh OAuth tokens within thirty minutes of expiry and atomically save rotated credentials. Access-only files work until expiry. For an account bound to Codex, Codex owns refresh and the tool reads back the latest file. Switching away synchronizes credentials before resuming managed refresh.
- Applying/restoring Codex credentials is a separate Windows file-store action. Close Codex desktop, CLI and IDE sessions first, then reopen manually after replacement. Credentials are backed up and verified. Existing config.toml and custom-provider routing remain unchanged.
- Account notes, monitoring and model choices are stored in accounts/accounts.json. Imported default emails are masked. Credential copies remain sensitive and must not be shared or committed.
- Usage reads wham/usage. Auto-continuation sends fixed hi to Codex Responses only after opt-in or confirmed testing, with stream=true and store=false. It never automatically consumes reset credits or starts a browser OAuth login.
- Per-account state, latest replies and reset-credit baselines live under accounts/<local-account-key>/. Legacy state migrates only when salted identity matches; unmatched originals remain. Replies are excluded from runtime/audit logs.
- The original usage-history.json retains its salt and anonymous account partitions. Disabling collection keeps history; clearing affects the selected account only. Older samples remain compacted at the existing age tiers.
- Notifications are deduplicated per account and include its note. Logs retain timestamps, levels, anonymous account keys, actions and fixed error categories for fourteen days, excluding credentials and reply text.
- Existing sanitized diagnostics, write-only clipboard access, constrained external links and signed-update checks remain in place. Update requests contain no account credentials.

Authentication failures retain the last successful snapshot as stale. Accounts distinguishes network errors, Codex-managed refresh, missing refresh tokens and required reimport.

## Architecture

| Layer | Technology | Responsibility |
| --- | --- | --- |
| Desktop UI | React 19 + TypeScript + Material UI + i18next + Recharts 3 + Vite | Compact bilingual card, lazy-loaded Trends page, themes, sidebar Settings, accessibility, drag and resize interactions |
| Native boundary | Tauri 2 commands, events, and capabilities | Window-scoped IPC, OS notification permission / delivery, write-only clipboard, and exact repository / Issue / Release links |
| Data and persistence | Rust + Tokio + Reqwest + Serde | Read-only usage by default, opt-in minimal continuation requests, fixed refresh/backoff, request de-duplication, versioned sanitized runtime state, local history, forecasting, settings, autostart, and redacted logs |

The frontend IPC also includes Settings-only `get_quota_auto_continue_status`, `set_quota_auto_continue_enabled`, and `test_quota_auto_continue`, with sanitized updates on `quota-auto-continue-updated`. Ordinary `save_settings` preserves the current continuation flag and cannot bypass the dedicated side-effect command. Dashboard/manual refresh remain main-window only; notification, history, continuation, diagnostics, autostart, and update actions are Settings-window only. Updater URLs/signatures, raw manifests and credentials never cross the Rust boundary; allowlisted full account details are restricted to Settings.

## Logs and troubleshooting

- Settings and independent main/settings-window placement: `%APPDATA%\com.creamtea47.codexusagebar\settings.json` on Windows; `~/Library/Application Support/com.creamtea47.codexusagebar/settings.json` on macOS.
- Auto-continuation state and latest retained replies: `accounts/<local-account-key>/quota-auto-continue.json`. Disabling the feature does not delete it; re-enabling first validates it against the latest account snapshot. Replies are not copied into logs.
- Reset-credit notification baseline: `accounts/<local-account-key>/reset-credit-notification.json`. It stores only a sanitized account fingerprint and the latest total count to detect arrivals across restarts, never the currently usable count or account plaintext.
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
git tag -a v0.7.0 -m "v0.7.0 account management, range statistics, and reset credit expiry"
git push origin master
git push origin v0.7.0
```

## Intentionally excluded

There are no unconfirmed silent downloads or updates, taskbar-docked mode, general legacy-layout migration, cloud history sync, custom continuation prompts, or OS-level forced wake. Automatic window sizing follows the compact layout; manual sizes remain user-controlled.
