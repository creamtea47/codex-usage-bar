#include "AppBarWindow.h"
#include "AppVersion.h"

#include <ShlObj.h>
#include <winreg.h>
#include <windowsx.h>

#include <algorithm>
#include <cmath>
#include <ctime>
#include <filesystem>
#include <memory>
#include <string>
#include <thread>
#include <vector>

namespace {

constexpr wchar_t kWindowClassName[] = L"CodexUsageBarWindow";
constexpr const wchar_t* kCurrentVersion = APP_VERSION_W;
constexpr int kLayoutVersion = 5;
constexpr UINT kCommandRefresh = 1;
constexpr UINT kCommandExit = 2;
constexpr UINT kCommandResetPosition = 3;
constexpr UINT kCommandLaunchAtStartup = 4;
constexpr UINT kCommandAlwaysOnTop = 5;
constexpr UINT kCommandLockPosition = 6;
constexpr UINT kCommandSimpleMode = 7;
constexpr UINT kCommandLanguageEnglish = 8;
constexpr UINT kCommandLanguageChinese = 9;
constexpr UINT kCommandRefreshInterval1Minute = 10;
constexpr UINT kCommandRefreshInterval3Minutes = 11;
constexpr UINT kCommandRefreshInterval5Minutes = 12;
constexpr UINT kCommandRefreshInterval10Minutes = 13;
constexpr UINT kCommandRefreshInterval30Minutes = 14;
constexpr UINT kCommandCheckVersion = 15;
constexpr int kDefaultWidgetWidth = 820;
constexpr int kMinimumWidgetWidth = 640;
constexpr int kSimpleDefaultWidgetWidth = 240;
constexpr int kSimpleMinimumWidgetWidth = 220;
constexpr int kDesktopMargin = 18;
constexpr int kHorizontalPadding = 12;
constexpr int kVerticalPadding = 10;
constexpr int kResizeGrip = 12;
constexpr long long kDaySeconds = 24LL * 60 * 60;
constexpr long long kWeekSeconds = 7LL * kDaySeconds;
constexpr int kReleaseCheckIntervalSeconds = 6 * 60 * 60;

int SanitizeRefreshIntervalSeconds(int seconds) {
    switch (seconds) {
        case 60:
        case 180:
        case 300:
        case 600:
        case 1800:
            return seconds;
        default:
            return 60;
    }
}

std::vector<int> ParseVersionParts(const std::wstring& version) {
    std::vector<int> parts;
    int value = 0;
    bool inNumber = false;

    for (wchar_t ch : version) {
        if (ch >= L'0' && ch <= L'9') {
            value = value * 10 + (ch - L'0');
            inNumber = true;
        } else if (inNumber) {
            parts.push_back(value);
            value = 0;
            inNumber = false;
        }
    }
    if (inNumber) {
        parts.push_back(value);
    }

    return parts;
}

int CompareVersions(const std::wstring& left, const std::wstring& right) {
    const std::vector<int> leftParts = ParseVersionParts(left);
    const std::vector<int> rightParts = ParseVersionParts(right);
    const size_t count = std::max(leftParts.size(), rightParts.size());

    for (size_t i = 0; i < count; ++i) {
        const int leftValue = i < leftParts.size() ? leftParts[i] : 0;
        const int rightValue = i < rightParts.size() ? rightParts[i] : 0;
        if (leftValue < rightValue) {
            return -1;
        }
        if (leftValue > rightValue) {
            return 1;
        }
    }

    return 0;
}

int ScaleForDpi(HWND hwnd, int value) {
    const UINT dpi = GetDpiForWindow(hwnd != nullptr ? hwnd : GetDesktopWindow());
    return MulDiv(value, static_cast<int>(dpi), 96);
}

int RectWidth(const RECT& rect) {
    return rect.right - rect.left;
}

int RectHeight(const RECT& rect) {
    return rect.bottom - rect.top;
}

int CalculateDetailedMinimumWidgetHeight(HWND hwnd, int width) {
    const int heroHeight = ScaleForDpi(hwnd, 76);
    const int metricsHeight = ScaleForDpi(hwnd, 52);
    const int meterInfoHeight = ScaleForDpi(hwnd, 30);
    const int footerRows = width >= ScaleForDpi(hwnd, 1040) ? 2 : 3;
    const int footerHeight = ScaleForDpi(hwnd, 4) + footerRows * ScaleForDpi(hwnd, 18) + ScaleForDpi(hwnd, 8);

    return heroHeight
        + 1
        + metricsHeight
        + ScaleForDpi(hwnd, 10)
        + meterInfoHeight
        + ScaleForDpi(hwnd, 6)
        + ScaleForDpi(hwnd, 10)
        + 1
        + footerHeight;
}

int CalculateSimpleMinimumWidgetHeight(HWND hwnd) {
    return ScaleForDpi(hwnd, 108);
}

RECT ShrinkRect(const RECT& rect, int dx, int dy) {
    RECT output = rect;
    output.left += dx;
    output.right -= dx;
    output.top += dy;
    output.bottom -= dy;
    return output;
}

struct PaceInfo {
    bool valid = false;
    double dailyBudgetPercent = 0.0;
    double expectedUsedPercent = 0.0;
    double actualUsedPercent = 0.0;
    double weeklyRemainingPercent = 0.0;
    double deltaPercent = 0.0;
    int cycleDay = 0;
    int elapsedSeconds = 0;
    int remainingSeconds = 0;
    long long weekStartUnixSeconds = 0;
    bool isOver = false;
};

int ClampInt(int value, int minValue, int maxValue) {
    return std::min(maxValue, std::max(minValue, value));
}

double ClampDouble(double value, double minValue, double maxValue) {
    return std::min(maxValue, std::max(minValue, value));
}

RECT MakeRect(int left, int top, int right, int bottom) {
    RECT rect = { left, top, right, bottom };
    return rect;
}

D2D1_RECT_F ToRectF(const RECT& rect) {
    return D2D1::RectF(
        static_cast<float>(rect.left),
        static_cast<float>(rect.top),
        static_cast<float>(rect.right),
        static_cast<float>(rect.bottom));
}

D2D1_COLOR_F ToColorF(COLORREF color, float alpha = 1.0f) {
    return D2D1::ColorF(
        static_cast<float>(GetRValue(color)) / 255.0f,
        static_cast<float>(GetGValue(color)) / 255.0f,
        static_cast<float>(GetBValue(color)) / 255.0f,
        alpha);
}

void FillSolidRect(HDC hdc, const RECT& rect, COLORREF color) {
    HBRUSH brush = CreateSolidBrush(color);
    FillRect(hdc, &rect, brush);
    DeleteObject(brush);
}

void StrokeRect(HDC hdc, const RECT& rect, COLORREF color) {
    HPEN pen = CreatePen(PS_SOLID, 1, color);
    HGDIOBJ oldPen = SelectObject(hdc, pen);
    HGDIOBJ oldBrush = SelectObject(hdc, GetStockObject(HOLLOW_BRUSH));
    Rectangle(hdc, rect.left, rect.top, rect.right, rect.bottom);
    SelectObject(hdc, oldBrush);
    SelectObject(hdc, oldPen);
    DeleteObject(pen);
}

std::wstring FormatNumber(double value) {
    wchar_t buffer[32] = {};
    swprintf_s(buffer, L"%.1f", value);
    return buffer;
}

std::wstring FormatNumberNoUnit(double value) {
    wchar_t buffer[32] = {};
    swprintf_s(buffer, L"%.1f", value);
    return buffer;
}

PaceInfo BuildPaceInfo(const UsageSnapshot& snapshot) {
    PaceInfo info;
    if (!snapshot.success || snapshot.weekly.windowSeconds <= 0) {
        return info;
    }

    info.dailyBudgetPercent = 100.0 / 7.0;
    info.actualUsedPercent = static_cast<double>(snapshot.weekly.usedPercent);
    info.weeklyRemainingPercent = static_cast<double>(snapshot.weekly.remainingPercent);
    info.remainingSeconds = std::max(0, snapshot.weekly.resetAfterSeconds);
    info.elapsedSeconds = ClampInt(snapshot.weekly.windowSeconds - info.remainingSeconds, 0, snapshot.weekly.windowSeconds);
    info.weekStartUnixSeconds = snapshot.weekly.resetAtUnixSeconds - snapshot.weekly.windowSeconds;

    const int elapsedDays = info.elapsedSeconds <= 0 ? 0 : (info.elapsedSeconds / static_cast<int>(kDaySeconds));
    info.cycleDay = ClampInt(elapsedDays + 1, 1, 7);
    info.expectedUsedPercent = ClampDouble(info.cycleDay * info.dailyBudgetPercent, 0.0, 100.0);
    info.deltaPercent = info.actualUsedPercent - info.expectedUsedPercent;
    info.isOver = info.deltaPercent > 0.001;
    info.valid = true;
    return info;
}

}  // namespace

AppBarWindow::AppBarWindow(HINSTANCE instance) : instance_(instance) {}

AppBarWindow::~AppBarWindow() {
    DiscardTextFormats();
    DiscardDeviceResources();
}

bool AppBarWindow::Create() {
    RegisterWindowClass();
    LoadSettings();
    hwnd_ = CreateWindowExW(
        WS_EX_TOOLWINDOW,
        kWindowClassName,
        LocalizeText(L"Codex Usage Widget", L"Codex 用量挂件"),
        WS_POPUP | WS_VISIBLE,
        0,
        0,
        100,
        100,
        nullptr,
        nullptr,
        instance_,
        this);

    if (hwnd_ == nullptr) {
        return false;
    }

    if (FAILED(CreateDeviceIndependentResources())) {
        return false;
    }

    RefreshTheme();
    UpdateWindowBounds(true);
    ShowWindow(hwnd_, SW_SHOW);
    UpdateWindow(hwnd_);
    InvalidateRect(hwnd_, nullptr, TRUE);

    refreshCountdownSeconds_ = refreshIntervalSeconds_;
    releaseCheckCountdownSeconds_ = kReleaseCheckIntervalSeconds;
    SetTimer(hwnd_, kCountdownTimerId, 1000, nullptr);
    RestartRefreshTimer();
    RequestRefresh(true);
    RequestLatestReleaseCheck(true);
    return true;
}

