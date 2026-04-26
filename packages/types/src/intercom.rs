use std::{
    sync::{
        Arc, Weak,
        atomic::{AtomicU64, Ordering},
    },
    time::SystemTime,
};

use crate::create_id;
use dashmap::DashMap;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, to_value};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct InterComEvent {
    pub event_id: String,
    pub event_type: String,
    pub payload: Value,
    pub timestamp: SystemTime,
}

pub type InterComCallback = Option<
    Arc<
        dyn Fn(InterComEvent) -> futures::future::BoxFuture<'static, anyhow::Result<()>>
            + Send
            + Sync,
    >,
>;
pub type BatchedCallback = Arc<
    dyn Fn(Vec<InterComEvent>) -> futures::future::BoxFuture<'static, anyhow::Result<()>>
        + Send
        + Sync,
>;

/// A buffered handler for inter-component communication events.
/// Collects events into batches by type and periodically sends them to the provided callback.
///
/// # Features
/// - Batches events by type for efficient processing
/// - Configurable flush interval and capacity limits
/// - Automatic background flushing
/// - Thread-safe for concurrent use
#[derive(Clone)]
pub struct BufferedInterComHandler {
    callback: BatchedCallback,
    interval_ms: u64,
    capacity: u64,
    buffer: Arc<DashMap<String, Vec<InterComEvent>>>,
    last_tick_ms: Arc<AtomicU64>,
}

impl BufferedInterComHandler {
    /// Creates a new buffered handler with the specified configuration.
    ///
    /// # Arguments
    /// * `callback` - The function to call when flushing event batches
    /// * `interval_ms` - Optional interval in milliseconds between automatic flushes (default: 20ms)
    /// * `capacity` - Optional maximum number of total events before forcing a flush (default: 200)
    /// * `background_check` - Optional flag to enable background flush checking (default: false)
    /// * `per_type_capacity` - Optional maximum number of events per type (default: 50)
    ///
    /// # Returns
    /// An Arc-wrapped instance of BufferedInterComHandler
    pub fn new(
        callback: BatchedCallback,
        interval_ms: Option<u64>,
        capacity: Option<u64>,
        background_check: Option<bool>,
    ) -> Arc<Self> {
        let background_check = background_check.unwrap_or(false);
        let interval_ms = interval_ms.unwrap_or(20);
        let capacity = capacity.unwrap_or(200);
        let last_tick_ms = Arc::new(AtomicU64::new(0));

        let handler = Self {
            buffer: Arc::new(DashMap::with_capacity(capacity as usize)),
            callback,
            interval_ms,
            capacity,
            last_tick_ms,
        };

        let handler = Arc::new(handler);
        let downgraded_handler = Arc::downgrade(&handler);
        if background_check {
            BufferedInterComHandler::spawn_idle_check_task(downgraded_handler, interval_ms);
        }
        handler
    }

    /// Converts this handler into a callback suitable for processing individual events.
    ///
    /// # Returns
    /// An InterComCallback that buffers events through this handler
    pub fn into_callback(&self) -> InterComCallback {
        let buffered_sender = self.clone();
        Some(Arc::new(move |response| {
            let buffered_handler = buffered_sender.clone();
            Box::pin({
                async move {
                    let handler = buffered_handler.clone();
                    handler.send(response).await?;
                    Ok(())
                }
            })
        }))
    }

    fn spawn_idle_check_task(handler: Weak<Self>, interval_ms: u64) {
        tokio::spawn(async move {
            let mut interval =
                tokio::time::interval(tokio::time::Duration::from_millis(interval_ms));

            loop {
                interval.tick().await;
                if let Some(handler) = handler.upgrade() {
                    let now = Self::now_as_millis();
                    let last_event = handler.last_tick_ms.load(Ordering::Relaxed);

                    if !handler.buffer.is_empty() && now - last_event >= 2 * interval_ms {
                        let _ = handler.flush().await;
                    }
                } else {
                    break;
                }
            }
        });
    }

    fn now_as_millis() -> u64 {
        let start = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        start.as_millis() as u64
    }

    /// Sends an event through the buffered handler.
    ///
    /// The event will be buffered until either:
    /// - The configured interval has passed
    /// - The buffer reaches capacity
    /// - This is the first event
    ///
    /// # Arguments
    /// * `event` - The event to send
    ///
    /// # Returns
    /// Result indicating success or failure
    pub async fn send(&self, event: InterComEvent) -> anyhow::Result<()> {
        self.buffer
            .entry(event.event_type.clone())
            .or_insert_with(|| Vec::with_capacity(10))
            .push(event);

        let last = self.last_tick_ms.load(Ordering::Relaxed);
        let now = Self::now_as_millis();

        // Flush if:
        // 1. Buffer is at capacity, OR
        // 2. This is the first event, OR
        // 3. Enough time has passed since last flush
        if self.buffer.len() >= self.capacity as usize
            || last == 0
            || (now - last >= self.interval_ms)
        {
            return self.flush().await;
        }

        Ok(())
    }

    /// Flushes all buffered events immediately.
    ///
    /// # Returns
    /// Result indicating success or failure
    pub async fn flush(&self) -> anyhow::Result<()> {
        if self.buffer.is_empty() {
            return Ok(());
        }

        let callback = self.callback.clone();

        // Move events out first to avoid holding DashMap guards across await
        let mut batches: Vec<(String, Vec<InterComEvent>)> = Vec::new();
        {
            for mut entry in self.buffer.iter_mut() {
                let key = entry.key().clone();
                let events_to_process = std::mem::take(entry.value_mut());
                if !events_to_process.is_empty() {
                    batches.push((key, events_to_process));
                }
            }
        }

        // Now process without any locks held
        for (_key, events_to_process) in batches {
            if let Err(err) = callback(events_to_process).await {
                println!("Error publishing events: {}", err);
            }
        }

        // Cleanup and update last tick
        self.buffer.retain(|_, events| !events.is_empty());
        self.last_tick_ms
            .store(Self::now_as_millis(), std::sync::atomic::Ordering::Relaxed);
        Ok(())
    }
}

impl Drop for BufferedInterComHandler {
    fn drop(&mut self) {
        std::mem::drop(self.flush());
    }
}

impl InterComEvent {
    pub fn new<T>(payload: T) -> Self
    where
        T: Serialize + DeserializeOwned,
    {
        Self {
            event_id: create_id(),
            event_type: "generic".to_string(),
            payload: to_value(payload).unwrap_or(Value::Null),
            timestamp: SystemTime::now(),
        }
    }

    pub fn with_type<T>(event_type: impl Into<String>, payload: T) -> Self
    where
        T: Serialize + DeserializeOwned,
    {
        Self {
            event_id: create_id(),
            event_type: event_type.into(),
            payload: to_value(payload).unwrap_or(Value::Null),
            timestamp: SystemTime::now(),
        }
    }

    pub async fn call(&self, callback: &InterComCallback) -> anyhow::Result<()> {
        if let Some(callback) = callback {
            callback(self.clone()).await?;
        }
        Ok(())
    }
}
