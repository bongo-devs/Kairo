//! The registry of named background tasks, so a task can be replaced or cancelled by key.

use std::collections::BTreeMap;
use std::future::Future;
use std::sync::{LazyLock, Mutex};
use std::time::Duration;

use tokio::task::JoinHandle;

pub static TASKS: LazyLock<Tasks> = LazyLock::new(Tasks::default);

pub fn session_stats(session_id: &str) -> String {
    format!("stats:{session_id}")
}

#[derive(Default)]
pub struct Tasks {
    runners: Mutex<BTreeMap<String, JoinHandle<()>>>,
}

impl Tasks {
    /// Run `handler` every `period`, aborting whatever was registered under `key` before.
    pub fn add<F, Fut>(&self, key: impl Into<String>, period: Duration, mut handler: F)
    where
        F: FnMut() -> Fut + Send + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let key = key.into();
        let runner = tokio::spawn(async move {
            let mut interval = tokio::time::interval(period);
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                interval.tick().await;
                handler().await;
            }
        });
        tracing::debug!(task = %key, secs = period.as_secs(), "started interval");
        if let Some(previous) = self.runners.lock().unwrap().insert(key, runner) {
            previous.abort();
        }
    }

    pub fn remove(&self, key: &str) {
        if let Some(runner) = self.runners.lock().unwrap().remove(key) {
            runner.abort();
            tracing::debug!(task = %key, "stopped interval");
        }
    }
}