int AppBarWindow::Run() {
    MSG message;
    while (GetMessageW(&message, nullptr, 0, 0) > 0) {
        TranslateMessage(&message);
        DispatchMessageW(&message);
    }
    return static_cast<int>(message.wParam);
}

LRESULT CALLBACK AppBarWindow::WindowProc(HWND hwnd, UINT message, WPARAM wParam, LPARAM lParam) {
    if (message == WM_NCCREATE) {
        const auto* createStruct = reinterpret_cast<CREATESTRUCTW*>(lParam);
        auto* self = static_cast<AppBarWindow*>(createStruct->lpCreateParams);
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, reinterpret_cast<LONG_PTR>(self));
        self->hwnd_ = hwnd;
    }

    auto* self = reinterpret_cast<AppBarWindow*>(GetWindowLongPtrW(hwnd, GWLP_USERDATA));
    if (self != nullptr) {
        return self->HandleMessage(message, wParam, lParam);
    }
    return DefWindowProcW(hwnd, message, wParam, lParam);
}

LRESULT AppBarWindow::HandleMessage(UINT message, WPARAM wParam, LPARAM lParam) {
    switch (message) {
        case WM_TIMER:
            if (wParam == kCountdownTimerId) {
                if (snapshot_.success) {
                    snapshot_.fiveHour.resetAfterSeconds = std::max(0, snapshot_.fiveHour.resetAfterSeconds - 1);
                    snapshot_.weekly.resetAfterSeconds = std::max(0, snapshot_.weekly.resetAfterSeconds - 1);
                }
                refreshCountdownSeconds_ = std::max(0, refreshCountdownSeconds_ - 1);
                releaseCheckCountdownSeconds_ = std::max(0, releaseCheckCountdownSeconds_ - 1);
                if (releaseCheckCountdownSeconds_ == 0) {
                    RequestLatestReleaseCheck(false);
                }
                InvalidateRect(hwnd_, nullptr, FALSE);
            } else if (wParam == kRefreshTimerId) {
                refreshCountdownSeconds_ = refreshIntervalSeconds_;
                RequestRefresh(false);
            }
            return 0;

        case WM_ERASEBKGND:
            return 1;

        case WM_PAINT: {
            PAINTSTRUCT ps;
            HDC hdc = BeginPaint(hwnd_, &ps);
            Paint(hdc);
            EndPaint(hwnd_, &ps);
            return 0;
        }

        case WM_THEMECHANGED:
        case WM_SETTINGCHANGE:
            RefreshTheme();
            InvalidateRect(hwnd_, nullptr, FALSE);
            return 0;

        case WM_DISPLAYCHANGE:
            UpdateWindowBounds(true);
            return 0;

        case WM_DPICHANGED:
            DiscardTextFormats();
            UpdateWindowBounds(true);
            InvalidateRect(hwnd_, nullptr, FALSE);
            return 0;

        case WM_SETCURSOR: {
            POINT screenPoint = {};
            GetCursorPos(&screenPoint);
            POINT clientPoint = screenPoint;
            ScreenToClient(hwnd_, &clientPoint);
            switch (HitTestDragMode(clientPoint)) {
                case DragMode::ResizeRight:
                    SetCursor(LoadCursorW(nullptr, IDC_SIZEWE));
                    return TRUE;
                case DragMode::ResizeBottom:
                    SetCursor(LoadCursorW(nullptr, IDC_SIZENS));
                    return TRUE;
                case DragMode::ResizeCorner:
                    SetCursor(LoadCursorW(nullptr, IDC_SIZENWSE));
                    return TRUE;
                case DragMode::Move:
                    SetCursor(LoadCursorW(nullptr, IDC_SIZEALL));
                    return TRUE;
                case DragMode::None:
                    break;
            }
            break;
        }

        case WM_LBUTTONDOWN: {
            POINT screenPoint = { GET_X_LPARAM(lParam), GET_Y_LPARAM(lParam) };
            ClientToScreen(hwnd_, &screenPoint);
            POINT clientPoint = { GET_X_LPARAM(lParam), GET_Y_LPARAM(lParam) };
            BeginDrag(HitTestDragMode(clientPoint), screenPoint);
            return 0;
        }

        case WM_MOUSEMOVE:
            if (dragMode_ != DragMode::None) {
                POINT screenPoint = { GET_X_LPARAM(lParam), GET_Y_LPARAM(lParam) };
                ClientToScreen(hwnd_, &screenPoint);
                UpdateDrag(screenPoint);
                return 0;
            }
            break;

        case WM_LBUTTONUP:
            if (dragMode_ != DragMode::None) {
                EndDrag(true);
            } else {
                RequestRefresh(true);
            }
            return 0;

        case WM_CAPTURECHANGED:
            if (dragMode_ != DragMode::None) {
                EndDrag(true);
            }
            return 0;

        case WM_CONTEXTMENU: {
            POINT point = { GET_X_LPARAM(lParam), GET_Y_LPARAM(lParam) };
            if (point.x == -1 && point.y == -1) {
                RECT windowRect = {};
                GetWindowRect(hwnd_, &windowRect);
                point.x = windowRect.left + ScaleForDpi(hwnd_, 18);
                point.y = windowRect.top + ScaleForDpi(hwnd_, 18);
            }
            ShowContextMenu(point);
            return 0;
        }

        case kUsageUpdatedMessage:
            OnUsageUpdated(reinterpret_cast<UsageSnapshot*>(lParam));
            return 0;

        case kReleaseVersionUpdatedMessage:
            OnLatestReleaseChecked(reinterpret_cast<ReleaseVersionInfo*>(lParam));
            return 0;

        case WM_DESTROY:
            KillTimer(hwnd_, kCountdownTimerId);
            KillTimer(hwnd_, kRefreshTimerId);
            SaveSettings();
            DiscardTextFormats();
            DiscardDeviceResources();
            PostQuitMessage(0);
            return 0;
    }

    return DefWindowProcW(hwnd_, message, wParam, lParam);
}

void AppBarWindow::RegisterWindowClass() {
    WNDCLASSEXW wc = {};
    wc.cbSize = sizeof(wc);
    wc.style = CS_HREDRAW | CS_VREDRAW;
    wc.lpfnWndProc = WindowProc;
    wc.hInstance = instance_;
    wc.hCursor = LoadCursorW(nullptr, IDC_ARROW);
    wc.hbrBackground = reinterpret_cast<HBRUSH>(GetStockObject(BLACK_BRUSH));
    wc.lpszClassName = kWindowClassName;
    RegisterClassExW(&wc);
}

RECT AppBarWindow::GetDesktopClientRect() const {
    RECT rect = {};
    rect.left = GetSystemMetrics(SM_XVIRTUALSCREEN);
    rect.top = GetSystemMetrics(SM_YVIRTUALSCREEN);
    rect.right = rect.left + GetSystemMetrics(SM_CXVIRTUALSCREEN);
    rect.bottom = rect.top + GetSystemMetrics(SM_CYVIRTUALSCREEN);
    return rect;
}

int AppBarWindow::GetMinimumWidgetWidth() const {
    return ScaleForDpi(hwnd_, simpleMode_ ? kSimpleMinimumWidgetWidth : kMinimumWidgetWidth);
}

int AppBarWindow::GetMinimumWidgetHeight(int width) const {
    if (simpleMode_) {
        return CalculateSimpleMinimumWidgetHeight(hwnd_);
    }
    return CalculateDetailedMinimumWidgetHeight(hwnd_, width);
}

void AppBarWindow::SetLanguage(Language language) {
    if (language_ == language) {
        return;
    }

    language_ = language;
    if (hwnd_ != nullptr) {
        SetWindowTextW(hwnd_, LocalizeText(L"Codex Usage Widget", L"Codex 用量挂件"));
        InvalidateRect(hwnd_, nullptr, TRUE);
    }
    SaveSettings();
}

void AppBarWindow::SetRefreshIntervalSeconds(int seconds) {
    const int sanitized = SanitizeRefreshIntervalSeconds(seconds);
    if (refreshIntervalSeconds_ == sanitized) {
        return;
    }

    refreshIntervalSeconds_ = sanitized;
    refreshCountdownSeconds_ = refreshIntervalSeconds_;
    if (hwnd_ != nullptr) {
        RestartRefreshTimer();
        InvalidateRect(hwnd_, nullptr, FALSE);
    }
    SaveSettings();
}

void AppBarWindow::RestartRefreshTimer() {
    if (hwnd_ == nullptr) {
        return;
    }

    KillTimer(hwnd_, kRefreshTimerId);
    SetTimer(hwnd_, kRefreshTimerId, static_cast<UINT>(refreshIntervalSeconds_ * 1000), nullptr);
}

const wchar_t* AppBarWindow::LocalizeText(const wchar_t* english, const wchar_t* chinese) const {
    return language_ == Language::Chinese ? chinese : english;
}

std::wstring AppBarWindow::GetVersionStatusText(bool compact) const {
    if (updateAvailable_ && !latestReleaseTag_.empty()) {
        if (compact) {
            return language_ == Language::Chinese
                ? (std::wstring(kCurrentVersion) + L" -> " + latestReleaseTag_)
                : (std::wstring(kCurrentVersion) + L" -> " + latestReleaseTag_);
        }
        return language_ == Language::Chinese
            ? (L"版本: " + std::wstring(kCurrentVersion) + L"，可更新到 " + latestReleaseTag_)
            : (L"Version: " + std::wstring(kCurrentVersion) + L", update available: " + latestReleaseTag_);
    }

    if (compact) {
        return kCurrentVersion;
    }
    return language_ == Language::Chinese
        ? (L"版本: " + std::wstring(kCurrentVersion))
        : (L"Version: " + std::wstring(kCurrentVersion));
}

