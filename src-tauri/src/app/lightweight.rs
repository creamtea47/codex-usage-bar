use crate::app::account_hub::AccountHub;
use crate::app::state::resolved_language;
use crate::app::windows::restore_main_window;
use crate::app::windows::restore_settings_window;
use crate::app::windows::settings_window_title;

use crate::app::state::AppState;
use crate::domain::settings::Language;
use crate::infra::desktop::MAIN_WINDOW_LABEL;
use serde::Serialize;
use std::{
    collections::{HashMap, HashSet},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};
use sysinfo::{Pid, ProcessRefreshKind, RefreshKind, System};
use tauri::{AppHandle, Manager, WebviewWindow, WebviewWindowBuilder};

#[derive(Default, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Measurement {
    status: &'static str,
    before: Option<u64>,
    after: Option<u64>,
    includes_webviews: bool,
}

impl Measurement {
    fn summary(&self, zh: bool) -> Option<String> {
        let message = match self.status {
            "measuring" => {
                if zh {
                    "正在测量内存…"
                } else {
                    "Measuring memory…"
                }
            }
            "interrupted" => {
                if zh {
                    "窗口已重开，测量中断"
                } else {
                    "Window reopened; measurement interrupted"
                }
            }
            "unavailable" => {
                if zh {
                    "内存数据不可用"
                } else {
                    "Memory data unavailable"
                }
            }
            "ready" => {
                let before = self.before? as f64 / 1e6;
                let after = self.after? as f64 / 1e6;
                let scope = match (zh, self.includes_webviews) {
                    (true, true) => "含界面进程",
                    (true, false) => "仅主进程",
                    (false, true) => "includes WebViews",
                    (false, false) => "main process only",
                };
                return Some(format!(
                    "{before:.1} MB → {after:.1} MB ({:+.1} MB · {scope})",
                    after - before
                ));
            }
            _ => return None,
        };
        Some(message.into())
    }
}

#[derive(Default)]
pub struct Runtime {
    generation: AtomicU64,
    pub(crate) transition: tokio::sync::Mutex<()>,
    timer: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
    last: Mutex<Measurement>,
}

pub(crate) fn summary(app: &AppHandle, language: Language) -> Option<String> {
    app.try_state::<Runtime>()?
        .last
        .lock()
        .ok()?
        .summary(language == Language::ZhCn)
}

pub(crate) fn refresh_tray(app: &AppHandle) {
    let lang = app.state::<Arc<AppState>>().current_settings().language;
    crate::app::tray::update_menu(app, resolved_language(lang));
}

pub(crate) fn on_open(app: &AppHandle) {
    let rt = app.state::<Runtime>();
    rt.timer.lock().unwrap_or_else(|v| v.into_inner()).take();
    rt.generation.fetch_add(1, Ordering::AcqRel);
    let mut last = rt.last.lock().unwrap_or_else(|v| v.into_inner());
    if last.status == "measuring" {
        last.status = "interrupted";
        drop(last);
        refresh_tray(app);
    }
}

pub(crate) fn ensure_window(
    app: &AppHandle,
    label: &str,
    section: Option<&str>,
) -> Result<WebviewWindow, String> {
    on_open(app);
    if let Ok(window) = crate::infra::window_activation::existing_window(app, label) {
        return Ok(window);
    }
    let mut config = app
        .config()
        .app
        .windows
        .iter()
        .find(|c| c.label == label)
        .cloned()
        .ok_or("windowConfig")?;
    if section == Some("about") {
        config.url = tauri::WebviewUrl::App("index.html#about".into());
    }
    let window = WebviewWindowBuilder::from_config(app, &config)
        .and_then(|b| b.build())
        .map_err(|_| "windowCreate")?;
    let state = app.state::<Arc<AppState>>();
    let stored = state.stored_settings().clone();
    if label == MAIN_WINDOW_LABEL {
        restore_main_window(&window, &stored, &state);
        if let Some(hub) = app.try_state::<Arc<AccountHub>>() {
            if let Ok(selected) = hub.resolve(None) {
                if let Ok(snapshot) = selected.snapshot.try_lock() {
                    selected.apply_auto_main_height(app, &snapshot);
                }
            }
        }
    } else {
        let _ = window.set_title(settings_window_title(stored.preferences.language));
        restore_settings_window(&window, &stored);
    }
    Ok(window)
}

