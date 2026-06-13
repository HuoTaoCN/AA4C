//! 进度上报：事件 ≥100ms 节流，写库 ≥1s 节流（V0.1 计划 M5 任务 6/8）。

use std::time::{Duration, Instant};

use aa4c_store::Store;
use aa4c_types::{CoreEvent, TaskId};

use crate::EventSender;

const EVENT_INTERVAL: Duration = Duration::from_millis(100);
const DB_INTERVAL: Duration = Duration::from_secs(1);

pub(crate) struct Progress {
    task_id: TaskId,
    events: EventSender,
    store: Store,
    total: u64,
    transferred: u64,
    last_event: Instant,
    last_db: Instant,
    /// 上次事件时的累计字节，用于速度计算
    bytes_at_last_event: u64,
}

impl Progress {
    pub fn new(task_id: TaskId, events: EventSender, store: Store, total: u64) -> Self {
        let now = Instant::now();
        Self {
            task_id,
            events,
            store,
            total,
            transferred: 0,
            last_event: now,
            last_db: now,
            bytes_at_last_event: 0,
        }
    }

    /// 重传时回退已计入的字节。
    pub fn rollback(&mut self, bytes: u64) {
        self.transferred = self.transferred.saturating_sub(bytes);
        self.bytes_at_last_event = self.bytes_at_last_event.min(self.transferred);
    }

    /// 累加进度并按节流规则上报。
    pub async fn add(&mut self, bytes: u64, current_file: &str) {
        self.transferred += bytes;
        let now = Instant::now();
        if now.duration_since(self.last_event) >= EVENT_INTERVAL {
            let dt = now.duration_since(self.last_event).as_secs_f64();
            let speed = ((self.transferred - self.bytes_at_last_event) as f64 / dt) as u64;
            let _ = self.events.send(CoreEvent::TransferProgress {
                task_id: self.task_id.clone(),
                transferred_bytes: self.transferred,
                total_bytes: self.total,
                speed_bps: speed,
                current_file: current_file.to_string(),
            });
            self.last_event = now;
            self.bytes_at_last_event = self.transferred;
        }
        if now.duration_since(self.last_db) >= DB_INTERVAL {
            let _ = self
                .store
                .update_task_progress(&self.task_id, self.transferred)
                .await;
            self.last_db = now;
        }
    }

    /// 任务结束前的最终落库。
    pub async fn finalize(&self) {
        let _ = self
            .store
            .update_task_progress(&self.task_id, self.transferred)
            .await;
    }
}