RECT AppBarWindow::BuildDefaultRect(const RECT& desktopRect) const {
    const int margin = ScaleForDpi(hwnd_, kDesktopMargin);
    const int width = ScaleForDpi(hwnd_, simpleMode_ ? kSimpleDefaultWidgetWidth : kDefaultWidgetWidth);
    const int height = GetMinimumWidgetHeight(width);
    RECT rect = {};
    rect.right = std::max(width + margin, static_cast<int>(desktopRect.right) - margin);
    rect.left = std::max(static_cast<int>(desktopRect.left) + margin, static_cast<int>(rect.right) - width);
    rect.bottom = std::max(height + margin, static_cast<int>(desktopRect.bottom) - margin);
    rect.top = std::max(static_cast<int>(desktopRect.top) + margin, static_cast<int>(rect.bottom) - height);
    return rect;
}

RECT AppBarWindow::ClampRectToDesktop(RECT rect) const {
    const RECT desktopRect = GetDesktopClientRect();
    const int minWidth = GetMinimumWidgetWidth();
    const int minHeight = GetMinimumWidgetHeight(std::max(RectWidth(rect), minWidth));

    if (RectWidth(rect) < minWidth) {
        rect.right = rect.left + minWidth;
    }
    if (RectHeight(rect) < minHeight) {
        rect.bottom = rect.top + minHeight;
    }

    if (rect.left < desktopRect.left) {
        const int width = RectWidth(rect);
        rect.left = desktopRect.left;
        rect.right = rect.left + width;
    }
    if (rect.top < desktopRect.top) {
        const int height = RectHeight(rect);
        rect.top = desktopRect.top;
        rect.bottom = rect.top + height;
    }
    if (rect.right > desktopRect.right) {
        const int width = RectWidth(rect);
        rect.right = desktopRect.right;
        rect.left = rect.right - width;
    }
    if (rect.bottom > desktopRect.bottom) {
        const int height = RectHeight(rect);
        rect.bottom = desktopRect.bottom;
        rect.top = rect.bottom - height;
    }

    rect.left = std::max(rect.left, desktopRect.left);
    rect.top = std::max(rect.top, desktopRect.top);
    rect.right = std::min(rect.right, desktopRect.right);
    rect.bottom = std::min(rect.bottom, desktopRect.bottom);
    return rect;
}

void AppBarWindow::UpdateWindowBounds(bool useSavedPosition) {
    const bool usingPersistedRect = useSavedPosition && hasSavedRect_;
    RECT rect = usingPersistedRect ? savedRect_ : BuildDefaultRect(GetDesktopClientRect());
    rect = ClampRectToDesktop(rect);
    savedRect_ = rect;
    hasSavedRect_ = true;
    MoveWindow(hwnd_, rect.left, rect.top, RectWidth(rect), RectHeight(rect), TRUE);
    SetWindowPos(hwnd_, alwaysOnTop_ ? HWND_TOPMOST : HWND_NOTOPMOST,
        rect.left, rect.top, RectWidth(rect), RectHeight(rect), SWP_NOACTIVATE);
    if (!usingPersistedRect) {
        SaveSettings();
    }
}

void AppBarWindow::LoadSettings() {
    const std::wstring path = GetSettingsPath();
    const int version = GetPrivateProfileIntW(L"layout", L"layout_version", 0, path.c_str());
    alwaysOnTop_ = GetPrivateProfileIntW(L"layout", L"always_on_top", 0, path.c_str()) != 0;
    lockPosition_ = GetPrivateProfileIntW(L"layout", L"lock_position", 0, path.c_str()) != 0;
    simpleMode_ = GetPrivateProfileIntW(L"layout", L"simple_mode", 0, path.c_str()) != 0;
    refreshIntervalSeconds_ = SanitizeRefreshIntervalSeconds(
        GetPrivateProfileIntW(L"layout", L"refresh_interval_seconds", 60, path.c_str()));
    refreshCountdownSeconds_ = refreshIntervalSeconds_;
    language_ = GetPrivateProfileIntW(L"layout", L"language", 0, path.c_str()) == 1
        ? Language::Chinese
        : Language::English;
    if (version < kLayoutVersion) {
        hasSavedRect_ = false;
        return;
    }

    const int width = GetPrivateProfileIntW(L"layout", L"width", 0, path.c_str());
    const int height = GetPrivateProfileIntW(L"layout", L"height", 0, path.c_str());
    if (width <= 0 || height <= 0) {
        hasSavedRect_ = false;
        return;
    }

    savedRect_.left = GetPrivateProfileIntW(L"layout", L"x", 0, path.c_str());
    savedRect_.top = GetPrivateProfileIntW(L"layout", L"y", 0, path.c_str());
    savedRect_.right = savedRect_.left + width;
    savedRect_.bottom = savedRect_.top + height;
    savedRect_ = ClampRectToDesktop(savedRect_);
    hasSavedRect_ = true;
}

void AppBarWindow::SaveSettings() const {
    if (!hasSavedRect_) {
        return;
    }

    const std::wstring path = GetSettingsPath();
    std::error_code ec;
    std::filesystem::create_directories(std::filesystem::path(path).parent_path(), ec);
    WritePrivateProfileStringW(L"layout", L"layout_version", std::to_wstring(kLayoutVersion).c_str(), path.c_str());
    WritePrivateProfileStringW(L"layout", L"always_on_top", alwaysOnTop_ ? L"1" : L"0", path.c_str());
    WritePrivateProfileStringW(L"layout", L"lock_position", lockPosition_ ? L"1" : L"0", path.c_str());
    WritePrivateProfileStringW(L"layout", L"simple_mode", simpleMode_ ? L"1" : L"0", path.c_str());
    WritePrivateProfileStringW(L"layout", L"refresh_interval_seconds", std::to_wstring(refreshIntervalSeconds_).c_str(), path.c_str());
    WritePrivateProfileStringW(L"layout", L"language", language_ == Language::Chinese ? L"1" : L"0", path.c_str());
    WritePrivateProfileStringW(L"layout", L"x", std::to_wstring(savedRect_.left).c_str(), path.c_str());
    WritePrivateProfileStringW(L"layout", L"y", std::to_wstring(savedRect_.top).c_str(), path.c_str());
    WritePrivateProfileStringW(L"layout", L"width", std::to_wstring(RectWidth(savedRect_)).c_str(), path.c_str());
    WritePrivateProfileStringW(L"layout", L"height", std::to_wstring(RectHeight(savedRect_)).c_str(), path.c_str());
}

std::wstring AppBarWindow::GetSettingsPath() const {
    PWSTR appDataPath = nullptr;
    if (SUCCEEDED(SHGetKnownFolderPath(FOLDERID_RoamingAppData, 0, nullptr, &appDataPath))) {
        const std::filesystem::path path = std::filesystem::path(appDataPath) / L"CodexUsageBar" / L"settings.ini";
        CoTaskMemFree(appDataPath);
        return path.wstring();
    }

    wchar_t modulePath[MAX_PATH] = {};
    GetModuleFileNameW(instance_, modulePath, MAX_PATH);
    return (std::filesystem::path(modulePath).parent_path() / L"settings.ini").wstring();
}

std::wstring AppBarWindow::GetExecutablePath() const {
    wchar_t modulePath[MAX_PATH] = {};
    GetModuleFileNameW(instance_, modulePath, MAX_PATH);
    return modulePath;
}

void AppBarWindow::RefreshTheme() {
    lightTheme_ = IsDesktopLightTheme();
}

bool AppBarWindow::IsDesktopLightTheme() const {
    DWORD value = 0;
    DWORD size = sizeof(value);
    const LONG status = RegGetValueW(
        HKEY_CURRENT_USER,
        L"Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize",
        L"AppsUseLightTheme",
        RRF_RT_REG_DWORD,
        nullptr,
        &value,
        &size);
    if (status != ERROR_SUCCESS) {
        return false;
    }
    return value != 0;
}

bool AppBarWindow::IsLaunchAtStartupEnabled() const {
    wchar_t value[2048] = {};
    DWORD size = sizeof(value);
    const LONG status = RegGetValueW(
        HKEY_CURRENT_USER,
        L"Software\\Microsoft\\Windows\\CurrentVersion\\Run",
        L"CodexUsageBar",
        RRF_RT_REG_SZ,
        nullptr,
        value,
        &size);
    if (status != ERROR_SUCCESS) {
        return false;
    }

    const std::wstring expected = L"\"" + GetExecutablePath() + L"\"";
    return std::wstring(value) == expected;
}

bool AppBarWindow::SetLaunchAtStartupEnabled(bool enabled) const {
    const wchar_t* subkey = L"Software\\Microsoft\\Windows\\CurrentVersion\\Run";
    const wchar_t* valueName = L"CodexUsageBar";

    if (enabled) {
        const std::wstring command = L"\"" + GetExecutablePath() + L"\"";
        const LONG status = RegSetKeyValueW(
            HKEY_CURRENT_USER,
            subkey,
            valueName,
            REG_SZ,
            command.c_str(),
            static_cast<DWORD>((command.size() + 1) * sizeof(wchar_t)));
        return status == ERROR_SUCCESS;
    }

    const LONG status = RegDeleteKeyValueW(HKEY_CURRENT_USER, subkey, valueName);
    return status == ERROR_SUCCESS || status == ERROR_FILE_NOT_FOUND;
}