pub(crate) async fn show(
    app: &AppHandle,
    label: &str,
    section: Option<&str>,
) -> Result<(), String> {
    let runtime = app.state::<Runtime>();
    let _guard = runtime.transition.lock().await;
    let window = ensure_window(app, label, section)?;
    #[cfg(target_os = "windows")]
    if label == MAIN_WINDOW_LABEL {
        let _ = window.set_skip_taskbar(false);
    }
    #[cfg(not(target_os = "windows"))]
    drop(window);
    crate::infra::desktop::prepare_macos_application_for_window_activation(app);
    crate::infra::window_activation::activate_window(app, label)
        .map_err(|error| error.stage().code().to_owned())
}

pub(crate) fn any_visible(app: &AppHandle) -> bool {
    app.webview_windows()
        .values()
        .any(|w| w.is_visible().unwrap_or(true))
}

pub fn schedule(app: &AppHandle) {
    let rt = app.state::<Runtime>();
    let prefs = app.state::<Arc<AppState>>().current_settings();
    if !prefs.auto_lightweight || any_visible(app) || app.webview_windows().is_empty() {
        return;
    }
    let generation = rt.generation.fetch_add(1, Ordering::AcqRel) + 1;
    let (cancel, cancelled) = tokio::sync::oneshot::channel();
    *rt.timer.lock().unwrap_or_else(|v| v.into_inner()) = Some(cancel);
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::select! {
            _ = cancelled => return,
            _ = tokio::time::sleep(Duration::from_secs(u64::from(prefs.lightweight_minutes) * 60)) => {}
        }
        let rt = app.state::<Runtime>();
        let _guard = rt.transition.lock().await;
        if rt.generation.load(Ordering::Acquire) == generation
            && !any_visible(&app)
            && app
                .state::<Arc<AppState>>()
                .current_settings()
                .auto_lightweight
        {
            if enter(&app).await.is_err() {
                log::warn!("Could not release hidden windows");
            }
        }
    });
}

pub(crate) struct Sample {
    main: u64,
    helpers: HashMap<Pid, (u64, u64)>,
}
impl Sample {
    fn total(&self) -> u64 {
        self.main + self.helpers.values().map(|(_, bytes)| *bytes).sum::<u64>()
    }
}

// WebKit XPC services may be parented by launchd. The responsibility API is
// resolved at runtime so macOS versions without it still report a scoped sample.
#[cfg(target_os = "macos")]
pub(crate) fn mac_responsibility() -> Option<unsafe extern "C" fn(i32) -> i32> {
    let ptr = unsafe {
        libc::dlsym(
            libc::RTLD_DEFAULT,
            c"responsibility_get_pid_responsible_for_pid".as_ptr(),
        )
    };
    if ptr.is_null() {
        None
    } else {
        Some(unsafe {
            std::mem::transmute::<*mut libc::c_void, unsafe extern "C" fn(i32) -> i32>(ptr)
        })
    }
}

// WKWebView can identify its own renderer even when launchd owns the process
// or the app was launched from a terminal. Older WebKit versions fall back
// to ancestry/responsibility without guessing which global renderer is ours.
#[cfg(target_os = "macos")]
pub(crate) async fn renderer_pids(windows: &HashMap<String, WebviewWindow>) -> HashSet<Pid> {
    use objc2::{msg_send, runtime::AnyObject, sel};
    let mut pids = HashSet::new();
    for window in windows.values() {
        let (tx, rx) = tokio::sync::oneshot::channel();
        if window
            .with_webview(move |native| {
                // Tauri provides a live WKWebView on its owning GUI thread.
                let webview = unsafe { &*native.inner().cast::<AnyObject>() };
                let available: bool =
                    unsafe { msg_send![webview, respondsToSelector: sel!(_webProcessIdentifier)] };
                let pid: i32 = if available {
                    unsafe { msg_send![webview, _webProcessIdentifier] }
                } else {
                    0
                };
                let _ = tx.send(pid);
            })
            .is_ok()
        {
            if let Ok(Ok(pid)) = tokio::time::timeout(Duration::from_secs(1), rx).await {
                if pid > 0 {
                    pids.insert(Pid::from_u32(pid as u32));
                }
            }
        }
    }
    pids
}

