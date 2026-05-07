#include "CodexUsageFetcher.h"

#include "JsonLite.h"

#include <Windows.h>
#include <Shlwapi.h>
#include <winhttp.h>

#include <algorithm>
#include <filesystem>
#include <fstream>
#include <sstream>
#include <vector>

#pragma comment(lib, "winhttp.lib")
#pragma comment(lib, "shlwapi.lib")

namespace {

std::wstring Utf8ToWide(const std::string& input) {
    if (input.empty()) {
        return {};
    }

    const int size = MultiByteToWideChar(CP_UTF8, 0, input.data(), static_cast<int>(input.size()), nullptr, 0);
    std::wstring output(static_cast<size_t>(size), L'\0');
    MultiByteToWideChar(CP_UTF8, 0, input.data(), static_cast<int>(input.size()), output.data(), size);
    return output;
}

std::wstring JoinPath(const std::wstring& base, const std::wstring& child) {
    std::wstring result = base;
    if (!result.empty() && result.back() != L'\\' && result.back() != L'/') {
        result.push_back(L'\\');
    }
    result += child;
    return result;
}

std::optional<std::wstring> ReadEnv(const wchar_t* name) {
    const DWORD size = GetEnvironmentVariableW(name, nullptr, 0);
    if (size == 0) {
        return std::nullopt;
    }

    std::wstring value(size - 1, L'\0');
    GetEnvironmentVariableW(name, value.data(), size);
    return value;
}

bool ExtractWindow(const jsonlite::Value* windowNode, UsageWindow* output) {
    if (windowNode == nullptr || output == nullptr) {
        return false;
    }

    const jsonlite::Value* usedPercent = windowNode->Find("used_percent");
    const jsonlite::Value* limitWindowSeconds = windowNode->Find("limit_window_seconds");
    const jsonlite::Value* resetAfterSeconds = windowNode->Find("reset_after_seconds");
    const jsonlite::Value* resetAt = windowNode->Find("reset_at");
    if (usedPercent == nullptr || limitWindowSeconds == nullptr || resetAfterSeconds == nullptr || resetAt == nullptr) {
        return false;
    }

    auto used = usedPercent->AsInt();
    auto limit = limitWindowSeconds->AsInt();
    auto resetAfter = resetAfterSeconds->AsInt();
    auto resetAtValue = resetAt->AsNumber();
    if (!used.has_value() || !limit.has_value() || !resetAfter.has_value() || !resetAtValue.has_value()) {
        return false;
    }

    output->usedPercent = std::clamp(*used, 0, 100);
    output->remainingPercent = 100 - output->usedPercent;
    output->windowSeconds = std::max(*limit, 0);
    output->resetAfterSeconds = std::max(*resetAfter, 0);
    output->resetAtUnixSeconds = static_cast<long long>(*resetAtValue);
    return true;
}

std::optional<std::string> HttpGetJson(const std::wstring& userAgent,
                                       const std::wstring& host,
                                       const std::wstring& path,
                                       const std::vector<std::wstring>& headers,
                                       std::wstring* errorMessage) {
    if (errorMessage != nullptr) {
        errorMessage->clear();
    }

    HINTERNET session = WinHttpOpen(userAgent.c_str(), WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
        WINHTTP_NO_PROXY_NAME, WINHTTP_NO_PROXY_BYPASS, 0);
    if (session == nullptr) {
        if (errorMessage != nullptr) {
            *errorMessage = L"WinHttpOpen failed";
        }
        return std::nullopt;
    }

    std::optional<std::string> responseBody;
    HINTERNET connect = nullptr;
    HINTERNET request = nullptr;

    do {
        connect = WinHttpConnect(session, host.c_str(), INTERNET_DEFAULT_HTTPS_PORT, 0);
        if (connect == nullptr) {
            if (errorMessage != nullptr) {
                *errorMessage = L"WinHttpConnect failed";
            }
            break;
        }

        request = WinHttpOpenRequest(connect, L"GET", path.c_str(), nullptr,
            WINHTTP_NO_REFERER, WINHTTP_DEFAULT_ACCEPT_TYPES, WINHTTP_FLAG_SECURE);
        if (request == nullptr) {
            if (errorMessage != nullptr) {
                *errorMessage = L"WinHttpOpenRequest failed";
            }
            break;
        }

        for (const std::wstring& header : headers) {
            if (!WinHttpAddRequestHeaders(request, header.c_str(), static_cast<DWORD>(-1L), WINHTTP_ADDREQ_FLAG_ADD)) {
                if (errorMessage != nullptr) {
                    *errorMessage = L"WinHttpAddRequestHeaders failed";
                }
                break;
            }
        }
        if (errorMessage != nullptr && !errorMessage->empty()) {
            break;
        }

        DWORD timeout = 15000;
        WinHttpSetTimeouts(request, timeout, timeout, timeout, timeout);

        if (!WinHttpSendRequest(request, WINHTTP_NO_ADDITIONAL_HEADERS, 0, WINHTTP_NO_REQUEST_DATA, 0, 0, 0)) {
            if (errorMessage != nullptr) {
                *errorMessage = L"WinHttpSendRequest failed";
            }
            break;
        }

        if (!WinHttpReceiveResponse(request, nullptr)) {
            if (errorMessage != nullptr) {
                *errorMessage = L"WinHttpReceiveResponse failed";
            }
            break;
        }

        DWORD statusCode = 0;
        DWORD statusCodeSize = sizeof(statusCode);
        WinHttpQueryHeaders(request, WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            WINHTTP_HEADER_NAME_BY_INDEX, &statusCode, &statusCodeSize, WINHTTP_NO_HEADER_INDEX);
        if (statusCode != 200) {
            if (errorMessage != nullptr) {
                *errorMessage = host + path + L" returned HTTP " + std::to_wstring(statusCode);
            }
            break;
        }

        std::string body;
        for (;;) {
            DWORD available = 0;
            if (!WinHttpQueryDataAvailable(request, &available)) {
                if (errorMessage != nullptr) {
                    *errorMessage = L"WinHttpQueryDataAvailable failed";
                }
                break;
            }
            if (available == 0) {
                responseBody = std::move(body);
                break;
            }

            std::string chunk(static_cast<size_t>(available), '\0');
            DWORD downloaded = 0;
            if (!WinHttpReadData(request, chunk.data(), available, &downloaded)) {
                if (errorMessage != nullptr) {
                    *errorMessage = L"WinHttpReadData failed";
                }
                break;
            }

            chunk.resize(downloaded);
            body.append(chunk);
        }
    } while (false);

    if (request != nullptr) {
        WinHttpCloseHandle(request);
    }
    if (connect != nullptr) {
        WinHttpCloseHandle(connect);
    }
    WinHttpCloseHandle(session);

    return responseBody;
}

}  // namespace

