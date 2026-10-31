//! Production progress advances only after the HTTP client accepts actual payload with write_all.
//! Sample timestamps bound event lag; pausing below independently checks the flushed shard lengths.
use super::*;
use std::sync::Mutex;
use tauri::Listener;

struct Sample {
    bytes: u64,
    at: std::time::Instant,
}
pub(super) struct Received {
    app: AppHandle,
    event: tauri::EventId,
    latest: Arc<Mutex<Sample>>,
}
impl Received {
    pub(super) fn new(app: &AppHandle, task_id: &str) -> Self {
        let latest = Arc::new(Mutex::new(Sample {
            bytes: 0,
            at: std::time::Instant::now(),
        }));
        let observed = latest.clone();
        let task_id = task_id.to_owned();
        let event = app.listen("task-updated", move |event| {
            let event: crate::models::TaskProgressEvent =
                serde_json::from_str(event.payload()).unwrap();
            if event.task.id == task_id {
                let mut sample = observed.lock().unwrap();
                sample.bytes = sample.bytes.max(event.task.downloaded_bytes);
                sample.at = std::time::Instant::now();
            }
        });
        Self {
            app: app.clone(),
            event,
            latest,
        }
    }
    pub(super) fn bytes(&self) -> u64 {
        self.latest.lock().unwrap().bytes
    }
    pub(super) async fn fresh_bytes(&self) -> u64 {
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                {
                    let sample = self.latest.lock().unwrap();
                    if sample.at.elapsed() <= Duration::from_millis(500) {
                        return sample.bytes;
                    }
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("active client must publish a payload sample no older than 500ms")
    }
}
impl Drop for Received {
    fn drop(&mut self) {
        self.app.unlisten(self.event);
    }
}
