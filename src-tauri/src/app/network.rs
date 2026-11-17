#![cfg(target_os = "macos")]
use crate::app::{account_hub::AccountHub, refresh::refresh_and_emit};
use std::{
    sync::{atomic::Ordering, Arc},
    time::Duration,
};
use tauri::{AppHandle, Manager};

pub(crate) fn watch(app: AppHandle) {
    let mut rx = crate::infra::network::watch_changes();
    tauri::async_runtime::spawn(async move {
        while rx.changed().await.is_ok() {
            loop {
                match tokio::time::timeout(Duration::from_millis(350), rx.changed()).await {
                    Ok(Ok(())) => continue,
                    Ok(Err(_)) => return,
                    Err(_) => break,
                }
            }
            let hub = app.state::<Arc<AccountHub>>();
            for state in hub.all() {
                if state
                    .account
                    .as_ref()
                    .is_some_and(|a| a.config().is_some_and(|c| c.enabled))
                {
                    state.network_revision.fetch_add(1, Ordering::AcqRel);
                    let handle = app.clone();
                    tauri::async_runtime::spawn(async move {
                        refresh_and_emit(&handle, &state).await;
                    });
                }
            }
        }
    });
}
