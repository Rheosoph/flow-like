use std::{
    sync::{
        Arc, Weak,
        atomic::{AtomicU64, Ordering},
    },
    time::SystemTime,
};

use crate::create_id;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, to_value};
use tokio::sync::Mutex;

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
/// Collects events in enqueue order and periodically sends contiguous, same-type
/// batches to the provided callback.
///
/// # Features
/// - Preserves ordering across event types while keeping callback batches homogeneous
/// - Configurable flush interval and capacity limits
/// - Automatic background flushing
/// - Serializes callback delivery across concurrent flushes
#[derive(Clone)]
pub struct BufferedInterComHandler {
    callback: BatchedCallback,
    interval_ms: u64,
    capacity: u64,
    buffer: Arc<Mutex<Vec<InterComEvent>>>,
    flush_lock: Arc<Mutex<()>>,
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
            buffer: Arc::new(Mutex::new(Vec::with_capacity(capacity as usize))),
            flush_lock: Arc::new(Mutex::new(())),
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
                    let has_buffered_events = !handler.buffer.lock().await.is_empty();

                    if has_buffered_events && now.saturating_sub(last_event) >= 2 * interval_ms {
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
        let last = self.last_tick_ms.load(Ordering::Relaxed);
        let now = Self::now_as_millis();
        let buffered_events = {
            let mut buffer = self.buffer.lock().await;
            buffer.push(event);
            buffer.len()
        };

        // Flush if:
        // 1. Buffer is at capacity, OR
        // 2. This is the first event, OR
        // 3. Enough time has passed since last flush
        if buffered_events >= self.capacity as usize
            || last == 0
            || now.saturating_sub(last) >= self.interval_ms
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
        // A flush may be triggered by send(), the background checker, or a
        // caller explicitly. Serialize the complete drain-and-callback path so
        // two callbacks can never overtake each other.
        let _flush_guard = self.flush_lock.lock().await;

        // Move events out before awaiting the callback. Producers can continue
        // enqueueing while this batch is being delivered; the next serialized
        // flush will pick those events up without reordering either batch.
        let events_to_process = {
            let mut buffer = self.buffer.lock().await;
            if buffer.is_empty() {
                return Ok(());
            }
            std::mem::take(&mut *buffer)
        };

        // Existing callbacks commonly route a batch using its first event type.
        // Keep each callback batch homogeneous, but only coalesce adjacent
        // events so an A/B/A sequence can never become A/A/B.
        let mut batches: Vec<Vec<InterComEvent>> = Vec::new();
        for event in events_to_process {
            let matches_last_batch = batches
                .last()
                .and_then(|batch| batch.first())
                .is_some_and(|first| first.event_type == event.event_type);
            if matches_last_batch {
                batches.last_mut().unwrap().push(event);
            } else {
                batches.push(vec![event]);
            }
        }

        for batch in batches {
            if let Err(err) = (self.callback)(batch).await {
                println!("Error publishing events: {}", err);
            }
        }

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

#[cfg(test)]
mod tests {
    use super::{BatchedCallback, BufferedInterComHandler, InterComEvent};
    use serde_json::json;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    use tokio::sync::{Barrier, Mutex};

    fn sequenced_event(event_type: &str, sequence: u64) -> InterComEvent {
        InterComEvent::with_type(event_type, json!({ "sequence": sequence }))
    }

    #[tokio::test]
    async fn mixed_event_types_keep_global_enqueue_order() {
        let delivered = Arc::new(Mutex::new(Vec::<Vec<(String, u64)>>::new()));
        let delivered_for_callback = delivered.clone();
        let callback: BatchedCallback = Arc::new(move |events| {
            let delivered = delivered_for_callback.clone();
            Box::pin(async move {
                let batch = events
                    .into_iter()
                    .map(|event| {
                        (
                            event.event_type,
                            event.payload["sequence"].as_u64().unwrap(),
                        )
                    })
                    .collect();
                delivered.lock().await.push(batch);
                Ok(())
            })
        });
        let handler = BufferedInterComHandler::new(callback, Some(60_000), Some(100), Some(false));
        handler
            .last_tick_ms
            .store(BufferedInterComHandler::now_as_millis(), Ordering::Relaxed);

        handler
            .send(sequenced_event("chat_stream_partial", 1))
            .await
            .unwrap();
        handler
            .send(sequenced_event("chat_stream_partial", 2))
            .await
            .unwrap();
        handler
            .send(sequenced_event("chat_local_session", 3))
            .await
            .unwrap();
        handler
            .send(sequenced_event("chat_stream_partial", 4))
            .await
            .unwrap();
        handler.send(sequenced_event("chat_out", 5)).await.unwrap();
        handler.flush().await.unwrap();

        assert_eq!(
            *delivered.lock().await,
            vec![
                vec![
                    ("chat_stream_partial".to_string(), 1),
                    ("chat_stream_partial".to_string(), 2),
                ],
                vec![("chat_local_session".to_string(), 3)],
                vec![("chat_stream_partial".to_string(), 4)],
                vec![("chat_out".to_string(), 5)],
            ]
        );
    }

    #[tokio::test]
    async fn concurrent_flushes_serialize_callbacks_and_batches() {
        let entered_first_callback = Arc::new(Barrier::new(2));
        let release_first_callback = Arc::new(Barrier::new(2));
        let active_callbacks = Arc::new(AtomicUsize::new(0));
        let max_active_callbacks = Arc::new(AtomicUsize::new(0));
        let callback_count = Arc::new(AtomicUsize::new(0));
        let delivered = Arc::new(Mutex::new(Vec::<Vec<u64>>::new()));

        let callback: BatchedCallback = Arc::new({
            let entered_first_callback = entered_first_callback.clone();
            let release_first_callback = release_first_callback.clone();
            let active_callbacks = active_callbacks.clone();
            let max_active_callbacks = max_active_callbacks.clone();
            let callback_count = callback_count.clone();
            let delivered = delivered.clone();
            move |events| {
                let entered_first_callback = entered_first_callback.clone();
                let release_first_callback = release_first_callback.clone();
                let active_callbacks = active_callbacks.clone();
                let max_active_callbacks = max_active_callbacks.clone();
                let callback_count = callback_count.clone();
                let delivered = delivered.clone();
                Box::pin(async move {
                    let active = active_callbacks.fetch_add(1, Ordering::SeqCst) + 1;
                    max_active_callbacks.fetch_max(active, Ordering::SeqCst);
                    let callback_index = callback_count.fetch_add(1, Ordering::SeqCst);

                    if callback_index == 0 {
                        entered_first_callback.wait().await;
                        release_first_callback.wait().await;
                    }

                    delivered.lock().await.push(
                        events
                            .into_iter()
                            .map(|event| event.payload["sequence"].as_u64().unwrap())
                            .collect(),
                    );
                    active_callbacks.fetch_sub(1, Ordering::SeqCst);
                    Ok(())
                })
            }
        });

        let handler = BufferedInterComHandler::new(callback, Some(60_000), Some(100), Some(false));
        handler
            .last_tick_ms
            .store(BufferedInterComHandler::now_as_millis(), Ordering::Relaxed);
        handler.send(sequenced_event("first", 1)).await.unwrap();

        let first_handler = handler.clone();
        let first_flush = tokio::spawn(async move { first_handler.flush().await.unwrap() });
        entered_first_callback.wait().await;

        handler.send(sequenced_event("second", 2)).await.unwrap();
        let second_handler = handler.clone();
        let second_flush = tokio::spawn(async move { second_handler.flush().await.unwrap() });

        tokio::task::yield_now().await;
        assert_eq!(callback_count.load(Ordering::SeqCst), 1);
        assert_eq!(max_active_callbacks.load(Ordering::SeqCst), 1);

        release_first_callback.wait().await;
        first_flush.await.unwrap();
        second_flush.await.unwrap();

        assert_eq!(*delivered.lock().await, vec![vec![1], vec![2]]);
        assert_eq!(max_active_callbacks.load(Ordering::SeqCst), 1);
    }
}
