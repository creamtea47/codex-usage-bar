#pragma once

#include <optional>
#include <string>
#include <vector>

struct UsageWindow {
    // False when the API omits this window (e.g. 5h primary removed).
    bool available = false;
    int usedPercent = 0;
    int remainingPercent = 100;
    int windowSeconds = 0;
    int resetAfterSeconds = 0;
    long long resetAtUnixSeconds = 0;
    // Derived: resetAt - windowSeconds when both are valid.
    long long startAtUnixSeconds = 0;
    bool hasStartAt = false;
};

struct RateLimitResetCredit {
    std::wstring id;
    std::wstring resetType;
    std::wstring status;
    long long grantedAtUnixSeconds = 0;
    // 0 means no expiry / unknown.
    long long expiresAtUnixSeconds = 0;
    bool hasExpiry = false;
    std::wstring title;
    std::wstring description;
};

struct RateLimitResetCreditsInfo {
    bool fetched = false;
    int availableCount = 0;
    std::wstring errorMessage;
    // Still-available inventory after local filtering (status + expiry).
    std::vector<RateLimitResetCredit> availableCredits;
    // Earliest expiring available credit; 0 / false if none or no expiry.
    long long nextExpiresAtUnixSeconds = 0;
    bool hasNextExpiry = false;
};

struct UsageSnapshot {
    bool success = false;
    std::wstring email;
    std::wstring planType;
    // From id_token chatgpt_subscription_active_start / until when present.
    long long planStartUnixSeconds = 0;
    long long planUntilUnixSeconds = 0;
    bool hasPlanStart = false;
    bool hasPlanUntil = false;
    std::wstring errorMessage;
    UsageWindow fiveHour;
    UsageWindow weekly;
    RateLimitResetCreditsInfo resetCredits;
};

struct ReleaseVersionInfo {
    bool success = false;
    std::wstring latestTag;
    std::wstring errorMessage;
};

struct ConsumeResetCreditResult {
    bool success = false;
    std::wstring errorMessage;
};

class CodexUsageFetcher {
public:
    struct AuthCredentials {
        std::string accessToken;
        std::string accountId;
        std::string idToken;
    };

    UsageSnapshot Fetch() const;
    ReleaseVersionInfo FetchLatestRelease() const;

    // Spends one real rate-limit reset credit. Do not call casually.
    ConsumeResetCreditResult ConsumeRateLimitResetCredit(const std::wstring& redeemRequestId) const;

private:
    std::wstring ResolveAuthJsonPath() const;
    std::optional<AuthCredentials> ReadAuthCredentials(std::wstring* errorMessage) const;
    std::optional<std::string> LoadFileUtf8(const std::wstring& path, std::wstring* errorMessage) const;
    std::optional<std::string> HttpGetUsageJson(const AuthCredentials& credentials, std::wstring* errorMessage) const;
    std::optional<std::string> HttpGetRateLimitResetCreditsJson(
        const AuthCredentials& credentials,
        std::wstring* errorMessage) const;
    std::optional<std::string> HttpGetLatestReleaseJson(std::wstring* errorMessage) const;
    bool HttpPostConsumeRateLimitResetCredit(
        const AuthCredentials& credentials,
        const std::wstring& redeemRequestId,
        std::wstring* errorMessage) const;
    UsageSnapshot ParseUsageJson(const std::string& jsonText, std::wstring* errorMessage) const;
    void EnrichSubscriptionFromIdToken(UsageSnapshot* snapshot, const std::string& idToken) const;
    RateLimitResetCreditsInfo ParseRateLimitResetCreditsJson(const std::string& jsonText, std::wstring* errorMessage) const;
    ReleaseVersionInfo ParseLatestReleaseJson(const std::string& jsonText, std::wstring* errorMessage) const;
};
