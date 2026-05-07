#pragma once

#include <optional>
#include <string>

struct UsageWindow {
    int usedPercent = 0;
    int remainingPercent = 100;
    int windowSeconds = 0;
    int resetAfterSeconds = 0;
    long long resetAtUnixSeconds = 0;
};

struct UsageSnapshot {
    bool success = false;
    std::wstring email;
    std::wstring planType;
    std::wstring errorMessage;
    UsageWindow fiveHour;
    UsageWindow weekly;
};

struct ReleaseVersionInfo {
    bool success = false;
    std::wstring latestTag;
    std::wstring errorMessage;
};

class CodexUsageFetcher {
public:
    UsageSnapshot Fetch() const;
    ReleaseVersionInfo FetchLatestRelease() const;

private:
    std::wstring ResolveAuthJsonPath() const;
    std::optional<std::string> ReadAccessToken(std::wstring* errorMessage) const;
    std::optional<std::string> LoadFileUtf8(const std::wstring& path, std::wstring* errorMessage) const;
    std::optional<std::string> HttpGetUsageJson(const std::string& accessToken, std::wstring* errorMessage) const;
    std::optional<std::string> HttpGetLatestReleaseJson(std::wstring* errorMessage) const;
    UsageSnapshot ParseUsageJson(const std::string& jsonText, std::wstring* errorMessage) const;
    ReleaseVersionInfo ParseLatestReleaseJson(const std::string& jsonText, std::wstring* errorMessage) const;
};