HRESULT AppBarWindow::CreateDeviceIndependentResources() {
    if (!d2dFactory_) {
        const HRESULT hr = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, d2dFactory_.GetAddressOf());
        if (FAILED(hr)) {
            return hr;
        }
    }

    if (!dwriteFactory_) {
        const HRESULT hr = DWriteCreateFactory(
            DWRITE_FACTORY_TYPE_SHARED,
            __uuidof(IDWriteFactory),
            reinterpret_cast<IUnknown**>(dwriteFactory_.GetAddressOf()));
        if (FAILED(hr)) {
            return hr;
        }
    }

    return S_OK;
}

HRESULT AppBarWindow::CreateTextFormat(float sizePixels, DWRITE_FONT_WEIGHT weight, IDWriteTextFormat** format) {
    return dwriteFactory_->CreateTextFormat(
        L"Segoe UI",
        nullptr,
        weight,
        DWRITE_FONT_STYLE_NORMAL,
        DWRITE_FONT_STRETCH_NORMAL,
        sizePixels,
        L"zh-CN",
        format);
}

void AppBarWindow::DiscardTextFormats() {
    textFormatKicker_.Reset();
    textFormatTitle_.Reset();
    textFormatDelta_.Reset();
    textFormatMetricLabel_.Reset();
    textFormatMetricValue_.Reset();
    textFormatFoot_.Reset();
    textFormatDpi_ = 0;
}

HRESULT AppBarWindow::EnsureTextFormats() {
    const UINT dpi = GetDpiForWindow(hwnd_);
    if (textFormatDpi_ == dpi &&
        textFormatKicker_ &&
        textFormatTitle_ &&
        textFormatDelta_ &&
        textFormatMetricLabel_ &&
        textFormatMetricValue_ &&
        textFormatFoot_) {
        return S_OK;
    }

    DiscardTextFormats();

    HRESULT hr = CreateTextFormat(static_cast<float>(ScaleForDpi(hwnd_, 12)), DWRITE_FONT_WEIGHT_NORMAL, textFormatKicker_.GetAddressOf());
    if (FAILED(hr)) return hr;
    hr = CreateTextFormat(static_cast<float>(ScaleForDpi(hwnd_, 18)), DWRITE_FONT_WEIGHT_SEMI_BOLD, textFormatTitle_.GetAddressOf());
    if (FAILED(hr)) return hr;
    hr = CreateTextFormat(static_cast<float>(ScaleForDpi(hwnd_, 28)), DWRITE_FONT_WEIGHT_BOLD, textFormatDelta_.GetAddressOf());
    if (FAILED(hr)) return hr;
    hr = CreateTextFormat(static_cast<float>(ScaleForDpi(hwnd_, 12)), DWRITE_FONT_WEIGHT_NORMAL, textFormatMetricLabel_.GetAddressOf());
    if (FAILED(hr)) return hr;
    hr = CreateTextFormat(static_cast<float>(ScaleForDpi(hwnd_, 17)), DWRITE_FONT_WEIGHT_BOLD, textFormatMetricValue_.GetAddressOf());
    if (FAILED(hr)) return hr;
    hr = CreateTextFormat(static_cast<float>(ScaleForDpi(hwnd_, 12)), DWRITE_FONT_WEIGHT_NORMAL, textFormatFoot_.GetAddressOf());
    if (FAILED(hr)) return hr;

    textFormatDpi_ = dpi;
    return S_OK;
}

HRESULT AppBarWindow::CreateDeviceResources() {
    if (FAILED(CreateDeviceIndependentResources())) {
        return E_FAIL;
    }

    if (!renderTarget_) {
        const D2D1_RENDER_TARGET_PROPERTIES properties = D2D1::RenderTargetProperties(
            D2D1_RENDER_TARGET_TYPE_DEFAULT,
            D2D1::PixelFormat(DXGI_FORMAT_B8G8R8A8_UNORM, D2D1_ALPHA_MODE_IGNORE),
            96.0f,
            96.0f,
            D2D1_RENDER_TARGET_USAGE_GDI_COMPATIBLE);
        HRESULT hr = d2dFactory_->CreateDCRenderTarget(&properties, renderTarget_.GetAddressOf());
        if (FAILED(hr)) {
            return hr;
        }

        hr = renderTarget_->CreateSolidColorBrush(D2D1::ColorF(0, 0.0f), solidBrush_.GetAddressOf());
        if (FAILED(hr)) {
            return hr;
        }
    }

    return EnsureTextFormats();
}

void AppBarWindow::DiscardDeviceResources() {
    solidBrush_.Reset();
    renderTarget_.Reset();
}

AppBarWindow::DragMode AppBarWindow::HitTestDragMode(POINT clientPoint) const {
    if (lockPosition_) {
        return DragMode::None;
    }

    RECT clientRect = {};
    GetClientRect(hwnd_, &clientRect);
    const int grip = ScaleForDpi(hwnd_, kResizeGrip);
    const bool nearRight = clientPoint.x >= clientRect.right - grip;
    const bool nearBottom = clientPoint.y >= clientRect.bottom - grip;
    if (nearRight && nearBottom) {
        return DragMode::ResizeCorner;
    }
    if (nearRight) {
        return DragMode::ResizeRight;
    }
    if (nearBottom) {
        return DragMode::ResizeBottom;
    }
    return DragMode::Move;
}

void AppBarWindow::BeginDrag(DragMode mode, POINT screenPoint) {
    if (mode == DragMode::None) {
        return;
    }

    dragMode_ = mode;
    dragStartPoint_ = screenPoint;
    dragStartRect_ = savedRect_;
    SetCapture(hwnd_);
}

void AppBarWindow::UpdateDrag(POINT screenPoint) {
    if (dragMode_ == DragMode::None) {
        return;
    }

    RECT rect = dragStartRect_;
    const int deltaX = screenPoint.x - dragStartPoint_.x;
    const int deltaY = screenPoint.y - dragStartPoint_.y;

    switch (dragMode_) {
        case DragMode::Move:
            OffsetRect(&rect, deltaX, deltaY);
            break;
        case DragMode::ResizeRight:
            rect.right += deltaX;
            break;
        case DragMode::ResizeBottom:
            rect.bottom += deltaY;
            break;
        case DragMode::ResizeCorner:
            rect.right += deltaX;
            rect.bottom += deltaY;
            break;
        case DragMode::None:
            break;
    }

    savedRect_ = ClampRectToDesktop(rect);
    MoveWindow(hwnd_, savedRect_.left, savedRect_.top, RectWidth(savedRect_), RectHeight(savedRect_), TRUE);
}

void AppBarWindow::EndDrag(bool saveSettings) {
    ReleaseCapture();
    dragMode_ = DragMode::None;
    if (saveSettings) {
        SaveSettings();
    }
}

void AppBarWindow::RequestRefresh(bool force) {
    bool expected = false;
    if (!force && !refreshInFlight_.compare_exchange_strong(expected, true)) {
        return;
    }
    if (force && refreshInFlight_.exchange(true)) {
        return;
    }

    refreshCountdownSeconds_ = refreshIntervalSeconds_;
    RestartRefreshTimer();

    const HWND target = hwnd_;
    std::thread([this, target]() {
        auto* result = new UsageSnapshot(fetcher_.Fetch());
        PostMessageW(target, kUsageUpdatedMessage, 0, reinterpret_cast<LPARAM>(result));
    }).detach();
}

void AppBarWindow::OnUsageUpdated(UsageSnapshot* snapshot) {
    std::unique_ptr<UsageSnapshot> holder(snapshot);
    refreshInFlight_ = false;
    if (snapshot != nullptr) {
        snapshot_ = *snapshot;
        if (snapshot_.success) {
            lastSuccessfulRefreshUnixSeconds_ = static_cast<long long>(std::time(nullptr));
        }
    }
    InvalidateRect(hwnd_, nullptr, FALSE);
}

void AppBarWindow::RequestLatestReleaseCheck(bool force) {
    bool expected = false;
    if (!force && !releaseCheckInFlight_.compare_exchange_strong(expected, true)) {
        return;
    }
    if (force && releaseCheckInFlight_.exchange(true)) {
        return;
    }

    releaseCheckCountdownSeconds_ = kReleaseCheckIntervalSeconds;

    const HWND target = hwnd_;
    std::thread([this, target]() {
        auto* result = new ReleaseVersionInfo(fetcher_.FetchLatestRelease());
        PostMessageW(target, kReleaseVersionUpdatedMessage, 0, reinterpret_cast<LPARAM>(result));
    }).detach();
}

void AppBarWindow::OnLatestReleaseChecked(ReleaseVersionInfo* info) {
    std::unique_ptr<ReleaseVersionInfo> holder(info);
    releaseCheckInFlight_ = false;
    lastReleaseCheckUnixSeconds_ = static_cast<long long>(std::time(nullptr));

    if (info != nullptr) {
        hasReleaseCheckResult_ = info->success;
        releaseCheckErrorMessage_ = info->errorMessage;
        if (info->success) {
            latestReleaseTag_ = info->latestTag;
            updateAvailable_ = CompareVersions(kCurrentVersion, latestReleaseTag_) < 0;
        } else {
            latestReleaseTag_.clear();
            updateAvailable_ = false;
        }
    }

    InvalidateRect(hwnd_, nullptr, FALSE);
}

