use tauri::{AppHandle, Manager, Runtime, WebviewWindow};

/// 激活既有窗口失败时的稳定阶段。
///
/// 这里只记录阶段，不保留平台原始错误，避免把系统细节传播到 IPC 或日志。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WindowActivationStage {
    Locate,
    Restore,
    Show,
    Focus,
}

impl WindowActivationStage {
    pub(crate) const fn code(self) -> &'static str {
        match self {
            Self::Locate => "locate",
            Self::Restore => "restore",
            Self::Show => "show",
            Self::Focus => "focus",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct WindowActivationError {
    stage: WindowActivationStage,
}

impl WindowActivationError {
    const fn at(stage: WindowActivationStage) -> Self {
        Self { stage }
    }

    pub(crate) const fn stage(self) -> WindowActivationStage {
        self.stage
    }
}

impl std::fmt::Display for WindowActivationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "window activation failed at {}",
            self.stage.code()
        )
    }
}

impl std::error::Error for WindowActivationError {}

trait ExistingWindowLocator {
    type Window: ActivatableWindow;

    fn existing_window(&self, label: &str) -> Option<Self::Window>;
}

trait ActivatableWindow {
    fn unminimize_for_activation(&self) -> Result<(), ()>;
    fn show_for_activation(&self) -> Result<(), ()>;
    fn focus_for_activation(&self) -> Result<(), ()>;
}

impl<R: Runtime> ExistingWindowLocator for AppHandle<R> {
    type Window = WebviewWindow<R>;

    fn existing_window(&self, label: &str) -> Option<Self::Window> {
        self.get_webview_window(label)
    }
}

impl<R: Runtime> ActivatableWindow for WebviewWindow<R> {
    fn unminimize_for_activation(&self) -> Result<(), ()> {
        self.unminimize().map_err(|_| ())
    }

    fn show_for_activation(&self) -> Result<(), ()> {
        self.show().map_err(|_| ())
    }

    fn focus_for_activation(&self) -> Result<(), ()> {
        self.set_focus().map_err(|_| ())
    }
}

/// 定位并激活一个已经由 Tauri 配置创建的窗口。
///
/// 每次调用都会完整执行恢复、显示、聚焦，不根据上一次调用缓存窗口状态，也不会创建窗口。
pub(crate) fn activate_window<R: Runtime>(
    app: &AppHandle<R>,
    label: &str,
) -> Result<(), WindowActivationError> {
    activate_window_with(app, label)
}

/// 定位一个已经由 Tauri 配置创建的窗口，不创建新的窗口或 WebView。
pub(crate) fn existing_window<R: Runtime>(
    app: &AppHandle<R>,
    label: &str,
) -> Result<WebviewWindow<R>, WindowActivationError> {
    existing_window_with(app, label)
}

/// 激活一个已经定位的窗口，供调用方在窗口级激活前插入平台专属准备步骤。
pub(crate) fn activate_existing_window<R: Runtime>(
    window: &WebviewWindow<R>,
) -> Result<(), WindowActivationError> {
    activate_existing_window_with(window)
}

fn activate_window_with<L: ExistingWindowLocator>(
    locator: &L,
    label: &str,
) -> Result<(), WindowActivationError> {
    let window = existing_window_with(locator, label)?;
    activate_existing_window_with(&window)
}

fn existing_window_with<L: ExistingWindowLocator>(
    locator: &L,
    label: &str,
) -> Result<L::Window, WindowActivationError> {
    locator
        .existing_window(label)
        .ok_or(WindowActivationError::at(WindowActivationStage::Locate))
}