UsageSnapshot CodexUsageFetcher::Fetch() const {
    UsageSnapshot snapshot;

    std::wstring errorMessage;
    std::optional<std::string> accessToken = ReadAccessToken(&errorMessage);
    if (!accessToken.has_value()) {
        snapshot.errorMessage = errorMessage;
        return snapshot;
    }

    std::optional<std::string> usageJson = HttpGetUsageJson(*accessToken, &errorMessage);
    if (!usageJson.has_value()) {
        snapshot.errorMessage = errorMessage;
        return snapshot;
    }

    snapshot = ParseUsageJson(*usageJson, &errorMessage);
    if (!snapshot.success) {
        snapshot.errorMessage = errorMessage;
    }

    return snapshot;
}

ReleaseVersionInfo CodexUsageFetcher::FetchLatestRelease() const {
    ReleaseVersionInfo info;

    std::wstring errorMessage;
    std::optional<std::string> releaseJson = HttpGetLatestReleaseJson(&errorMessage);
    if (!releaseJson.has_value()) {
        info.errorMessage = errorMessage;
        return info;
    }

    info = ParseLatestReleaseJson(*releaseJson, &errorMessage);
    if (!info.success) {
        info.errorMessage = errorMessage;
    }
    return info;
}

std::wstring CodexUsageFetcher::ResolveAuthJsonPath() const {
    if (auto codexHome = ReadEnv(L"CODEX_HOME"); codexHome.has_value() && !codexHome->empty()) {
        return JoinPath(*codexHome, L"auth.json");
    }

    if (auto userProfile = ReadEnv(L"USERPROFILE"); userProfile.has_value() && !userProfile->empty()) {
        return JoinPath(JoinPath(*userProfile, L".codex"), L"auth.json");
    }

    return L".codex\\auth.json";
}

