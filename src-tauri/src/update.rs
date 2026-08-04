use crate::models::UpdateInfo;
use chrono::{DateTime, Utc};
use reqwest::{Client, StatusCode};
use semver::Version;
use serde::Deserialize;
use std::time::Duration;

const LATEST_RELEASE_ENDPOINT: &str =
    "https://api.github.com/repos/creamtea47/codex-usage-bar/releases/latest";
const RELEASE_PAGE_PREFIX: &str = "https://github.com/creamtea47/codex-usage-bar/releases/tag/";
const ASSET_DOWNLOAD_PREFIX: &str =
    "https://github.com/creamtea47/codex-usage-bar/releases/download/";

#[derive(Debug, thiserror::Error)]
pub enum UpdateError {
    #[error("无法建立安全的更新检查请求。")]
    Client,
    #[error("暂未发布可供检查的正式版本。")]
    NoRelease,
    #[error("暂时无法检查更新，请稍后重试。")]
    Network,
    #[error("更新服务暂时不可用，请稍后重试。")]
    Server,
    #[error("收到的发布信息无效，请稍后再试。")]
    InvalidRelease,
}

#[derive(Clone)]
pub struct UpdateClient {
    client: Client,
}

#[derive(Debug, Deserialize)]
struct GithubRelease {
    tag_name: String,
    html_url: String,
    #[serde(default)]
    published_at: Option<DateTime<Utc>>,
    #[serde(default)]
    assets: Vec<GithubAsset>,
}

#[derive(Debug, Deserialize)]
struct GithubAsset {
    name: String,
    browser_download_url: String,
}

impl UpdateClient {
    pub fn new() -> Result<Self, UpdateError> {
        let client = Client::builder()
            .user_agent(format!(
                "CodexUsageBar/{} (update-check)",
                env!("CARGO_PKG_VERSION")
            ))
            .connect_timeout(Duration::from_secs(8))
            .timeout(Duration::from_secs(12))
            .build()
            .map_err(|_| UpdateError::Client)?;
        Ok(Self { client })
    }

    /// 只读取公开 GitHub Release，不携带 auth.json、Token 或任何用户身份信息。
    pub async fn check(&self) -> Result<UpdateInfo, UpdateError> {
        let response = self
            .client
            .get(LATEST_RELEASE_ENDPOINT)
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .send()
            .await
            .map_err(|_| UpdateError::Network)?;
        let status = response.status();
        if !status.is_success() {
            return Err(status_to_error(status));
        }
        let release = response
            .json::<GithubRelease>()
            .await
            .map_err(|_| UpdateError::InvalidRelease)?;
        parse_latest_release(
            release,
            env!("CARGO_PKG_VERSION"),
            std::env::consts::OS,
            std::env::consts::ARCH,
        )
    }
}

fn status_to_error(status: StatusCode) -> UpdateError {
    match status {
        StatusCode::NOT_FOUND => UpdateError::NoRelease,
        value if value.is_server_error() => UpdateError::Server,
        _ => UpdateError::Network,
    }
}

fn parse_latest_release(
    release: GithubRelease,
    current_version: &str,
    operating_system: &str,
    architecture: &str,
) -> Result<UpdateInfo, UpdateError> {
    let current_version = parse_version(current_version)?;
    let latest_version = parse_version(&release.tag_name)?;
    if !release.html_url.starts_with(RELEASE_PAGE_PREFIX) {
        return Err(UpdateError::InvalidRelease);
    }

    let expected_asset = installer_asset_name(operating_system, architecture);
    let download_url = expected_asset.and_then(|expected_asset| {
        release
            .assets
            .iter()
            .find(|asset| {
                asset.name == expected_asset
                    && asset
                        .browser_download_url
                        .starts_with(ASSET_DOWNLOAD_PREFIX)
            })
            .map(|asset| asset.browser_download_url.clone())
    });

    Ok(UpdateInfo {
        current_version: current_version.to_string(),
        latest_version: latest_version.to_string(),
        update_available: latest_version > current_version,
        release_url: release.html_url,
        download_url,
        published_at: release.published_at,
    })
}

