//! Polling the manual workflow runs you started, and the ones you follow by link.
use super::engine::lock;
use super::*;
use crate::runs::{RunCadence, RunMsg, RunSource};

impl Server {
    /// Start polling runs through `source`, for the repos the service works out and the runs you follow.
    pub fn poll_runs(self: &Arc<Self>, source: Arc<dyn RunSource>) {
        *lock(&self.eng.runs_on) = true;
        lock(&self.eng.runs).loading = true;
        let (rx, wake) = crate::runs::spawn_with(source, self.eng.run_repos.clone(), self.eng.run_watched.clone(), RunCadence::default(), self.attention.clone());
        *lock(&self.eng.run_wake) = Some(wake);
        let me = self.clone();
        std::thread::spawn(move || {
            while let Ok(msg) = rx.recv() {
                match msg {
                    RunMsg::Started => lock(&me.eng.runs).loading = true,
                    RunMsg::Result(Ok(batch)) => {
                        let runs = me.settle_runs(batch.runs);
                        let mut data = lock(&me.eng.runs);
                        data.items = runs;
                        data.error = None;
                        data.loading = false;
                        data.updated = Some(Utc::now());
                    }
                    RunMsg::Result(Err(e)) => {
                        let mut data = lock(&me.eng.runs);
                        data.error = Some(e);
                        data.loading = false;
                    }
                }
            }
        });
    }
}
