use crate::infra::desktop::MAIN_WINDOW_LABEL;
use crate::infra::desktop::SETTINGS_WINDOW_LABEL;
use crate::ipc::access::has_exact_window_label;
use crate::ipc::access::is_known_window_label;

#[test]
fn command_window_scopes_do_not_overlap() {
    assert!(has_exact_window_label(MAIN_WINDOW_LABEL, MAIN_WINDOW_LABEL));
    assert!(!has_exact_window_label(
        SETTINGS_WINDOW_LABEL,
        MAIN_WINDOW_LABEL
    ));
    assert!(is_known_window_label(MAIN_WINDOW_LABEL));
    assert!(is_known_window_label(SETTINGS_WINDOW_LABEL));
    assert!(!is_known_window_label("untrusted-window"));
}