fn parse_version(value: &str) -> Result<Version, UpdateError> {
    Version::parse(value.trim().trim_start_matches('v')).map_err(|_| UpdateError::InvalidRelease)
}

/// 只向当前系统显示可直接安装的资产，避免把 Windows EXE 推荐给 macOS 用户。
fn installer_asset_name(operating_system: &str, architecture: &str) -> Option<&'static str> {
    match (operating_system, architecture) {
        ("windows", "x86_64") => Some("CodexUsageBar-x64-setup.exe"),
        ("windows", "aarch64") => Some("CodexUsageBar-arm64-setup.exe"),
        ("macos", "x86_64") => Some("CodexUsageBar-macos-x64.dmg"),
        ("macos", "aarch64") => Some("CodexUsageBar-macos-arm64.dmg"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release(tag_name: &str, asset_name: &str, asset_url: &str) -> GithubRelease {
        GithubRelease {
            tag_name: tag_name.to_owned(),
            html_url: format!("{RELEASE_PAGE_PREFIX}{tag_name}"),
            published_at: Some("2030-01-04T12:00:00Z".parse().unwrap()),
            assets: vec![GithubAsset {
                name: asset_name.to_owned(),
                browser_download_url: asset_url.to_owned(),
            }],
        }
    }

    #[test]
    fn detects_a_newer_release_and_selects_the_matching_installer() {
        let info = parse_latest_release(
            release(
                "v0.2.1",
                "CodexUsageBar-x64-setup.exe",
                "https://github.com/creamtea47/codex-usage-bar/releases/download/v0.2.1/CodexUsageBar-x64-setup.exe",
            ),
            "0.2.0",
            "windows",
            "x86_64",
        )
        .unwrap();

        assert!(info.update_available);
        assert_eq!(info.current_version, "0.2.0");
        assert_eq!(info.latest_version, "0.2.1");
        assert!(info.download_url.unwrap().ends_with("x64-setup.exe"));
    }

    #[test]
    fn does_not_offer_an_older_release_as_an_update() {
        let info = parse_latest_release(
            release("v0.1.19", "CodexUsageBar-x64-setup.exe", "https://github.com/creamtea47/codex-usage-bar/releases/download/v0.1.19/CodexUsageBar-x64-setup.exe"),
            "0.2.1",
            "windows",
            "x86_64",
        )
        .unwrap();

        assert!(!info.update_available);
    }

    #[test]
    fn ignores_untrusted_or_wrong_architecture_assets() {
        let info = parse_latest_release(
            release(
                "v0.2.1",
                "CodexUsageBar-arm64-setup.exe",
                "https://example.invalid/setup.exe",
            ),
            "0.2.0",
            "windows",
            "x86_64",
        )
        .unwrap();

        assert_eq!(info.download_url, None);
    }

    #[test]
    fn rejects_invalid_release_versions() {
        assert!(matches!(
            parse_latest_release(
                release(
                    "latest",
                    "CodexUsageBar-x64-setup.exe",
                    "https://example.invalid/setup.exe",
                ),
                "0.2.0",
                "windows",
                "x86_64",
            ),
            Err(UpdateError::InvalidRelease)
        ));
    }

    #[test]
    fn selects_a_dmg_for_apple_silicon_macos() {
        let info = parse_latest_release(
            release(
                "v0.2.2",
                "CodexUsageBar-macos-arm64.dmg",
                "https://github.com/creamtea47/codex-usage-bar/releases/download/v0.2.2/CodexUsageBar-macos-arm64.dmg",
            ),
            "0.2.1",
            "macos",
            "aarch64",
        )
        .unwrap();

        assert!(info.update_available);
        assert!(info.download_url.unwrap().ends_with("macos-arm64.dmg"));
    }
}
