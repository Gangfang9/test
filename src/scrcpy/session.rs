//! One owner for a projection, including preparation and shutdown.
use once_cell::sync::Lazy;
use serde::Serialize;
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::{Mutex as AsyncMutex, Notify};
use tokio_util::sync::CancellationToken;

pub static OPERATIONS: AsyncMutex<()> = AsyncMutex::const_new(());
static REGISTRY: Lazy<Mutex<Registry>> = Lazy::new(|| Mutex::new(Registry::default()));
static INTENT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static CLEANUP: Lazy<Mutex<Vec<(String, String)>>> = Lazy::new(|| Mutex::new(Vec::new()));
pub fn intent() -> u64 {
    INTENT.load(std::sync::atomic::Ordering::Acquire)
}
pub fn new_intent() {
    INTENT.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
}
pub fn remember_cleanup(device: &str, scid: &str) {
    let mut cleanup = CLEANUP.lock().unwrap();
    if !cleanup.iter().any(|(d, s)| d == device && s == scid) {
        cleanup.push((device.into(), scid.into()));
    }
}
pub async fn cleanup_pending(device: &str) -> Result<(), String> {
    let entries = CLEANUP
        .lock()
        .unwrap()
        .iter()
        .filter(|(d, _)| d == device)
        .cloned()
        .collect::<Vec<_>>();
    for (_, scid) in entries {
        super::managed_adb::cleanup(device, &scid).await?;
        CLEANUP
            .lock()
            .unwrap()
            .retain(|(d, s)| d != device || s != &scid);
    }
    Ok(())
}

#[derive(Clone, Debug, Serialize)]
pub struct ProjectionStatus {
    pub phase: String,
    pub scid: Option<String>,
    pub message: String,
}
impl Default for ProjectionStatus {
    fn default() -> Self {
        Self {
            phase: "idle".into(),
            scid: None,
            message: String::new(),
        }
    }
}
#[derive(Default)]
struct Registry {
    active: Option<Arc<ProjectionSession>>,
    status: ProjectionStatus,
}

#[derive(Debug)]
pub struct ProjectionSession {
    pub scid: String,
    pub device_id: String,
    pub token: CancellationToken,
    pub retries: u8,
    pub manual_stop: std::sync::atomic::AtomicBool,
    finished: CancellationToken,
    pub ready: Notify,
    is_ready: std::sync::atomic::AtomicBool,
}
impl ProjectionSession {
    pub fn reserve(device_id: String, scid: String, retries: u8) -> Result<Arc<Self>, String> {
        let mut registry = REGISTRY.lock().unwrap();
        if registry.active.is_some() {
            return Err("正在投屏或清理上一场投屏，请稍后重试".into());
        }
        let session = Self::new(device_id, scid.clone(), retries);
        registry.status = ProjectionStatus {
            phase: "starting".into(),
            scid: Some(scid),
            message: String::new(),
        };
        registry.active = Some(session.clone());
        Ok(session)
    }
    fn new(device_id: String, scid: String, retries: u8) -> Arc<Self> {
        Arc::new(Self {
            scid,
            device_id,
            retries,
            token: CancellationToken::new(),
            finished: CancellationToken::new(),
            manual_stop: std::sync::atomic::AtomicBool::new(false),
            ready: Notify::new(),
            is_ready: std::sync::atomic::AtomicBool::new(false),
        })
    }
    #[cfg(test)]
    pub fn isolated() -> Arc<Self> {
        Self::new("fixture".into(), "10000000".into(), 0)
    }
    pub fn current() -> Option<Arc<Self>> {
        REGISTRY.lock().unwrap().active.clone()
    }
    pub fn status() -> ProjectionStatus {
        REGISTRY.lock().unwrap().status.clone()
    }
    pub fn set_status(&self, phase: &str, message: impl Into<String>) {
        let mut registry = REGISTRY.lock().unwrap();
        if registry
            .active
            .as_ref()
            .is_some_and(|s| s.scid == self.scid)
        {
            registry.status = ProjectionStatus {
                phase: phase.into(),
                scid: Some(self.scid.clone()),
                message: message.into(),
            };
        }
    }
    pub fn mark_ready(&self) {
        self.is_ready
            .store(true, std::sync::atomic::Ordering::Release);
        self.set_status("streaming", "");
        self.ready.notify_one();
    }
    pub async fn wait_ready(&self) -> Result<(), String> {
        tokio::select! {
            _ = self.ready.notified() => Ok(()),
            _ = self.finished.cancelled() => Err(Self::status().message),
            _ = tokio::time::sleep(Duration::from_secs(15)) => {
                if self.is_ready.load(std::sync::atomic::Ordering::Acquire) { Ok(()) }
                else { self.token.cancel(); Err("投屏连接或首帧等待超时，请重试或检查 USB 调试".into()) }
            }
        }
    }
    pub fn stop(&self) {
        new_intent();
        self.manual_stop
            .store(true, std::sync::atomic::Ordering::Release);
        self.set_status("stopping", "正在释放投屏连接");
        self.token.cancel();
    }
    pub async fn wait_stopped(&self) -> Result<(), String> {
        tokio::time::timeout(Duration::from_secs(12), self.finished.cancelled())
            .await
            .map_err(|_| "投屏清理仍在进行，请稍后重试".to_string())
    }
    pub fn finish(&self, message: String) {
        let mut registry = REGISTRY.lock().unwrap();
        if registry
            .active
            .as_ref()
            .is_some_and(|s| s.scid == self.scid)
        {
            registry.status = ProjectionStatus {
                phase: if message.is_empty() { "idle" } else { "failed" }.into(),
                scid: None,
                message,
            };
            registry.active = None;
        }
        self.finished.cancel();
    }
    pub async fn stop_current() -> Result<(), String> {
        new_intent();
        if let Some(session) = Self::current() {
            session.stop();
            session.wait_stopped().await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn exclusive_owner_survives_cancel_until_cleanup_and_ignores_late_finish() {
        let old = ProjectionSession::reserve("test-device".into(), "old".into(), 0).unwrap();
        old.stop();
        assert!(ProjectionSession::reserve("test-device".into(), "blocked".into(), 0).is_err());
        old.finish(String::new());
        old.wait_stopped().await.unwrap();
        let new = ProjectionSession::reserve("test-device".into(), "new".into(), 0).unwrap();
        old.finish("late failure".into());
        assert_eq!(ProjectionSession::current().unwrap().scid, "new");
        assert_eq!(ProjectionSession::status().phase, "starting");
        new.mark_ready();
        new.wait_ready().await.unwrap();
        new.stop();
        new.finish(String::new());
    }
}