// Wry retains the WKWebView after removing it from its NSView. Close the page
// on the GUI thread as part of teardown so WebKit can release its renderer.
#[cfg(target_os = "macos")]
pub(crate) async fn close_page(window: &WebviewWindow) {
    use objc2::{msg_send, runtime::AnyObject, sel};
    let (done, closed) = tokio::sync::oneshot::channel();
    if window
        .with_webview(move |native| {
            let page = unsafe { &*native.inner().cast::<AnyObject>() };
            if unsafe { msg_send![page, respondsToSelector: sel!(_close)] } {
                let _: () = unsafe { msg_send![page, _close] };
            }
            let _ = done.send(());
        })
        .is_ok()
    {
        let _ = tokio::time::timeout(Duration::from_secs(1), closed).await;
    }
}

pub(crate) fn sample(before: Option<&Sample>, renderers: &HashSet<Pid>) -> Option<Sample> {
    let system = System::new_with_specifics(
        RefreshKind::nothing().with_processes(ProcessRefreshKind::nothing().with_memory()),
    );
    let own = Pid::from_u32(std::process::id());
    let main = system.process(own)?.memory();
    if main == 0 {
        return None;
    }
    let mut helpers = HashMap::new();
    #[cfg(target_os = "macos")]
    let responsible = mac_responsibility();
    for (pid, process) in system.processes() {
        let name = process.name().to_string_lossy().to_ascii_lowercase();
        let mut owned = renderers.contains(pid)
            || before.is_some_and(|s| {
                s.helpers
                    .get(pid)
                    .is_some_and(|(start, _)| *start == process.start_time())
            });
        if !owned && !name.contains("webkit") && name != "msedgewebview2.exe" {
            continue;
        }
        let mut parent = process.parent();
        for _ in 0..32 {
            let Some(id) = parent else { break };
            if id == own {
                owned = true;
                break;
            }
            parent = system.process(id).and_then(|p| p.parent());
        }
        #[cfg(target_os = "macos")]
        if !owned {
            owned = responsible
                .is_some_and(|f| unsafe { f(pid.as_u32() as i32) == own.as_u32() as i32 });
        }
        if owned && process.memory() > 0 {
            helpers.insert(*pid, (process.start_time(), process.memory()));
        }
    }
    Some(Sample { main, helpers })
}

// Window transitions run off the native event loop and are serialized by the caller.
pub(crate) async fn enter(app: &AppHandle) -> Result<(), String> {
    let windows = app.webview_windows();
    if windows.is_empty() {
        return Ok(());
    }
    let rt = app.state::<Runtime>();
    let generation = rt.generation.fetch_add(1, Ordering::AcqRel) + 1;
    rt.timer.lock().unwrap_or_else(|v| v.into_inner()).take();
    #[cfg(target_os = "macos")]
    let renderers = renderer_pids(&windows).await;
    #[cfg(not(target_os = "macos"))]
    let renderers = HashSet::new();
    let before = sample(None, &renderers);
    let state = app.state::<Arc<AppState>>();
    for (label, window) in windows {
        state.save_window_placement_for_label(app, &label);
        #[cfg(target_os = "macos")]
        close_page(&window).await;
        window.destroy().map_err(|_| "windowDestroy")?;
    }
    *rt.last.lock().unwrap_or_else(|v| v.into_inner()) = Measurement {
        status: "measuring",
        ..Default::default()
    };
    // destroy() queues native events. Wait until Tauri has dropped the old
    // labels before letting a concurrent tray click rebuild either WebView.
    let closed = tokio::time::timeout(Duration::from_secs(5), async {
        while !app.webview_windows().is_empty() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await;
    if closed.is_err() {
        rt.last.lock().unwrap_or_else(|v| v.into_inner()).status = "unavailable";
        refresh_tray(app);
        return Err("windowDestroy".into());
    }
    crate::infra::desktop::sync_platform_visibility(app);
    refresh_tray(app);
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(3)).await;
        let rt = app.state::<Runtime>();
        if rt.generation.load(Ordering::Acquire) != generation {
            return;
        }
        let result = match (before.as_ref(), sample(before.as_ref(), &HashSet::new())) {
            (Some(first), Some(last)) => {
                let full = !first.helpers.is_empty();
                Measurement {
                    status: "ready",
                    includes_webviews: full,
                    before: Some(first.total()),
                    after: Some(if full { last.total() } else { last.main }),
                }
            }
            _ => Measurement {
                status: "unavailable",
                ..Default::default()
            },
        };
        let mut last = rt.last.lock().unwrap_or_else(|v| v.into_inner());
        if rt.generation.load(Ordering::Acquire) != generation {
            return;
        }
        *last = result;
        drop(last);
        refresh_tray(&app);
    });
    Ok(())
}