void AppBarWindow::Paint(HDC hdc) {
    RECT clientRect = {};
    GetClientRect(hwnd_, &clientRect);

    if (RectWidth(clientRect) <= 0 || RectHeight(clientRect) <= 0) {
        return;
    }

    if (FAILED(CreateDeviceResources())) {
        return;
    }

    if (FAILED(renderTarget_->BindDC(hdc, &clientRect))) {
        DiscardDeviceResources();
        return;
    }

    const UINT dpi = GetDpiForWindow(hwnd_);
    renderTarget_->SetDpi(static_cast<float>(dpi), static_cast<float>(dpi));
    renderTarget_->SetTransform(D2D1::Matrix3x2F::Scale(96.0f / dpi, 96.0f / dpi));
    renderTarget_->SetAntialiasMode(D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
    renderTarget_->SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_CLEARTYPE);

    renderTarget_->BeginDraw();
    PaintContent(clientRect);
    const HRESULT hr = renderTarget_->EndDraw();
    if (hr == D2DERR_RECREATE_TARGET) {
        DiscardDeviceResources();
    }
}

void AppBarWindow::PaintContent(const RECT& clientRect) {
    const PaceInfo pace = BuildPaceInfo(snapshot_);
    const int padX = ScaleForDpi(hwnd_, kHorizontalPadding);
    const int padY = ScaleForDpi(hwnd_, kVerticalPadding);
    const int meterHeight = ScaleForDpi(hwnd_, 12);
    const int footerTop = ScaleForDpi(hwnd_, 10);
    const int footerGap = ScaleForDpi(hwnd_, 12);
    const int sectionGap = ScaleForDpi(hwnd_, 10);
    const int heroHeight = ScaleForDpi(hwnd_, 76);
    const int metricRowHeight = ScaleForDpi(hwnd_, 52);

    const COLORREF background = lightTheme_ ? RGB(251, 252, 248) : RGB(24, 28, 25);
    const COLORREF textPrimary = lightTheme_ ? RGB(21, 27, 24) : RGB(240, 244, 241);
    const COLORREF textSecondary = lightTheme_ ? RGB(94, 106, 97) : RGB(167, 178, 171);
    const COLORREF border = lightTheme_ ? RGB(219, 224, 220) : RGB(57, 66, 60);
    const COLORREF shadow = lightTheme_ ? RGB(226, 231, 225) : RGB(10, 14, 12);
    const COLORREF heroBg = pace.valid
        ? (pace.isOver ? (lightTheme_ ? RGB(255, 240, 234) : RGB(60, 34, 28))
                       : (lightTheme_ ? RGB(233, 248, 239) : RGB(27, 48, 36)))
        : (lightTheme_ ? RGB(243, 246, 242) : RGB(35, 40, 37));
    const COLORREF heroValue = pace.valid
        ? (pace.isOver ? (lightTheme_ ? RGB(189, 54, 31) : RGB(255, 144, 120))
                       : (lightTheme_ ? RGB(22, 113, 65) : RGB(118, 216, 163)))
        : textPrimary;
    const COLORREF trackColor = lightTheme_ ? RGB(229, 235, 230) : RGB(68, 76, 71);
    const COLORREF actualBarColor = pace.valid
        ? (pace.isOver ? (lightTheme_ ? RGB(214, 149, 57) : RGB(227, 165, 79))
                       : (lightTheme_ ? RGB(41, 185, 128) : RGB(84, 208, 154)))
        : (lightTheme_ ? RGB(164, 174, 167) : RGB(108, 118, 112));
    auto fillRect = [&](const RECT& rect, COLORREF color) {
        solidBrush_->SetColor(ToColorF(color));
        renderTarget_->FillRectangle(ToRectF(rect), solidBrush_.Get());
    };

    auto drawRectBorder = [&](const RECT& rect, COLORREF color) {
        solidBrush_->SetColor(ToColorF(color));
        renderTarget_->DrawRectangle(ToRectF(rect), solidBrush_.Get(), 1.0f);
    };

    auto drawTextBlock = [&](IDWriteTextFormat* format,
                             const std::wstring& text,
                             const RECT& rect,
                             COLORREF color,
                             DWRITE_TEXT_ALIGNMENT textAlignment,
                             DWRITE_PARAGRAPH_ALIGNMENT paragraphAlignment,
                             DWRITE_WORD_WRAPPING wrapping,
                             bool trimEllipsis) {
        format->SetTextAlignment(textAlignment);
        format->SetParagraphAlignment(paragraphAlignment);
        format->SetWordWrapping(wrapping);

        Microsoft::WRL::ComPtr<IDWriteTextLayout> layout;
        const float layoutWidth = std::max(1.0f, static_cast<float>(RectWidth(rect)));
        const float layoutHeight = std::max(1.0f, static_cast<float>(RectHeight(rect)));
        if (FAILED(dwriteFactory_->CreateTextLayout(
                text.c_str(),
                static_cast<UINT32>(text.size()),
                format,
                layoutWidth,
                layoutHeight,
                layout.GetAddressOf()))) {
            return;
        }

        if (trimEllipsis) {
            Microsoft::WRL::ComPtr<IDWriteInlineObject> ellipsisSign;
            const DWRITE_TRIMMING trimming = { DWRITE_TRIMMING_GRANULARITY_CHARACTER, 0, 0 };
            if (SUCCEEDED(dwriteFactory_->CreateEllipsisTrimmingSign(format, ellipsisSign.GetAddressOf()))) {
                layout->SetTrimming(&trimming, ellipsisSign.Get());
            }
        }

        solidBrush_->SetColor(ToColorF(color));
        renderTarget_->DrawTextLayout(
            D2D1::Point2F(static_cast<float>(rect.left), static_cast<float>(rect.top)),
            layout.Get(),
            solidBrush_.Get(),
            D2D1_DRAW_TEXT_OPTIONS_CLIP);
    };

    auto measureTextWidth = [&](IDWriteTextFormat* format, const std::wstring& text) -> float {
        Microsoft::WRL::ComPtr<IDWriteTextLayout> layout;
        if (FAILED(dwriteFactory_->CreateTextLayout(
                text.c_str(),
                static_cast<UINT32>(text.size()),
                format,
                4096.0f,
                256.0f,
                layout.GetAddressOf()))) {
            return 0.0f;
        }

        DWRITE_TEXT_METRICS metrics = {};
        if (FAILED(layout->GetMetrics(&metrics))) {
            return 0.0f;
        }
        return metrics.widthIncludingTrailingWhitespace;
    };

    if (simpleMode_) {
        fillRect(MakeRect(clientRect.left + 2, clientRect.top + 3, clientRect.right + 2, clientRect.bottom + 3), shadow);
        fillRect(clientRect, background);
        drawRectBorder(clientRect, border);

        const bool exhausted = snapshot_.success &&
            (snapshot_.fiveHour.remainingPercent <= 0 || snapshot_.weekly.remainingPercent <= 0);
        const bool warning = snapshot_.success &&
            !exhausted &&
            (snapshot_.fiveHour.remainingPercent <= 15 || snapshot_.weekly.remainingPercent <= 15 || pace.isOver);
        const wchar_t* statusText = !snapshot_.success
            ? LocalizeText(L"Loading", L"加载中")
            : (exhausted
                ? LocalizeText(L"Exhausted", L"用尽")
                : (warning ? LocalizeText(L"Tight", L"紧张") : LocalizeText(L"Normal", L"正常")));
        const COLORREF statusColor = !snapshot_.success
            ? textSecondary
            : (exhausted ? (lightTheme_ ? RGB(196, 54, 32) : RGB(255, 144, 120))
                         : (warning ? (lightTheme_ ? RGB(184, 121, 38) : RGB(233, 180, 91))
                                    : (lightTheme_ ? RGB(21, 148, 78) : RGB(118, 216, 163))));
        const COLORREF dayCard = lightTheme_ ? RGB(224, 246, 239) : RGB(31, 58, 46);
        const COLORREF weekCard = lightTheme_ ? RGB(239, 247, 226) : RGB(47, 59, 35);
        const std::wstring versionStatusText = GetVersionStatusText(true);
        const int topBandHeight = ScaleForDpi(hwnd_, 34);
        const int innerPad = ScaleForDpi(hwnd_, 12);
        const int cardGap = ScaleForDpi(hwnd_, 10);
        const int footerHeight = ScaleForDpi(hwnd_, 16);

        RECT titleRect = MakeRect(clientRect.left + innerPad, clientRect.top + ScaleForDpi(hwnd_, 6),
            clientRect.right - innerPad - ScaleForDpi(hwnd_, 66), clientRect.top + topBandHeight);
        RECT statusRect = MakeRect(clientRect.right - innerPad - ScaleForDpi(hwnd_, 54), clientRect.top + ScaleForDpi(hwnd_, 8),
            clientRect.right - innerPad, clientRect.top + ScaleForDpi(hwnd_, 28));
        drawTextBlock(textFormatMetricValue_.Get(), LocalizeText(L"Usage", L"额度用量"), titleRect, textPrimary,
            DWRITE_TEXT_ALIGNMENT_LEADING, DWRITE_PARAGRAPH_ALIGNMENT_CENTER, DWRITE_WORD_WRAPPING_NO_WRAP, true);
        drawTextBlock(textFormatMetricLabel_.Get(), statusText, statusRect, statusColor,
            DWRITE_TEXT_ALIGNMENT_TRAILING, DWRITE_PARAGRAPH_ALIGNMENT_CENTER, DWRITE_WORD_WRAPPING_NO_WRAP, false);

        RECT cardsRect = MakeRect(clientRect.left + innerPad, clientRect.top + topBandHeight + ScaleForDpi(hwnd_, 2),
            clientRect.right - innerPad, clientRect.bottom - footerHeight - ScaleForDpi(hwnd_, 4));
        const int cardWidth = (RectWidth(cardsRect) - cardGap) / 2;
        RECT dayRect = MakeRect(cardsRect.left, cardsRect.top, cardsRect.left + cardWidth, cardsRect.bottom);
        RECT weekRect = MakeRect(dayRect.right + cardGap, cardsRect.top, cardsRect.right, cardsRect.bottom);
        fillRect(dayRect, dayCard);
        fillRect(weekRect, weekCard);

        const std::wstring dayValue = snapshot_.success ? FormatPercent(snapshot_.fiveHour.usedPercent) : L"--";
        const std::wstring weekValue = snapshot_.success ? FormatPercent(snapshot_.weekly.usedPercent) : L"--";
        RECT dayLabelRect = MakeRect(dayRect.left + innerPad, dayRect.top + ScaleForDpi(hwnd_, 8), dayRect.right - innerPad, dayRect.top + ScaleForDpi(hwnd_, 24));
        RECT dayValueRect = MakeRect(dayRect.left + innerPad, dayRect.top + ScaleForDpi(hwnd_, 24), dayRect.right - innerPad, dayRect.bottom - ScaleForDpi(hwnd_, 8));
        RECT weekLabelRect = MakeRect(weekRect.left + innerPad, weekRect.top + ScaleForDpi(hwnd_, 8), weekRect.right - innerPad, weekRect.top + ScaleForDpi(hwnd_, 24));
        RECT weekValueRect = MakeRect(weekRect.left + innerPad, weekRect.top + ScaleForDpi(hwnd_, 24), weekRect.right - innerPad, weekRect.bottom - ScaleForDpi(hwnd_, 8));
        drawTextBlock(textFormatMetricLabel_.Get(), LocalizeText(L"5 Hours", L"5小时"), dayLabelRect, textSecondary,
            DWRITE_TEXT_ALIGNMENT_LEADING, DWRITE_PARAGRAPH_ALIGNMENT_NEAR, DWRITE_WORD_WRAPPING_NO_WRAP, false);
        drawTextBlock(textFormatDelta_.Get(), dayValue, dayValueRect, textPrimary,
            DWRITE_TEXT_ALIGNMENT_LEADING, DWRITE_PARAGRAPH_ALIGNMENT_CENTER, DWRITE_WORD_WRAPPING_NO_WRAP, false);
        drawTextBlock(textFormatMetricLabel_.Get(), LocalizeText(L"This Week", L"本周"), weekLabelRect, textSecondary,
            DWRITE_TEXT_ALIGNMENT_LEADING, DWRITE_PARAGRAPH_ALIGNMENT_NEAR, DWRITE_WORD_WRAPPING_NO_WRAP, false);
        drawTextBlock(textFormatDelta_.Get(), weekValue, weekValueRect, textPrimary,
            DWRITE_TEXT_ALIGNMENT_LEADING, DWRITE_PARAGRAPH_ALIGNMENT_CENTER, DWRITE_WORD_WRAPPING_NO_WRAP, false);

        const std::wstring refreshTimeText = lastSuccessfulRefreshUnixSeconds_ > 0
            ? FormatClockTime(lastSuccessfulRefreshUnixSeconds_)
            : L"--";
        const std::wstring refreshCountdownText = refreshInFlight_
            ? std::wstring(LocalizeText(L"Refreshing", L"刷新中"))
            : FormatRefreshCountdown(refreshCountdownSeconds_);
        RECT footerLeftRect = MakeRect(clientRect.left + innerPad, clientRect.bottom - footerHeight - ScaleForDpi(hwnd_, 1),
            clientRect.right / 2, clientRect.bottom - ScaleForDpi(hwnd_, 1));
        RECT footerRightRect = MakeRect(clientRect.right / 2, clientRect.bottom - footerHeight - ScaleForDpi(hwnd_, 1),
            clientRect.right - innerPad, clientRect.bottom - ScaleForDpi(hwnd_, 1));
        drawTextBlock(textFormatFoot_.Get(), versionStatusText, footerLeftRect, updateAvailable_ ? heroValue : textSecondary,
            DWRITE_TEXT_ALIGNMENT_LEADING, DWRITE_PARAGRAPH_ALIGNMENT_CENTER, DWRITE_WORD_WRAPPING_NO_WRAP, false);
        drawTextBlock(textFormatFoot_.Get(), refreshCountdownText, footerRightRect, textSecondary,
            DWRITE_TEXT_ALIGNMENT_TRAILING, DWRITE_PARAGRAPH_ALIGNMENT_CENTER, DWRITE_WORD_WRAPPING_NO_WRAP, false);
        return;
    }

    RECT heroRect = MakeRect(clientRect.left, clientRect.top, clientRect.right, clientRect.top + heroHeight);
    fillRect(MakeRect(clientRect.left + 2, clientRect.top + 3, clientRect.right + 2, clientRect.bottom + 3), shadow);
    fillRect(clientRect, background);
    drawRectBorder(clientRect, border);
    fillRect(heroRect, heroBg);
    fillRect(MakeRect(heroRect.left, heroRect.bottom, heroRect.right, heroRect.bottom + 1), border);

    if (!snapshot_.success || !pace.valid) {
        RECT kickerRect = MakeRect(heroRect.left + padX, heroRect.top + padY + ScaleForDpi(hwnd_, 2),
            heroRect.right - padX, heroRect.top + padY + ScaleForDpi(hwnd_, 20));
        drawTextBlock(textFormatKicker_.Get(), LocalizeText(L"Codex Usage Budget", L"Codex 用量预算"), kickerRect, textSecondary,
            DWRITE_TEXT_ALIGNMENT_LEADING, DWRITE_PARAGRAPH_ALIGNMENT_CENTER, DWRITE_WORD_WRAPPING_NO_WRAP, false);

        RECT titleRect = MakeRect(heroRect.left + padX, kickerRect.bottom + ScaleForDpi(hwnd_, 6),
            heroRect.right - padX, heroRect.bottom - padY);
        drawTextBlock(textFormatTitle_.Get(), LocalizeText(L"Loading usage data", L"正在加载用量信息"), titleRect, textPrimary,
            DWRITE_TEXT_ALIGNMENT_LEADING, DWRITE_PARAGRAPH_ALIGNMENT_NEAR, DWRITE_WORD_WRAPPING_WRAP, false);

        if (!snapshot_.errorMessage.empty()) {
            RECT errorRect = MakeRect(clientRect.left + padX, heroRect.bottom + sectionGap, clientRect.right - padX, clientRect.bottom - padY);
            drawTextBlock(textFormatFoot_.Get(), snapshot_.errorMessage, errorRect, RGB(215, 73, 73),
                DWRITE_TEXT_ALIGNMENT_LEADING, DWRITE_PARAGRAPH_ALIGNMENT_NEAR, DWRITE_WORD_WRAPPING_WRAP, false);
        }

        RECT versionRect = MakeRect(clientRect.left + padX, clientRect.bottom - ScaleForDpi(hwnd_, 22),
            clientRect.left + padX + ScaleForDpi(hwnd_, 220), clientRect.bottom - ScaleForDpi(hwnd_, 6));
        drawTextBlock(textFormatFoot_.Get(), GetVersionStatusText(true), versionRect, updateAvailable_ ? heroValue : textSecondary,
            DWRITE_TEXT_ALIGNMENT_LEADING, DWRITE_PARAGRAPH_ALIGNMENT_CENTER, DWRITE_WORD_WRAPPING_NO_WRAP, false);
        return;
    }

    RECT kickerRect = MakeRect(heroRect.left + padX, heroRect.top + padY + ScaleForDpi(hwnd_, 2),
        heroRect.right - padX, heroRect.top + padY + ScaleForDpi(hwnd_, 20));
    drawTextBlock(textFormatKicker_.Get(), LocalizeText(L"Weekly quota pacing", L"每周限额进度"), kickerRect, textSecondary,
        DWRITE_TEXT_ALIGNMENT_LEADING, DWRITE_PARAGRAPH_ALIGNMENT_CENTER, DWRITE_WORD_WRAPPING_NO_WRAP, false);

    const wchar_t* titleText = pace.isOver
        ? LocalizeText(L"Currently above the average pace", L"当前已超过平均进度")
        : LocalizeText(L"Currently below the average pace", L"当前低于平均进度");
    const std::wstring deltaText = (pace.deltaPercent >= 0.0 ? L"+" : L"-") + FormatPercent(std::fabs(pace.deltaPercent));

    RECT titleRect = MakeRect(heroRect.left + padX, kickerRect.bottom + ScaleForDpi(hwnd_, 4),
        heroRect.right - ScaleForDpi(hwnd_, 214), heroRect.bottom - padY + ScaleForDpi(hwnd_, 2));
    drawTextBlock(textFormatTitle_.Get(), titleText, titleRect, textPrimary,
        DWRITE_TEXT_ALIGNMENT_LEADING, DWRITE_PARAGRAPH_ALIGNMENT_CENTER, DWRITE_WORD_WRAPPING_NO_WRAP, true);

    RECT deltaRect = MakeRect(heroRect.right - ScaleForDpi(hwnd_, 204), heroRect.top + padY,
        heroRect.right - padX, heroRect.bottom - padY);
    drawTextBlock(textFormatDelta_.Get(), deltaText, deltaRect, heroValue,
        DWRITE_TEXT_ALIGNMENT_TRAILING, DWRITE_PARAGRAPH_ALIGNMENT_CENTER, DWRITE_WORD_WRAPPING_NO_WRAP, false);

    const int metricsTop = heroRect.bottom + 1;
    RECT metricsRect = MakeRect(clientRect.left, metricsTop, clientRect.right, metricsTop + metricRowHeight);
    const wchar_t* metricLabels[4] = {
        LocalizeText(L"Daily average budget", L"今日平均预算"),
        LocalizeText(L"Expected used today", L"今日可累计使用"),
        LocalizeText(L"5-hour remaining", L"5小时剩余"),
        LocalizeText(L"Weekly remaining", L"每周剩余")
    };
    const std::wstring metricValues[4] = {
        FormatPercent(pace.dailyBudgetPercent),
        FormatPercent(pace.expectedUsedPercent),
        FormatPercent(snapshot_.fiveHour.remainingPercent),
        FormatPercent(pace.weeklyRemainingPercent),
    };
    const int metricWidth = RectWidth(metricsRect) / 4;
    for (int i = 0; i < 4; ++i) {
        RECT metricRect = MakeRect(
            metricsRect.left + i * metricWidth,
            metricsRect.top,
            i == 3 ? metricsRect.right : metricsRect.left + (i + 1) * metricWidth,
            metricsRect.bottom);
        if (i != 0) {
            fillRect(MakeRect(metricRect.left, metricRect.top, metricRect.left + 1, metricRect.bottom), border);
        }

        RECT labelRect = MakeRect(metricRect.left + padX, metricRect.top + ScaleForDpi(hwnd_, 8),
            metricRect.right - padX, metricRect.top + ScaleForDpi(hwnd_, 22));
        drawTextBlock(textFormatMetricLabel_.Get(), metricLabels[i], labelRect, textSecondary,
            DWRITE_TEXT_ALIGNMENT_LEADING, DWRITE_PARAGRAPH_ALIGNMENT_NEAR, DWRITE_WORD_WRAPPING_NO_WRAP, true);

        RECT valueRect = MakeRect(metricRect.left + padX, metricRect.top + ScaleForDpi(hwnd_, 22),
            metricRect.right - padX, metricRect.bottom - ScaleForDpi(hwnd_, 6));
        drawTextBlock(textFormatMetricValue_.Get(), metricValues[i], valueRect, textPrimary,
            DWRITE_TEXT_ALIGNMENT_LEADING, DWRITE_PARAGRAPH_ALIGNMENT_CENTER, DWRITE_WORD_WRAPPING_NO_WRAP, false);
    }

    const int meterLeft = clientRect.left + padX;
    const int meterRight = clientRect.right - padX;
    const int meterHeadTop = metricsRect.bottom + sectionGap;
    const std::wstring meterHint = language_ == Language::Chinese
        ? (L"绿色条是实际已用，黑线是按周期第 " + std::to_wstring(pace.cycleDay) + L" 天应到预算")
        : (L"Green is actual usage, black line is the expected budget by day " + std::to_wstring(pace.cycleDay));
    const std::wstring meterStat = FormatPercent(pace.actualUsedPercent) + L" / " + FormatPercent(pace.expectedUsedPercent);

    RECT meterHeadLeft = MakeRect(meterLeft, meterHeadTop, meterRight - ScaleForDpi(hwnd_, 186), meterHeadTop + ScaleForDpi(hwnd_, 24));
    RECT meterHeadRight = MakeRect(meterRight - ScaleForDpi(hwnd_, 176), meterHeadTop, meterRight, meterHeadTop + ScaleForDpi(hwnd_, 18));
    drawTextBlock(textFormatMetricLabel_.Get(), meterHint, meterHeadLeft, textSecondary,
        DWRITE_TEXT_ALIGNMENT_LEADING, DWRITE_PARAGRAPH_ALIGNMENT_NEAR, DWRITE_WORD_WRAPPING_NO_WRAP, true);
    drawTextBlock(textFormatMetricLabel_.Get(), meterStat, meterHeadRight, textPrimary,
        DWRITE_TEXT_ALIGNMENT_TRAILING, DWRITE_PARAGRAPH_ALIGNMENT_NEAR, DWRITE_WORD_WRAPPING_NO_WRAP, false);
    const int meterInfoBottom = meterHeadLeft.bottom;

    RECT meterRect = MakeRect(meterLeft, meterInfoBottom + ScaleForDpi(hwnd_, 6),
        meterRight, meterInfoBottom + ScaleForDpi(hwnd_, 6) + meterHeight);
    fillRect(meterRect, trackColor);

    RECT actualRect = meterRect;
    actualRect.right = actualRect.left + static_cast<int>(RectWidth(meterRect) * ClampDouble(pace.actualUsedPercent, 0.0, 100.0) / 100.0);
    if (actualRect.right > actualRect.left) {
        fillRect(actualRect, actualBarColor);
    }

    const int markerX = meterRect.left + static_cast<int>(RectWidth(meterRect) * ClampDouble(pace.expectedUsedPercent, 0.0, 100.0) / 100.0);
    fillRect(
        MakeRect(markerX - 1, meterRect.top - ScaleForDpi(hwnd_, 3), markerX + 1, meterRect.bottom + ScaleForDpi(hwnd_, 6)),
        lightTheme_ ? RGB(21, 27, 24) : RGB(240, 244, 241));

    RECT footerLine = MakeRect(clientRect.left, meterRect.bottom + footerTop, clientRect.right, meterRect.bottom + footerTop + 1);
    fillRect(footerLine, border);
    const std::wstring versionStatusText = GetVersionStatusText(true);
    const std::wstring footerItems[6] = {
        std::wstring(LocalizeText(L"Week start: ", L"本周开始: ")) + FormatDateTime(pace.weekStartUnixSeconds),
        std::wstring(LocalizeText(L"Reset at: ", L"重置时间: ")) + FormatDateTime(snapshot_.weekly.resetAtUnixSeconds),
        language_ == Language::Chinese
            ? (L"当前: 第 " + std::to_wstring(pace.cycleDay) + L" 天")
            : (L"Current: Day " + std::to_wstring(pace.cycleDay)),
        std::wstring(LocalizeText(L"Elapsed: ", L"已用时间: ")) + FormatDuration(pace.elapsedSeconds),
        std::wstring(LocalizeText(L"Remaining: ", L"剩余时间: ")) + FormatDuration(pace.remainingSeconds),
        language_ == Language::Chinese
            ? (L"5 小时限额: " + FormatPercent(snapshot_.fiveHour.usedPercent) + L" 已用，" + FormatPercent(snapshot_.fiveHour.remainingPercent) + L" 剩余")
            : (L"5-hour quota: " + FormatPercent(snapshot_.fiveHour.usedPercent) + L" used, " + FormatPercent(snapshot_.fiveHour.remainingPercent) + L" remaining"),
    };

    const std::wstring refreshTimeText = lastSuccessfulRefreshUnixSeconds_ > 0
        ? (std::wstring(LocalizeText(L"Refresh: ", L"刷新: ")) + FormatClockTime(lastSuccessfulRefreshUnixSeconds_))
        : (std::wstring(LocalizeText(L"Refresh: ", L"刷新: ")) + L"--");
    std::wstring refreshCountdownText;
    if (refreshInFlight_) {
        refreshCountdownText = std::wstring(LocalizeText(L"Countdown: Refreshing...", L"倒计时: 刷新中..."));
    } else {
        refreshCountdownText = std::wstring(LocalizeText(L"Countdown: ", L"倒计时: ")) + FormatRefreshCountdown(refreshCountdownSeconds_);
    }

    const int refreshInfoWidth = std::max(
        static_cast<int>(std::ceil(measureTextWidth(textFormatFoot_.Get(), refreshTimeText))),
        static_cast<int>(std::ceil(measureTextWidth(textFormatFoot_.Get(), refreshCountdownText)))) + ScaleForDpi(hwnd_, 8);
    const int versionInfoWidth = static_cast<int>(std::ceil(measureTextWidth(textFormatFoot_.Get(), versionStatusText))) + ScaleForDpi(hwnd_, 8);
    const int versionInfoLeft = clientRect.left + padX;
    const int refreshInfoLeft = std::max(static_cast<int>(clientRect.left) + padX,
        static_cast<int>(clientRect.right) - padX - refreshInfoWidth);
    const int footerContentLeft = clientRect.left + padX;
    const int footerContentRight = std::max(static_cast<int>(footerContentLeft), refreshInfoLeft - footerGap);

    int footX = footerContentLeft;
    int footY = footerLine.bottom + ScaleForDpi(hwnd_, 6);
    for (const std::wstring& item : footerItems) {
        const float itemWidth = measureTextWidth(textFormatFoot_.Get(), item);
        if (footX + itemWidth > footerContentRight) {
            footX = footerContentLeft;
            footY += ScaleForDpi(hwnd_, 18);
        }
        RECT itemRect = MakeRect(footX, footY, footX + static_cast<int>(std::ceil(itemWidth)) + ScaleForDpi(hwnd_, 4), footY + ScaleForDpi(hwnd_, 16));
        drawTextBlock(textFormatFoot_.Get(), item, itemRect, textSecondary,
            DWRITE_TEXT_ALIGNMENT_LEADING, DWRITE_PARAGRAPH_ALIGNMENT_NEAR, DWRITE_WORD_WRAPPING_NO_WRAP, false);
        footX += static_cast<int>(std::ceil(itemWidth)) + footerGap;
    }

    const int refreshInfoTop = footerLine.bottom + ScaleForDpi(hwnd_, 6);
    const int versionRectBottom = clientRect.bottom - ScaleForDpi(hwnd_, 4);
    RECT versionRect = MakeRect(versionInfoLeft, versionRectBottom - ScaleForDpi(hwnd_, 16),
        versionInfoLeft + versionInfoWidth, versionRectBottom);
    RECT refreshTimeRect = MakeRect(refreshInfoLeft, refreshInfoTop, clientRect.right - padX, refreshInfoTop + ScaleForDpi(hwnd_, 16));
    RECT refreshCountdownRect = MakeRect(refreshInfoLeft, refreshTimeRect.bottom + ScaleForDpi(hwnd_, 2),
        clientRect.right - padX, refreshTimeRect.bottom + ScaleForDpi(hwnd_, 18));
    drawTextBlock(textFormatFoot_.Get(), versionStatusText, versionRect, updateAvailable_ ? heroValue : textSecondary,
        DWRITE_TEXT_ALIGNMENT_LEADING, DWRITE_PARAGRAPH_ALIGNMENT_NEAR, DWRITE_WORD_WRAPPING_NO_WRAP, false);
    drawTextBlock(textFormatFoot_.Get(), refreshTimeText, refreshTimeRect, textSecondary,
        DWRITE_TEXT_ALIGNMENT_TRAILING, DWRITE_PARAGRAPH_ALIGNMENT_NEAR, DWRITE_WORD_WRAPPING_NO_WRAP, false);
    drawTextBlock(textFormatFoot_.Get(), refreshCountdownText, refreshCountdownRect, textSecondary,
        DWRITE_TEXT_ALIGNMENT_TRAILING, DWRITE_PARAGRAPH_ALIGNMENT_NEAR, DWRITE_WORD_WRAPPING_NO_WRAP, false);

    if (!lockPosition_) {
        RECT gripRect = MakeRect(clientRect.right - ScaleForDpi(hwnd_, 30), clientRect.bottom - ScaleForDpi(hwnd_, 18),
            clientRect.right - ScaleForDpi(hwnd_, 8), clientRect.bottom - ScaleForDpi(hwnd_, 6));
        drawTextBlock(textFormatFoot_.Get(), L"///", gripRect, lightTheme_ ? RGB(147, 156, 149) : RGB(117, 126, 120),
            DWRITE_TEXT_ALIGNMENT_TRAILING, DWRITE_PARAGRAPH_ALIGNMENT_FAR, DWRITE_WORD_WRAPPING_NO_WRAP, false);
    }
}

