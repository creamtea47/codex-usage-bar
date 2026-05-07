#pragma once

#include "CodexUsageFetcher.h"

#include <Windows.h>
#include <d2d1.h>
#include <dwrite.h>
#include <wrl/client.h>
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

    HRESULT CreateDeviceIndependentResources();
    HRESULT CreateDeviceResources();
    void DiscardDeviceResources();
    void DiscardTextFormats();
    HRESULT EnsureTextFormats();
    HRESULT CreateTextFormat(float sizePixels, DWRITE_FONT_WEIGHT weight, IDWriteTextFormat** format);

    void Paint(HDC hdc);
    void PaintContent(const RECT& clientRect);
    void ShowContextMenu(POINT screenPoint);
    int GetMinimumWidgetWidth() const;
    int GetMinimumWidgetHeight(int width) const;

    std::wstring FormatDuration(int totalSeconds) const;
    std::wstring FormatDateTime(long long unixSeconds) const;
    std::wstring FormatClockTime(long long unixSeconds) const;
    std::wstring FormatPercent(double value) const;

    HINSTANCE instance_ = nullptr;
    HWND hwnd_ = nullptr;
    std::atomic_bool refreshInFlight_ = false;
    bool lightTheme_ = false;
    bool alwaysOnTop_ = false;
    bool lockPosition_ = false;
    bool simpleMode_ = false;
    bool hasSavedRect_ = false;
    RECT savedRect_ = {};
    DragMode dragMode_ = DragMode::None;
    POINT dragStartPoint_ = {};
    RECT dragStartRect_ = {};
    UINT textFormatDpi_ = 0;
    long long lastSuccessfulRefreshUnixSeconds_ = 0;
    int refreshCountdownSeconds_ = 60;

    UsageSnapshot snapshot_;
    CodexUsageFetcher fetcher_;

    Microsoft::WRL::ComPtr<ID2D1Factory> d2dFactory_;
    Microsoft::WRL::ComPtr<IDWriteFactory> dwriteFactory_;
    Microsoft::WRL::ComPtr<ID2D1DCRenderTarget> renderTarget_;
    Microsoft::WRL::ComPtr<ID2D1SolidColorBrush> solidBrush_;
    Microsoft::WRL::ComPtr<IDWriteTextFormat> textFormatKicker_;
    Microsoft::WRL::ComPtr<IDWriteTextFormat> textFormatTitle_;
    Microsoft::WRL::ComPtr<IDWriteTextFormat> textFormatDelta_;
    Microsoft::WRL::ComPtr<IDWriteTextFormat> textFormatMetricLabel_;
    Microsoft::WRL::ComPtr<IDWriteTextFormat> textFormatMetricValue_;
    Microsoft::WRL::ComPtr<IDWriteTextFormat> textFormatFoot_;
};
