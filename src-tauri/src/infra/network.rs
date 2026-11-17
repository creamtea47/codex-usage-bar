//! Low-frequency account requests build their connector at dispatch so system proxy
//! settings and the connection pool always belong to the current network.
use reqwest::Client;
use std::sync::Mutex;
use std::time::{Duration, Instant};

#[derive(Clone)]
pub struct HttpClient {
    timeout: Duration,
    #[cfg(test)]
    fixed: Option<Client>,
}

impl HttpClient {
    pub fn new(seconds: u64) -> Self {
        Self {
            timeout: Duration::from_secs(seconds),
            #[cfg(test)]
            fixed: None,
        }
    }

    #[cfg(test)]
    pub fn fixed(client: Client) -> Self {
        Self {
            fixed: Some(client),
            ..Self::new(5)
        }
    }

    pub fn build(&self) -> reqwest::Result<Client> {
        #[cfg(test)]
        if let Some(client) = &self.fixed {
            return Ok(client.clone());
        }
        Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(self.timeout)
            .user_agent(concat!("CodexUsageBar/", env!("CARGO_PKG_VERSION")))
            .build()
    }
}

#[derive(Default)]
pub struct Connectivity(Mutex<(Option<Instant>, bool)>);

impl Connectivity {
    pub fn observe(&self, started: Instant, offline: bool) -> bool {
        let mut last = self.0.lock().unwrap_or_else(|v| v.into_inner());
        if last.0.is_some_and(|previous| started <= previous) {
            return false;
        }
        let changed = last.1 != offline;
        *last = (Some(started), offline);
        changed
    }
}

#[cfg(target_os = "macos")]
pub(crate) fn watch_changes() -> tokio::sync::watch::Receiver<u64> {
    use core_foundation::{
        array::CFArray,
        runloop::{kCFRunLoopCommonModes, CFRunLoop},
        string::CFString,
    };
    use system_configuration::dynamic_store::{
        SCDynamicStore, SCDynamicStoreBuilder, SCDynamicStoreCallBackContext,
    };
    use tokio::sync::watch;

    fn changed(_: SCDynamicStore, _: CFArray<CFString>, tx: &mut watch::Sender<u64>) {
        tx.send_modify(|v| *v = v.wrapping_add(1));
    }
    let (tx, rx) = watch::channel(0);
    std::thread::spawn(move || {
        let context = SCDynamicStoreCallBackContext {
            callout: changed,
            info: tx,
        };
        let Some(store) = SCDynamicStoreBuilder::new("CodexUsageBar.Network")
            .callback_context(context)
            .build()
        else {
            log::warn!("Network configuration watcher unavailable");
            return;
        };
        let patterns = CFArray::from_CFTypes(&[CFString::from(
            "(State|Setup):/Network/(Global|Service/[^/]+)/(Proxies|IPv4|IPv6|DNS)",
        )]);
        if !store.set_notification_keys(&CFArray::<CFString>::from_CFTypes(&[]), &patterns) {
            return;
        }
        if let Some(source) = store.create_run_loop_source() {
            CFRunLoop::get_current().add_source(&source, unsafe { kCFRunLoopCommonModes });
            CFRunLoop::run_current();
        }
    });
    rx
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn outages_are_shared_and_older_requests_cannot_reopen_them() {
        let tracker = Connectivity::default();
        let first = Instant::now();
        assert!(tracker.observe(first, true));
        assert!(!tracker.observe(first + Duration::from_secs(1), true));
        assert!(tracker.observe(first + Duration::from_secs(3), false));
        assert!(!tracker.observe(first + Duration::from_secs(2), true));
        assert!(!tracker.observe(first + Duration::from_secs(4), false));
    }
}
