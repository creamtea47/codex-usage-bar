#include "AppBarWindow.h"

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

namespace {

constexpr wchar_t kWindowClassName[] = L"CodexUsageBarWindow";
constexpr int kLayoutVersion = 5;
constexpr int kDefaultWidgetWidth = 820;
constexpr int kMinimumWidgetWidth = 640;
constexpr int kDesktopMargin = 18;
constexpr int kHorizontalPadding = 12;
constexpr int kVerticalPadding = 10;
constexpr int kResizeGrip = 12;
constexpr long long kDaySeconds = 24LL * 60 * 60;
constexpr long long kWeekSeconds = 7LL * kDaySeconds;

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

int CalculateMinimumWidgetHeight(HWND hwnd, int width) {
    const int heroHeight = ScaleForDpi(hwnd, 76);
    const int metricsHeight = ScaleForDpi(hwnd, 52);
    const int meterInfoHeight = ScaleForDpi(hwnd, 30);
    const int footerRows = width >= ScaleForDpi(hwnd, 1180) ? 1 : 2;
    const int footerHeight = ScaleForDpi(hwnd, 8) + footerRows * ScaleForDpi(hwnd, 18) + ScaleForDpi(hwnd, 14);

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

AppBarWindow::~AppBarWindow() = default;

bool AppBarWindow::Create() {
    RegisterWindowClass();
    LoadSettings();
    hwnd_ = CreateWindowExW(
        WS_EX_TOOLWINDOW,
        kWindowClassName,
        L"Codex Usage Widget",
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

    RefreshTheme();
    UpdateWindowBounds(true);
    ShowWindow(hwnd_, SW_SHOW);
    UpdateWindow(hwnd_);
    InvalidateRect(hwnd_, nullptr, TRUE);

    SetTimer(hwnd_, kCountdownTimerId, 1000, nullptr);
    SetTimer(hwnd_, kRefreshTimerId, 60000, nullptr);
    RequestRefresh(true);
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
                InvalidateRect(hwnd_, nullptr, FALSE);
            } else if (wParam == kRefreshTimerId) {
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

        case WM_DESTROY:
            KillTimer(hwnd_, kCountdownTimerId);
            KillTimer(hwnd_, kRefreshTimerId);
            SaveSettings();
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

RECT AppBarWindow::BuildDefaultRect(const RECT& desktopRect) const {
    const int margin = ScaleForDpi(hwnd_, kDesktopMargin);
    const int width = ScaleForDpi(hwnd_, kDefaultWidgetWidth);
    const int height = CalculateMinimumWidgetHeight(hwnd_, width);
    RECT rect = {};
    rect.right = std::max(width + margin, static_cast<int>(desktopRect.right) - margin);
    rect.left = std::max(static_cast<int>(desktopRect.left) + margin, static_cast<int>(rect.right) - width);
    rect.bottom = std::max(height + margin, static_cast<int>(desktopRect.bottom) - margin);
    rect.top = std::max(static_cast<int>(desktopRect.top) + margin, static_cast<int>(rect.bottom) - height);
    return rect;
}

RECT AppBarWindow::ClampRectToDesktop(RECT rect) const {
    const RECT desktopRect = GetDesktopClientRect();
    const int minWidth = ScaleForDpi(hwnd_, kMinimumWidgetWidth);
    const int minHeight = CalculateMinimumWidgetHeight(hwnd_, std::max(RectWidth(rect), minWidth));

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
    SetWindowPos(hwnd_, HWND_TOPMOST, rect.left, rect.top, RectWidth(rect), RectHeight(rect), SWP_NOACTIVATE);
    if (!usingPersistedRect) {
        SaveSettings();
    }
}

void AppBarWindow::LoadSettings() {
    const std::wstring path = GetSettingsPath();
    const int version = GetPrivateProfileIntW(L"layout", L"layout_version", 0, path.c_str());
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

AppBarWindow::DragMode AppBarWindow::HitTestDragMode(POINT clientPoint) const {
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
    }
    InvalidateRect(hwnd_, nullptr, FALSE);
}

void AppBarWindow::Paint(HDC hdc) {
    RECT clientRect = {};
    GetClientRect(hwnd_, &clientRect);

    if (RectWidth(clientRect) <= 0 || RectHeight(clientRect) <= 0) {
        return;
    }

    HDC memoryDc = CreateCompatibleDC(hdc);
    HBITMAP bitmap = CreateCompatibleBitmap(hdc, RectWidth(clientRect), RectHeight(clientRect));
    HGDIOBJ oldBitmap = SelectObject(memoryDc, bitmap);

    const COLORREF background = lightTheme_ ? RGB(251, 252, 248) : RGB(24, 28, 25);
    const COLORREF border = lightTheme_ ? RGB(216, 224, 216) : RGB(56, 64, 59);
    const COLORREF shadow = lightTheme_ ? RGB(226, 231, 225) : RGB(10, 14, 12);

    RECT shadowRect = clientRect;
    OffsetRect(&shadowRect, 2, 3);
    FillSolidRect(memoryDc, shadowRect, shadow);

    FillSolidRect(memoryDc, clientRect, background);

    HPEN borderPen = CreatePen(PS_SOLID, 1, border);
    HGDIOBJ oldPen = SelectObject(memoryDc, borderPen);
    HGDIOBJ oldBrush = SelectObject(memoryDc, GetStockObject(HOLLOW_BRUSH));
    RoundRect(memoryDc, clientRect.left, clientRect.top, clientRect.right, clientRect.bottom, 18, 18);
    SelectObject(memoryDc, oldBrush);
    SelectObject(memoryDc, oldPen);
    DeleteObject(borderPen);

    PaintContent(memoryDc, clientRect);
    BitBlt(hdc, 0, 0, RectWidth(clientRect), RectHeight(clientRect), memoryDc, 0, 0, SRCCOPY);

    SelectObject(memoryDc, oldBitmap);
    DeleteObject(bitmap);
    DeleteDC(memoryDc);
}

void AppBarWindow::PaintContent(HDC hdc, const RECT& clientRect) {
    SetBkMode(hdc, TRANSPARENT);
    const PaceInfo pace = BuildPaceInfo(snapshot_);
    const int padX = ScaleForDpi(hwnd_, kHorizontalPadding);
    const int padY = ScaleForDpi(hwnd_, kVerticalPadding);
    const int meterHeight = ScaleForDpi(hwnd_, 12);
    const int footerTop = ScaleForDpi(hwnd_, 10);
    const int footerGap = ScaleForDpi(hwnd_, 12);
    const int sectionGap = ScaleForDpi(hwnd_, 10);
    const int width = RectWidth(clientRect);
    const int heroHeight = ScaleForDpi(hwnd_, 76);
    const int metricRowHeight = ScaleForDpi(hwnd_, 52);

    const COLORREF textPrimary = lightTheme_ ? RGB(21, 27, 24) : RGB(240, 244, 241);
    const COLORREF textSecondary = lightTheme_ ? RGB(94, 106, 97) : RGB(167, 178, 171);
    const COLORREF border = lightTheme_ ? RGB(219, 224, 220) : RGB(57, 66, 60);
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

    auto makeFont = [](int size, int weight) -> HFONT {
        return CreateFontW(-size, 0, 0, 0, weight, FALSE, FALSE, FALSE, DEFAULT_CHARSET,
            OUT_DEFAULT_PRECIS, CLIP_DEFAULT_PRECIS, CLEARTYPE_QUALITY, VARIABLE_PITCH, L"Segoe UI");
    };

    HFONT fontKicker = makeFont(ScaleForDpi(hwnd_, 12), FW_NORMAL);
    HFONT fontTitle = makeFont(ScaleForDpi(hwnd_, 18), FW_SEMIBOLD);
    HFONT fontDelta = makeFont(ScaleForDpi(hwnd_, 28), FW_BOLD);
    HFONT fontMetricLabel = makeFont(ScaleForDpi(hwnd_, 12), FW_NORMAL);
    HFONT fontMetricValue = makeFont(ScaleForDpi(hwnd_, 17), FW_BOLD);
    HFONT fontFoot = makeFont(ScaleForDpi(hwnd_, 12), FW_NORMAL);
    HGDIOBJ oldFont = SelectObject(hdc, fontKicker);

    auto cleanupFonts = [&]() {
        SelectObject(hdc, oldFont);
        DeleteObject(fontKicker);
        DeleteObject(fontTitle);
        DeleteObject(fontDelta);
        DeleteObject(fontMetricLabel);
        DeleteObject(fontMetricValue);
        DeleteObject(fontFoot);
    };

    RECT heroRect = MakeRect(clientRect.left, clientRect.top, clientRect.right, clientRect.top + heroHeight);
    FillSolidRect(hdc, heroRect, heroBg);
    FillSolidRect(hdc, MakeRect(heroRect.left, heroRect.bottom, heroRect.right, heroRect.bottom + 1), border);

    if (!snapshot_.success || !pace.valid) {
        RECT kickerRect = MakeRect(heroRect.left + padX, heroRect.top + padY + ScaleForDpi(hwnd_, 2),
            heroRect.right - padX, heroRect.top + padY + ScaleForDpi(hwnd_, 20));
        SetTextColor(hdc, textSecondary);
        DrawTextW(hdc, L"Codex Usage Budget", -1, &kickerRect, DT_LEFT | DT_VCENTER | DT_SINGLELINE);

        SelectObject(hdc, fontTitle);
        RECT titleRect = MakeRect(heroRect.left + padX, kickerRect.bottom + ScaleForDpi(hwnd_, 6),
            heroRect.right - padX, heroRect.bottom - padY);
        SetTextColor(hdc, textPrimary);
        DrawTextW(hdc, L"正在加载用量信息", -1, &titleRect, DT_LEFT | DT_TOP | DT_WORDBREAK);

        if (!snapshot_.errorMessage.empty()) {
            RECT errorRect = MakeRect(clientRect.left + padX, heroRect.bottom + sectionGap, clientRect.right - padX, clientRect.bottom - padY);
            SelectObject(hdc, fontFoot);
            SetTextColor(hdc, RGB(215, 73, 73));
            DrawTextW(hdc, snapshot_.errorMessage.c_str(), -1, &errorRect, DT_LEFT | DT_TOP | DT_WORDBREAK);
        }

        cleanupFonts();
        return;
    }

    RECT kickerRect = MakeRect(heroRect.left + padX, heroRect.top + padY + ScaleForDpi(hwnd_, 2),
        heroRect.right - padX, heroRect.top + padY + ScaleForDpi(hwnd_, 20));
    SetTextColor(hdc, textSecondary);
    DrawTextW(hdc, L"每周限额进度", -1, &kickerRect, DT_LEFT | DT_VCENTER | DT_SINGLELINE);

    const wchar_t* titleText = pace.isOver ? L"当前已超过平均进度" : L"当前低于平均进度";
    const std::wstring deltaText = (pace.deltaPercent >= 0.0 ? L"+" : L"-") + FormatPercent(std::fabs(pace.deltaPercent));

    RECT titleRect = MakeRect(heroRect.left + padX, kickerRect.bottom + ScaleForDpi(hwnd_, 4),
        heroRect.right - ScaleForDpi(hwnd_, 214), heroRect.bottom - padY + ScaleForDpi(hwnd_, 2));
    SelectObject(hdc, fontTitle);
    SetTextColor(hdc, textPrimary);
    DrawTextW(hdc, titleText, -1, &titleRect, DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS);

    RECT deltaRect = MakeRect(heroRect.right - ScaleForDpi(hwnd_, 204), heroRect.top + padY,
        heroRect.right - padX, heroRect.bottom - padY);
    SelectObject(hdc, fontDelta);
    SetTextColor(hdc, heroValue);
    DrawTextW(hdc, deltaText.c_str(), -1, &deltaRect, DT_RIGHT | DT_VCENTER | DT_SINGLELINE);

    const int metricsTop = heroRect.bottom + 1;
    RECT metricsRect = MakeRect(clientRect.left, metricsTop, clientRect.right, metricsTop + metricRowHeight);
    const wchar_t* metricLabels[4] = { L"今日平均预算", L"今日可累计使用", L"实际已使用", L"每周剩余" };
    const std::wstring metricValues[4] = {
        FormatPercent(pace.dailyBudgetPercent),
        FormatPercent(pace.expectedUsedPercent),
        FormatPercent(pace.actualUsedPercent),
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
            FillSolidRect(hdc, MakeRect(metricRect.left, metricRect.top, metricRect.left + 1, metricRect.bottom), border);
        }

        RECT labelRect = MakeRect(metricRect.left + padX, metricRect.top + ScaleForDpi(hwnd_, 8),
            metricRect.right - padX, metricRect.top + ScaleForDpi(hwnd_, 22));
        SelectObject(hdc, fontMetricLabel);
        SetTextColor(hdc, textSecondary);
        DrawTextW(hdc, metricLabels[i], -1, &labelRect, DT_LEFT | DT_TOP | DT_SINGLELINE | DT_END_ELLIPSIS);

        RECT valueRect = MakeRect(metricRect.left + padX, metricRect.top + ScaleForDpi(hwnd_, 22),
            metricRect.right - padX, metricRect.bottom - ScaleForDpi(hwnd_, 6));
        SelectObject(hdc, fontMetricValue);
        SetTextColor(hdc, textPrimary);
        DrawTextW(hdc, metricValues[i].c_str(), -1, &valueRect, DT_LEFT | DT_VCENTER | DT_SINGLELINE);
    }

    const int meterLeft = clientRect.left + padX;
    const int meterRight = clientRect.right - padX;
    const int meterHeadTop = metricsRect.bottom + sectionGap;
    const std::wstring meterHint = L"绿色条是实际已用，黑线是按周期第 " + std::to_wstring(pace.cycleDay) + L" 天应到预算";
    const std::wstring meterStat = FormatPercent(pace.actualUsedPercent) + L" / " + FormatPercent(pace.expectedUsedPercent);
    SelectObject(hdc, fontMetricLabel);
    SetTextColor(hdc, textSecondary);

    RECT meterHeadLeft = MakeRect(meterLeft, meterHeadTop, meterRight - ScaleForDpi(hwnd_, 186), meterHeadTop + ScaleForDpi(hwnd_, 24));
    RECT meterHeadRight = MakeRect(meterRight - ScaleForDpi(hwnd_, 176), meterHeadTop, meterRight, meterHeadTop + ScaleForDpi(hwnd_, 18));
    DrawTextW(hdc, meterHint.c_str(), -1, &meterHeadLeft, DT_LEFT | DT_TOP | DT_SINGLELINE | DT_END_ELLIPSIS);
    SetTextColor(hdc, textPrimary);
    DrawTextW(hdc, meterStat.c_str(), -1, &meterHeadRight, DT_RIGHT | DT_TOP | DT_SINGLELINE);
    const int meterInfoBottom = meterHeadLeft.bottom;

    RECT meterRect = MakeRect(meterLeft, meterInfoBottom + ScaleForDpi(hwnd_, 6),
        meterRight, meterInfoBottom + ScaleForDpi(hwnd_, 6) + meterHeight);
    FillSolidRect(hdc, meterRect, trackColor);

    RECT actualRect = meterRect;
    actualRect.right = actualRect.left + static_cast<int>(RectWidth(meterRect) * ClampDouble(pace.actualUsedPercent, 0.0, 100.0) / 100.0);
    if (actualRect.right > actualRect.left) {
        FillSolidRect(hdc, actualRect, actualBarColor);
    }

    const int markerX = meterRect.left + static_cast<int>(RectWidth(meterRect) * ClampDouble(pace.expectedUsedPercent, 0.0, 100.0) / 100.0);
    FillSolidRect(hdc,
        MakeRect(markerX - 1, meterRect.top - ScaleForDpi(hwnd_, 3), markerX + 1, meterRect.bottom + ScaleForDpi(hwnd_, 6)),
        lightTheme_ ? RGB(21, 27, 24) : RGB(240, 244, 241));

    RECT footerLine = MakeRect(clientRect.left, meterRect.bottom + footerTop, clientRect.right, meterRect.bottom + footerTop + 1);
    FillSolidRect(hdc, footerLine, border);

    SelectObject(hdc, fontFoot);
    SetTextColor(hdc, textSecondary);
    const std::wstring footerItems[6] = {
        L"本周开始: " + FormatDateTime(pace.weekStartUnixSeconds),
        L"重置时间: " + FormatDateTime(snapshot_.weekly.resetAtUnixSeconds),
        L"当前: 第 " + std::to_wstring(pace.cycleDay) + L" 天",
        L"已用时间: " + FormatDuration(pace.elapsedSeconds),
        L"剩余时间: " + FormatDuration(pace.remainingSeconds),
        L"5 小时限额: " + FormatPercent(snapshot_.fiveHour.usedPercent) + L" 已用，" + FormatPercent(snapshot_.fiveHour.remainingPercent) + L" 剩余",
    };

    int footX = clientRect.left + padX;
    int footY = footerLine.bottom + ScaleForDpi(hwnd_, 10);
    for (const std::wstring& item : footerItems) {
        SIZE size = {};
        GetTextExtentPoint32W(hdc, item.c_str(), static_cast<int>(item.size()), &size);
        if (footX + size.cx > clientRect.right - padX) {
            footX = clientRect.left + padX;
            footY += ScaleForDpi(hwnd_, 18);
        }
        RECT itemRect = MakeRect(footX, footY, footX + size.cx + ScaleForDpi(hwnd_, 4), footY + ScaleForDpi(hwnd_, 16));
        DrawTextW(hdc, item.c_str(), -1, &itemRect, DT_LEFT | DT_TOP | DT_SINGLELINE);
        footX += size.cx + footerGap;
    }

    RECT gripRect = MakeRect(clientRect.right - ScaleForDpi(hwnd_, 30), clientRect.bottom - ScaleForDpi(hwnd_, 18),
        clientRect.right - ScaleForDpi(hwnd_, 8), clientRect.bottom - ScaleForDpi(hwnd_, 6));
    SetTextColor(hdc, lightTheme_ ? RGB(147, 156, 149) : RGB(117, 126, 120));
    DrawTextW(hdc, L"///", -1, &gripRect, DT_RIGHT | DT_BOTTOM | DT_SINGLELINE);

    cleanupFonts();
}

void AppBarWindow::ShowContextMenu(POINT screenPoint) {
    HMENU menu = CreatePopupMenu();
    const bool launchAtStartup = IsLaunchAtStartupEnabled();
    AppendMenuW(menu, MF_STRING, 1, L"Refresh now");
    AppendMenuW(menu, MF_STRING | (launchAtStartup ? MF_CHECKED : MF_UNCHECKED), 4, L"Launch at startup");
    AppendMenuW(menu, MF_STRING, 3, L"Reset widget position");
    AppendMenuW(menu, MF_SEPARATOR, 0, nullptr);
    AppendMenuW(menu, MF_STRING, 2, L"Exit");

    const UINT command = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON, screenPoint.x, screenPoint.y, 0, hwnd_, nullptr);
    DestroyMenu(menu);

    if (command == 1) {
        RequestRefresh(true);
    } else if (command == 4) {
        SetLaunchAtStartupEnabled(!launchAtStartup);
    } else if (command == 3) {
        hasSavedRect_ = false;
        UpdateWindowBounds(false);
        SaveSettings();
    } else if (command == 2) {
        DestroyWindow(hwnd_);
    }
}

std::wstring AppBarWindow::FormatDuration(int totalSeconds) const {
    const int days = totalSeconds / 86400;
    const int hours = (totalSeconds % 86400) / 3600;

    if (days > 0) {
        return std::to_wstring(days) + L" 天 " + std::to_wstring(hours) + L" 小时";
    }
    const int minutes = (totalSeconds % 3600) / 60;
    return std::to_wstring(hours) + L" 小时 " + std::to_wstring(minutes) + L" 分钟";
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

std::wstring AppBarWindow::FormatPercent(double value) const {
    return FormatNumber(value) + L"%";
}