fn activate_existing_window_with<W: ActivatableWindow>(
    window: &W,
) -> Result<(), WindowActivationError> {
    window
        .unminimize_for_activation()
        .map_err(|()| WindowActivationError::at(WindowActivationStage::Restore))?;
    window
        .show_for_activation()
        .map_err(|()| WindowActivationError::at(WindowActivationStage::Show))?;
    window
        .focus_for_activation()
        .map_err(|()| WindowActivationError::at(WindowActivationStage::Focus))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::RefCell, rc::Rc};

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Action {
        Locate,
        Unminimize,
        Show,
        Focus,
    }

    #[derive(Debug, Default)]
    struct FakeState {
        actions: Vec<Action>,
        fail_at: Option<WindowActivationStage>,
    }

    #[derive(Clone)]
    struct FakeWindow {
        state: Rc<RefCell<FakeState>>,
    }

    impl ActivatableWindow for FakeWindow {
        fn unminimize_for_activation(&self) -> Result<(), ()> {
            let mut state = self.state.borrow_mut();
            state.actions.push(Action::Unminimize);
            (state.fail_at != Some(WindowActivationStage::Restore))
                .then_some(())
                .ok_or(())
        }

        fn show_for_activation(&self) -> Result<(), ()> {
            let mut state = self.state.borrow_mut();
            state.actions.push(Action::Show);
            (state.fail_at != Some(WindowActivationStage::Show))
                .then_some(())
                .ok_or(())
        }

        fn focus_for_activation(&self) -> Result<(), ()> {
            let mut state = self.state.borrow_mut();
            state.actions.push(Action::Focus);
            (state.fail_at != Some(WindowActivationStage::Focus))
                .then_some(())
                .ok_or(())
        }
    }

    struct FakeLocator {
        state: Rc<RefCell<FakeState>>,
        has_window: bool,
    }

    impl FakeLocator {
        fn new(fail_at: Option<WindowActivationStage>) -> Self {
            Self {
                state: Rc::new(RefCell::new(FakeState {
                    fail_at,
                    ..FakeState::default()
                })),
                has_window: fail_at != Some(WindowActivationStage::Locate),
            }
        }

        fn actions(&self) -> Vec<Action> {
            self.state.borrow().actions.clone()
        }
    }

    impl ExistingWindowLocator for FakeLocator {
        type Window = FakeWindow;

        fn existing_window(&self, _label: &str) -> Option<Self::Window> {
            self.state.borrow_mut().actions.push(Action::Locate);
            self.has_window.then(|| FakeWindow {
                state: Rc::clone(&self.state),
            })
        }
    }

    #[test]
    fn every_request_runs_the_complete_activation_sequence() {
        let locator = FakeLocator::new(None);

        activate_window_with(&locator, "settings").unwrap();
        activate_window_with(&locator, "settings").unwrap();

        assert_eq!(
            locator.actions(),
            vec![
                Action::Locate,
                Action::Unminimize,
                Action::Show,
                Action::Focus,
                Action::Locate,
                Action::Unminimize,
                Action::Show,
                Action::Focus,
            ]
        );
    }

    #[test]
    fn locate_failure_stops_before_window_actions() {
        let locator = FakeLocator::new(Some(WindowActivationStage::Locate));

        let error = activate_window_with(&locator, "settings").unwrap_err();

        assert_eq!(error.stage(), WindowActivationStage::Locate);
        assert_eq!(error.stage().code(), "locate");
        assert_eq!(locator.actions(), vec![Action::Locate]);
    }

    #[test]
    fn unminimize_failure_stops_before_show_and_focus() {
        let locator = FakeLocator::new(Some(WindowActivationStage::Restore));

        let error = activate_window_with(&locator, "settings").unwrap_err();

        assert_eq!(error.stage(), WindowActivationStage::Restore);
        assert_eq!(error.stage().code(), "restore");
        assert_eq!(locator.actions(), vec![Action::Locate, Action::Unminimize]);
    }

    #[test]
    fn show_failure_stops_before_focus() {
        let locator = FakeLocator::new(Some(WindowActivationStage::Show));

        let error = activate_window_with(&locator, "settings").unwrap_err();

        assert_eq!(error.stage(), WindowActivationStage::Show);
        assert_eq!(error.stage().code(), "show");
        assert_eq!(
            locator.actions(),
            vec![Action::Locate, Action::Unminimize, Action::Show]
        );
    }

    #[test]
    fn focus_failure_reports_after_restore_and_show() {
        let locator = FakeLocator::new(Some(WindowActivationStage::Focus));

        let error = activate_window_with(&locator, "settings").unwrap_err();

        assert_eq!(error.stage(), WindowActivationStage::Focus);
        assert_eq!(error.stage().code(), "focus");
        assert_eq!(
            locator.actions(),
            vec![
                Action::Locate,
                Action::Unminimize,
                Action::Show,
                Action::Focus,
            ]
        );
    }
}