void AppBarWindow::ShowContextMenu(POINT screenPoint) {
    HMENU menu = CreatePopupMenu();
    HMENU languageMenu = CreatePopupMenu();
    HMENU refreshIntervalMenu = CreatePopupMenu();
    const bool launchAtStartup = IsLaunchAtStartupEnabled();
    AppendMenuW(languageMenu, MF_STRING | (language_ == Language::English ? MF_CHECKED : MF_UNCHECKED),
        kCommandLanguageEnglish, L"English");
    AppendMenuW(languageMenu, MF_STRING | (language_ == Language::Chinese ? MF_CHECKED : MF_UNCHECKED),
        kCommandLanguageChinese, L"中文");
    AppendMenuW(refreshIntervalMenu, MF_STRING | (refreshIntervalSeconds_ == 60 ? MF_CHECKED : MF_UNCHECKED),
        kCommandRefreshInterval1Minute, LocalizeText(L"1 minute", L"1分钟"));
    AppendMenuW(refreshIntervalMenu, MF_STRING | (refreshIntervalSeconds_ == 180 ? MF_CHECKED : MF_UNCHECKED),
        kCommandRefreshInterval3Minutes, LocalizeText(L"3 minutes", L"3分钟"));
    AppendMenuW(refreshIntervalMenu, MF_STRING | (refreshIntervalSeconds_ == 300 ? MF_CHECKED : MF_UNCHECKED),
        kCommandRefreshInterval5Minutes, LocalizeText(L"5 minutes", L"5分钟"));
    AppendMenuW(refreshIntervalMenu, MF_STRING | (refreshIntervalSeconds_ == 600 ? MF_CHECKED : MF_UNCHECKED),
        kCommandRefreshInterval10Minutes, LocalizeText(L"10 minutes", L"10分钟"));
    AppendMenuW(refreshIntervalMenu, MF_STRING | (refreshIntervalSeconds_ == 1800 ? MF_CHECKED : MF_UNCHECKED),
        kCommandRefreshInterval30Minutes, LocalizeText(L"30 minutes", L"30分钟"));

    AppendMenuW(menu, MF_STRING, kCommandRefresh, LocalizeText(L"Refresh now", L"立即刷新"));
    AppendMenuW(menu, MF_STRING, kCommandCheckVersion, LocalizeText(L"Check version", L"检查版本"));
    AppendMenuW(menu, MF_POPUP, reinterpret_cast<UINT_PTR>(refreshIntervalMenu), LocalizeText(L"Refresh interval", L"刷新间隔"));
    AppendMenuW(menu, MF_STRING | (launchAtStartup ? MF_CHECKED : MF_UNCHECKED),
        kCommandLaunchAtStartup, LocalizeText(L"Launch at startup", L"开机自启"));
    AppendMenuW(menu, MF_STRING | (alwaysOnTop_ ? MF_CHECKED : MF_UNCHECKED),
        kCommandAlwaysOnTop, LocalizeText(L"Always on top", L"始终置顶"));
    AppendMenuW(menu, MF_STRING | (lockPosition_ ? MF_CHECKED : MF_UNCHECKED),
        kCommandLockPosition, LocalizeText(L"Lock position", L"固定位置"));
    AppendMenuW(menu, MF_STRING | (simpleMode_ ? MF_CHECKED : MF_UNCHECKED),
        kCommandSimpleMode, LocalizeText(L"Simple mode", L"简单模式"));
    AppendMenuW(menu, MF_POPUP, reinterpret_cast<UINT_PTR>(languageMenu), LocalizeText(L"Language", L"语言"));
    AppendMenuW(menu, MF_STRING, kCommandResetPosition, LocalizeText(L"Reset widget position", L"重置组件位置"));
    AppendMenuW(menu, MF_SEPARATOR, 0, nullptr);
    AppendMenuW(menu, MF_STRING, kCommandExit, LocalizeText(L"Exit", L"退出"));

    const UINT command = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON, screenPoint.x, screenPoint.y, 0, hwnd_, nullptr);
    DestroyMenu(menu);

    if (command == kCommandRefresh) {
        RequestRefresh(true);
    } else if (command == kCommandCheckVersion) {
        RequestLatestReleaseCheck(true);
    } else if (command == kCommandRefreshInterval1Minute) {
        SetRefreshIntervalSeconds(60);
    } else if (command == kCommandRefreshInterval3Minutes) {
        SetRefreshIntervalSeconds(180);
    } else if (command == kCommandRefreshInterval5Minutes) {
        SetRefreshIntervalSeconds(300);
    } else if (command == kCommandRefreshInterval10Minutes) {
        SetRefreshIntervalSeconds(600);
    } else if (command == kCommandRefreshInterval30Minutes) {
        SetRefreshIntervalSeconds(1800);
    } else if (command == kCommandLaunchAtStartup) {
        SetLaunchAtStartupEnabled(!launchAtStartup);
    } else if (command == kCommandAlwaysOnTop) {
        alwaysOnTop_ = !alwaysOnTop_;
        UpdateWindowBounds(true);
        SaveSettings();
    } else if (command == kCommandLockPosition) {
        lockPosition_ = !lockPosition_;
        SaveSettings();
    } else if (command == kCommandSimpleMode) {
        simpleMode_ = !simpleMode_;
        UpdateWindowBounds(true);
        SaveSettings();
        InvalidateRect(hwnd_, nullptr, TRUE);
    } else if (command == kCommandLanguageEnglish) {
        SetLanguage(Language::English);
    } else if (command == kCommandLanguageChinese) {
        SetLanguage(Language::Chinese);
    } else if (command == kCommandResetPosition) {
        hasSavedRect_ = false;
        UpdateWindowBounds(false);
        SaveSettings();
    } else if (command == kCommandExit) {
        DestroyWindow(hwnd_);
    }
}