std::optional<std::string> CodexUsageFetcher::ReadAccessToken(std::wstring* errorMessage) const {
    const std::wstring authPath = ResolveAuthJsonPath();
    std::optional<std::string> jsonText = LoadFileUtf8(authPath, errorMessage);
    if (!jsonText.has_value()) {
        return std::nullopt;
    }

    jsonlite::Parser parser(*jsonText);
    std::optional<jsonlite::Value> root = parser.Parse();
    if (!root.has_value()) {
        if (errorMessage != nullptr) {
            *errorMessage = L"auth.json parse failed: " + Utf8ToWide(parser.Error());
        }
        return std::nullopt;
    }

    const jsonlite::Value* tokens = root->Find("tokens");
    const jsonlite::Value* accessToken = tokens != nullptr ? tokens->Find("access_token") : nullptr;
    auto token = accessToken != nullptr ? accessToken->AsString() : std::nullopt;
    if (!token.has_value() || token->empty()) {
        if (errorMessage != nullptr) {
            *errorMessage = L"auth.json missing tokens.access_token";
        }
        return std::nullopt;
    }

    return std::string(*token);
}

std::optional<std::string> CodexUsageFetcher::LoadFileUtf8(const std::wstring& path, std::wstring* errorMessage) const {
    std::ifstream input(path, std::ios::binary);
    if (!input) {
        if (errorMessage != nullptr) {
            *errorMessage = L"cannot open " + path;
        }
        return std::nullopt;
    }

    std::ostringstream buffer;
    buffer << input.rdbuf();
    return buffer.str();
}

std::optional<std::string> CodexUsageFetcher::HttpGetUsageJson(const std::string& accessToken, std::wstring* errorMessage) const {
    return HttpGetJson(
        L"CodexUsageBar/0.1",
        L"chatgpt.com",
        L"/backend-api/wham/usage",
        { L"Authorization: Bearer " + Utf8ToWide(accessToken) },
        errorMessage);
}

std::optional<std::string> CodexUsageFetcher::HttpGetLatestReleaseJson(std::wstring* errorMessage) const {
    return HttpGetJson(
        L"CodexUsageBar/0.1",
        L"api.github.com",
        L"/repos/luodaoyi/codex-useage-win/releases/latest",
        {
            L"Accept: application/vnd.github+json",
            L"X-GitHub-Api-Version: 2022-11-28",
            L"User-Agent: CodexUsageBar"
        },
        errorMessage);
}

UsageSnapshot CodexUsageFetcher::ParseUsageJson(const std::string& jsonText, std::wstring* errorMessage) const {
    UsageSnapshot snapshot;

    jsonlite::Parser parser(jsonText);
    std::optional<jsonlite::Value> root = parser.Parse();
    if (!root.has_value()) {
        if (errorMessage != nullptr) {
            *errorMessage = L"usage JSON parse failed: " + Utf8ToWide(parser.Error());
        }
        return snapshot;
    }

    const jsonlite::Value* email = root->Find("email");
    const jsonlite::Value* planType = root->Find("plan_type");
    const jsonlite::Value* rateLimit = root->Find("rate_limit");
    const jsonlite::Value* primaryWindow = rateLimit != nullptr ? rateLimit->Find("primary_window") : nullptr;
    const jsonlite::Value* secondaryWindow = rateLimit != nullptr ? rateLimit->Find("secondary_window") : nullptr;
    if (!ExtractWindow(primaryWindow, &snapshot.fiveHour) || !ExtractWindow(secondaryWindow, &snapshot.weekly)) {
        if (errorMessage != nullptr) {
            *errorMessage = L"usage payload missing rate_limit windows";
        }
        return snapshot;
    }

    if (auto emailString = email != nullptr ? email->AsString() : std::nullopt; emailString.has_value()) {
        snapshot.email = Utf8ToWide(std::string(*emailString));
    }
    if (auto planTypeString = planType != nullptr ? planType->AsString() : std::nullopt; planTypeString.has_value()) {
        snapshot.planType = Utf8ToWide(std::string(*planTypeString));
    }

    snapshot.success = true;
    return snapshot;
}

ReleaseVersionInfo CodexUsageFetcher::ParseLatestReleaseJson(const std::string& jsonText, std::wstring* errorMessage) const {
    ReleaseVersionInfo info;

    jsonlite::Parser parser(jsonText);
    std::optional<jsonlite::Value> root = parser.Parse();
    if (!root.has_value()) {
        if (errorMessage != nullptr) {
            *errorMessage = L"latest release JSON parse failed: " + Utf8ToWide(parser.Error());
        }
        return info;
    }

    const jsonlite::Value* tagName = root->Find("tag_name");
    auto tag = tagName != nullptr ? tagName->AsString() : std::nullopt;
    if (!tag.has_value() || tag->empty()) {
        if (errorMessage != nullptr) {
            *errorMessage = L"latest release payload missing tag_name";
        }
        return info;
    }

    info.latestTag = Utf8ToWide(std::string(*tag));
    info.success = true;
    return info;
}
