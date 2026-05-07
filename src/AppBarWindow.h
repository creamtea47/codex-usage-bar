#pragma once

#include "CodexUsageFetcher.h"

#include <Windows.h>
#include <atomic>
#include <filesystem>
#include <string>

class AppBarWindow {
public:
    explicit AppBarWindow(HINSTANCE instance);
    ~AppBarWindow();

    bool Create();
    int Run();

private:
    static constexpr UINT kUsageUpdatedMessage = WM_APP + 1;
    static constexpr UINT_PTR kCountdownTimerId = 1;
    static constexpr UINT_PTR kRefreshTimerId = 2;

    enum class DragMode {
        None,
        Move,
        ResizeRight,
        ResizeBottom,
        ResizeCorner,
    };

    static LRESULT CALLBACK WindowProc(HWND hwnd, UINT message, WPARAM wParam, LPARAM lParam);
    LRESULT HandleMessage(UINT message, WPARAM wParam, LPARAM lParam);

    void RegisterWindowClass();
    RECT GetDesktopClientRect() const;
    RECT BuildDefaultRect(const RECT& desktopRect) const;
    RECT ClampRectToDesktop(RECT rect) const;
    void UpdateWindowBounds(bool useSavedPosition);

    void LoadSettings();
    void SaveSettings() const;
    std::wstring GetSettingsPath() const;
    std::wstring GetExecutablePath() const;
    void RefreshTheme();
    bool IsDesktopLightTheme() const;
    bool IsLaunchAtStartupEnabled() const;
    bool SetLaunchAtStartupEnabled(bool enabled) const;

    DragMode HitTestDragMode(POINT clientPoint) const;
    void BeginDrag(DragMode mode, POINT screenPoint);
    void UpdateDrag(POINT screenPoint);
    void EndDrag(bool saveSettings);

    void RequestRefresh(bool force);
    void OnUsageUpdated(UsageSnapshot* snapshot);

    void Paint(HDC hdc);
    void PaintContent(HDC hdc, const RECT& clientRect);
    void ShowContextMenu(POINT screenPoint);

    std::wstring FormatDuration(int totalSeconds) const;
    std::wstring FormatDateTime(long long unixSeconds) const;
    std::wstring FormatPercent(double value) const;

    HINSTANCE instance_ = nullptr;
    HWND hwnd_ = nullptr;
    std::atomic_bool refreshInFlight_ = false;
    bool lightTheme_ = false;
    bool hasSavedRect_ = false;
    RECT savedRect_ = {};
    DragMode dragMode_ = DragMode::None;
    POINT dragStartPoint_ = {};
    RECT dragStartRect_ = {};

    UsageSnapshot snapshot_;
    CodexUsageFetcher fetcher_;
};