std::wstring AppBarWindow::FormatDuration(int totalSeconds) const {
    const int days = totalSeconds / 86400;
    const int hours = (totalSeconds % 86400) / 3600;

    if (days > 0) {
        if (language_ == Language::Chinese) {
            return std::to_wstring(days) + L" 天 " + std::to_wstring(hours) + L" 小时";
        }
        return std::to_wstring(days) + L"d " + std::to_wstring(hours) + L"h";
    }
    const int minutes = (totalSeconds % 3600) / 60;
    if (language_ == Language::Chinese) {
        return std::to_wstring(hours) + L" 小时 " + std::to_wstring(minutes) + L" 分钟";
    }
    return std::to_wstring(hours) + L"h " + std::to_wstring(minutes) + L"m";
}

std::wstring AppBarWindow::FormatRefreshCountdown(int totalSeconds) const {
    const int hours = totalSeconds / 3600;
    const int minutes = (totalSeconds % 3600) / 60;
    const int seconds = totalSeconds % 60;

    if (language_ == Language::Chinese) {
        if (hours > 0) {
            return std::to_wstring(hours) + L"小时 " + std::to_wstring(minutes) + L"分";
        }
        if (minutes > 0) {
            return std::to_wstring(minutes) + L"分 " + std::to_wstring(seconds) + L"秒";
        }
        return std::to_wstring(seconds) + L"秒";
    }

    if (hours > 0) {
        return std::to_wstring(hours) + L"h " + std::to_wstring(minutes) + L"m";
    }
    if (minutes > 0) {
        return std::to_wstring(minutes) + L"m " + std::to_wstring(seconds) + L"s";
    }
    return std::to_wstring(seconds) + L"s";
}

std::wstring AppBarWindow::FormatDateTime(long long unixSeconds) const {
    if (unixSeconds <= 0) {
        return L"--";
    }

    std::time_t t = static_cast<std::time_t>(unixSeconds);
    std::tm localTime = {};
    localtime_s(&localTime, &t);

    wchar_t buffer[64] = {};
    wcsftime(buffer, sizeof(buffer) / sizeof(buffer[0]), L"%m/%d %H:%M", &localTime);
    return buffer;
}

std::wstring AppBarWindow::FormatClockTime(long long unixSeconds) const {
    if (unixSeconds <= 0) {
        return L"--";
    }

    std::time_t t = static_cast<std::time_t>(unixSeconds);
    std::tm localTime = {};
    localtime_s(&localTime, &t);

    wchar_t buffer[64] = {};
    wcsftime(buffer, sizeof(buffer) / sizeof(buffer[0]), L"%H:%M:%S", &localTime);
    return buffer;
}

std::wstring AppBarWindow::FormatPercent(double value) const {
    return FormatNumber(value) + L"%";
}
