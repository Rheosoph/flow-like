//! Gateway statistics: request rows kept 48 h, minute rollups 7 days and hourly rollups
//! 90 days, keyed by model and consumer. Counts and timings only, never text.

use super::db::{ModelsDb, RequestRecord, RollupRecord};
use crate::enrollment::unix_time;
use anyhow::{Context, Result};
use flow_like_device_protocol::{
    MODEL_STATS_MAX_CONSUMERS, ModelConsumer, ModelConsumerTotals, ModelStats, ModelStatsSeries,
    StatsStep,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap},
    path::Path,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

pub const REQUEST_RETENTION_SECONDS: i64 = 48 * 3_600;
pub const MINUTE_RETENTION_SECONDS: i64 = 7 * 86_400;
pub const HOUR_RETENTION_SECONDS: i64 = 90 * 86_400;
/// Requests are written when they end, at most one flush late; a bucket closes after this.
const ROLLUP_GRACE_SECONDS: i64 = 15;
const FLUSH_INTERVAL: Duration = Duration::from_secs(2);
const ROLLUP_INTERVAL: Duration = Duration::from_secs(60);
const FLUSH_ROWS: usize = 256;
const QUEUED_ROWS: usize = 16_384;
const STEPS: [StatsStep; 2] = [StatsStep::Minute, StatsStep::Hour];
const HISTOGRAM_BASE: f64 = 1.2;
const HISTOGRAM_LAST: u8 = 95;

/// Log-scale latency buckets in milliseconds: bucket `i > 0` holds `[1.2^(i-1), 1.2^i)`,
/// so a quantile is off by at most 10 %. Histograms of buckets and consumers merge exactly.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct LatencyHistogram(BTreeMap<u8, u64>);

impl LatencyHistogram {
    fn bucket(milliseconds: u32) -> u8 {
        if milliseconds == 0 {
            return 0;
        }
        let index = (f64::from(milliseconds).ln() / HISTOGRAM_BASE.ln()).floor() + 1.0;
        (index as u64).min(u64::from(HISTOGRAM_LAST)) as u8
    }

    pub fn record(&mut self, milliseconds: u32) {
        *self.0.entry(Self::bucket(milliseconds)).or_default() += 1;
    }

    pub fn merge(&mut self, other: &Self) {
        for (bucket, count) in &other.0 {
            *self.0.entry(*bucket).or_default() += count;
        }
    }

    pub fn count(&self) -> u64 {
        self.0.values().sum()
    }

    /// The geometric middle of the bucket holding the `quantile` rank.
    pub fn quantile(&self, quantile: f64) -> Option<u32> {
        let total = self.count();
        if total == 0 {
            return None;
        }
        let rank = ((quantile * total as f64).ceil() as u64).clamp(1, total);
        let mut seen = 0;
        let bucket = self.0.iter().find_map(|(bucket, count)| {
            seen += count;
            (seen >= rank).then_some(*bucket)
        })?;
        Some(match bucket {
            0 => 0,
            bucket => HISTOGRAM_BASE
                .powf(f64::from(bucket) - 0.5)
                .round()
                .min(f64::from(u32::MAX)) as u32,
        })
    }
}

/// Counts an error once the gateway answered with a failure; a client that went away is
/// not one.
pub fn is_error(status: u16) -> bool {
    status >= 400 && status != CLIENT_CLOSED
}

/// Status of a request whose client disconnected before the answer ended.
pub const CLIENT_CLOSED: u16 = 499;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Bucket {
    requests: u64,
    errors: u64,
    prompt_tokens: u64,
    completion_tokens: u64,
    cached_tokens: u64,
    decode_ms: u64,
    ttft: LatencyHistogram,
    queue: LatencyHistogram,
}

impl Bucket {
    fn add_request(&mut self, request: &RequestRecord) {
        self.requests += 1;
        self.errors += u64::from(is_error(request.status));
        self.prompt_tokens += request.prompt_tokens;
        self.completion_tokens += request.completion_tokens;
        self.cached_tokens += request.cached_tokens;
        self.decode_ms += u64::from(request.decode_ms);
        if let Some(ttft) = request.ttft_ms {
            self.ttft.record(ttft);
        }
        self.queue.record(request.queue_ms);
    }

    fn add_rollup(&mut self, rollup: &RollupRecord) {
        self.requests += rollup.requests;
        self.errors += rollup.errors;
        self.prompt_tokens += rollup.prompt_tokens;
        self.completion_tokens += rollup.completion_tokens;
        self.cached_tokens += rollup.cached_tokens;
        self.decode_ms += rollup.decode_ms;
        self.ttft.merge(&rollup.ttft);
        self.queue.merge(&rollup.queue);
    }

    fn rollup(
        self,
        step: i64,
        bucket: i64,
        model_id: String,
        consumer: ModelConsumer,
    ) -> RollupRecord {
        RollupRecord {
            step,
            bucket,
            model_id,
            consumer,
            requests: self.requests,
            errors: self.errors,
            prompt_tokens: self.prompt_tokens,
            completion_tokens: self.completion_tokens,
            cached_tokens: self.cached_tokens,
            decode_ms: self.decode_ms,
            ttft: self.ttft,
            queue: self.queue,
        }
    }
}

fn rolled_key(step: &StatsStep) -> &str {
    match step {
        StatsStep::Minute => "stats_rolled_minute",
        StatsStep::Hour => "stats_rolled_hour",
    }
}

fn retention(step: StatsStep) -> i64 {
    match step {
        StatsStep::Minute => MINUTE_RETENTION_SECONDS,
        StatsStep::Hour => HOUR_RETENTION_SECONDS,
    }
}

fn floor_to(value: i64, step: i64) -> i64 {
    value - value.rem_euclid(step)
}

/// Totals of the last 24 hours for the headline and the fleet snapshot.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DayTotals {
    pub requests: u64,
    pub tokens: u64,
    pub errors: u64,
}

/// Writes request rows in batches on its own connection to `models.sqlite`.
pub struct Stats {
    db: Mutex<ModelsDb>,
}

/// Database work the statistics task runs off the async runtime.
type Work = Box<dyn FnOnce(&Stats) -> Result<()> + Send>;

/// Cheap to clone; a full queue drops rows instead of slowing requests down.
#[derive(Clone)]
pub struct StatsRecorder {
    sender: mpsc::Sender<RequestRecord>,
}

impl StatsRecorder {
    pub fn record(&self, request: RequestRecord) {
        if self.sender.try_send(request).is_err() {
            tracing::warn!("Model gateway statistics queue is full; a request row was dropped");
        }
    }
}

impl Stats {
    pub fn open(database: &Path) -> Result<Self> {
        Ok(Self {
            db: Mutex::new(ModelsDb::open(database)?),
        })
    }

    /// The recorder and the task that writes, rolls up and prunes until `cancel`.
    pub fn start(self: &Arc<Self>, cancel: CancellationToken) -> StatsRecorder {
        let (sender, receiver) = mpsc::channel(QUEUED_ROWS);
        tokio::spawn(Arc::clone(self).run(receiver, cancel));
        StatsRecorder { sender }
    }

    async fn run(
        self: Arc<Self>,
        mut receiver: mpsc::Receiver<RequestRecord>,
        cancel: CancellationToken,
    ) {
        let mut flush = tokio::time::interval(FLUSH_INTERVAL);
        let mut rollup = tokio::time::interval(ROLLUP_INTERVAL);
        let mut pending = Vec::new();
        loop {
            let closing = tokio::select! {
                () = cancel.cancelled() => true,
                received = receiver.recv() => match received {
                    Some(row) => {
                        pending.push(row);
                        if pending.len() < FLUSH_ROWS {
                            continue;
                        }
                        false
                    }
                    None => true,
                },
                _ = flush.tick() => false,
                _ = rollup.tick() => {
                    self.background(Box::new(|stats| stats.maintain(unix_time()?))).await;
                    false
                }
            };
            while let Ok(row) = receiver.try_recv() {
                pending.push(row);
            }
            if !pending.is_empty() {
                let rows = std::mem::take(&mut pending);
                self.background(Box::new(move |stats| stats.write(&rows)))
                    .await;
            }
            if closing {
                return;
            }
        }
    }

    async fn background(self: &Arc<Self>, work: Work) {
        let stats = Arc::clone(self);
        match tokio::task::spawn_blocking(move || work(&stats)).await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => tracing::warn!("Model gateway statistics: {error:#}"),
            Err(error) => tracing::warn!("Model gateway statistics task failed: {error}"),
        }
    }

    pub fn write(&self, rows: &[RequestRecord]) -> Result<()> {
        lock!(self.db).insert_requests(rows)
    }

    /// Rolls up every closed bucket and prunes what outlived its retention.
    pub fn maintain(&self, now: i64) -> Result<()> {
        let db = lock!(self.db);
        for step in STEPS {
            roll_up(&db, step, now)?;
        }
        db.prune_stats(
            now - REQUEST_RETENTION_SECONDS,
            &STEPS.map(|step| (step.seconds(), now - retention(step))),
        )
    }

    /// One value per `step` from `from` up to `to`, plus totals per consumer.
    pub fn query(
        &self,
        model_id: Option<&str>,
        from: i64,
        to: i64,
        step: StatsStep,
    ) -> Result<ModelStats> {
        self.query_shown(model_id, from, to, step, |_| Ok(true))
    }

    /// As [`Stats::query`], with totals only for the consumers `shown` accepts; the series
    /// still count every request.
    pub fn query_shown(
        &self,
        model_id: Option<&str>,
        from: i64,
        to: i64,
        step: StatsStep,
        mut shown: impl FnMut(&ModelConsumer) -> Result<bool>,
    ) -> Result<ModelStats> {
        let seconds = step.seconds();
        let mut buckets: BTreeMap<i64, Bucket> = BTreeMap::new();
        let mut consumers: HashMap<ModelConsumer, Bucket> = HashMap::new();
        {
            let db = lock!(self.db);
            let rolled = rolled_until(&db, step)?.clamp(from, to);
            for rollup in db.rollups_between(seconds, from, rolled, model_id)? {
                buckets
                    .entry(rollup.bucket)
                    .or_default()
                    .add_rollup(&rollup);
                consumers
                    .entry(rollup.consumer.clone())
                    .or_default()
                    .add_rollup(&rollup);
            }
            for request in db.requests_between(rolled, to, model_id)? {
                let bucket = floor_to(request.at, seconds);
                buckets.entry(bucket).or_default().add_request(&request);
                consumers
                    .entry(request.consumer.clone())
                    .or_default()
                    .add_request(&request);
            }
        }
        let mut listed = HashMap::with_capacity(consumers.len());
        for (consumer, totals) in consumers {
            if shown(&consumer)? {
                listed.insert(consumer, totals);
            }
        }
        Ok(ModelStats {
            model_id: model_id.map(str::to_owned),
            from,
            step,
            series: series(&buckets, from, to, seconds),
            consumers: consumer_totals(listed),
        })
    }

    pub fn day_totals(&self, now: i64) -> Result<DayTotals> {
        let to = floor_to(now, 3_600) + 3_600;
        let stats = self.query(None, to - 25 * 3_600, to, StatsStep::Hour)?;
        let series = &stats.series;
        Ok(DayTotals {
            requests: series.requests.iter().sum(),
            tokens: series.prompt_tokens.iter().sum::<u64>()
                + series.completion_tokens.iter().sum::<u64>(),
            errors: series.errors.iter().sum(),
        })
    }
}

fn rolled_until(db: &ModelsDb, step: StatsStep) -> Result<i64> {
    Ok(db
        .setting(rolled_key(&step))?
        .and_then(|value| value.as_i64())
        .unwrap_or(0))
}

/// Aggregates the raw rows of every bucket that closed since the last rollup.
fn roll_up(db: &ModelsDb, step: StatsStep, now: i64) -> Result<()> {
    let seconds = step.seconds();
    let closed = floor_to(now - ROLLUP_GRACE_SECONDS, seconds);
    let from = rolled_until(db, step)?.max(floor_to(now - REQUEST_RETENTION_SECONDS, seconds));
    if from >= closed {
        return Ok(());
    }
    let mut groups: BTreeMap<(i64, String, String, String), (ModelConsumer, Bucket)> =
        BTreeMap::new();
    for request in db.requests_between(from, closed, None)? {
        let bucket = floor_to(request.at, seconds);
        let (kind, id) = consumer_key(&request.consumer);
        groups
            .entry((bucket, request.model_id.clone(), kind.into(), id.into()))
            .or_insert_with(|| (request.consumer.clone(), Bucket::default()))
            .1
            .add_request(&request);
    }
    let rows: Vec<RollupRecord> = groups
        .into_iter()
        .map(|((bucket, model_id, _, _), (consumer, totals))| {
            totals.rollup(seconds, bucket, model_id, consumer)
        })
        .collect();
    db.put_rollups(&rows)?;
    db.set_setting(rolled_key(&step), &closed.into(), now)
        .context("Record the statistics rollup position")
}

fn consumer_key(consumer: &ModelConsumer) -> (&str, &str) {
    match consumer {
        ModelConsumer::Owner => ("owner", ""),
        ModelConsumer::Grant { grant_id } => ("grant", grant_id),
        ModelConsumer::Placement { placement_id } => ("placement", placement_id),
    }
}

fn series(buckets: &BTreeMap<i64, Bucket>, from: i64, to: i64, step: i64) -> ModelStatsSeries {
    let mut series = ModelStatsSeries::default();
    let empty = Bucket::default();
    for start in (from..to).step_by(step as usize) {
        let bucket = buckets.get(&start).unwrap_or(&empty);
        series.requests.push(bucket.requests);
        series.errors.push(bucket.errors);
        series.prompt_tokens.push(bucket.prompt_tokens);
        series.completion_tokens.push(bucket.completion_tokens);
        series.cached_tokens.push(bucket.cached_tokens);
        series.decode_ms.push(bucket.decode_ms);
        series.ttft_p50_ms.push(bucket.ttft.quantile(0.5));
        series.ttft_p95_ms.push(bucket.ttft.quantile(0.95));
        series.queue_wait_p95_ms.push(bucket.queue.quantile(0.95));
    }
    series
}

/// The busiest consumers first, as many as a reply may carry.
fn consumer_totals(consumers: HashMap<ModelConsumer, Bucket>) -> Vec<ModelConsumerTotals> {
    let mut totals: Vec<ModelConsumerTotals> = consumers
        .into_iter()
        .map(|(consumer, bucket)| ModelConsumerTotals {
            consumer,
            requests: bucket.requests,
            errors: bucket.errors,
            prompt_tokens: bucket.prompt_tokens,
            completion_tokens: bucket.completion_tokens,
        })
        .collect();
    totals.sort_by(|left, right| {
        right
            .requests
            .cmp(&left.requests)
            .then_with(|| consumer_key(&left.consumer).cmp(&consumer_key(&right.consumer)))
    });
    totals.truncate(MODEL_STATS_MAX_CONSUMERS);
    totals
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(at: i64, model: &str, consumer: ModelConsumer, ttft: u32) -> RequestRecord {
        RequestRecord {
            at,
            model_id: model.into(),
            consumer,
            status: 200,
            prompt_tokens: 10,
            completion_tokens: 20,
            cached_tokens: 4,
            ttft_ms: Some(ttft),
            duration_ms: 500,
            decode_ms: 400,
            queue_ms: 3,
            request_bytes: 100,
            response_bytes: 900,
        }
    }

    fn stats() -> (tempfile::TempDir, Stats) {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let stats = Stats::open(&directory.path().join("models.sqlite")).expect("statistics");
        (directory, stats)
    }

    #[test]
    fn histogram_quantiles_stay_within_a_bucket() {
        let mut histogram = LatencyHistogram::default();
        for value in 1..=100 {
            histogram.record(value * 10);
        }
        let p50 = histogram.quantile(0.5).unwrap();
        let p95 = histogram.quantile(0.95).unwrap();
        assert!((450..=550).contains(&p50), "{p50}");
        assert!((860..=1_050).contains(&p95), "{p95}");
        assert_eq!(LatencyHistogram::default().quantile(0.5), None);
        let mut zero = LatencyHistogram::default();
        zero.record(0);
        assert_eq!(zero.quantile(0.95), Some(0));
        let mut huge = LatencyHistogram::default();
        huge.record(u32::MAX);
        assert!(huge.quantile(0.5).is_some());
    }

    #[test]
    fn rollups_and_open_buckets_answer_the_same_series() -> Result<()> {
        let (_directory, stats) = stats();
        let base = 1_700_000_000 - 1_700_000_000 % 3_600;
        let owner = ModelConsumer::Owner;
        let placement = ModelConsumer::Placement {
            placement_id: "api".into(),
        };
        let mut failed = request(base + 70, "qwen", placement.clone(), 900);
        failed.status = 503;
        stats.write(&[
            request(base + 5, "qwen", owner.clone(), 100),
            request(base + 65, "qwen", placement.clone(), 300),
            failed,
            request(base + 61, "nomic", owner.clone(), 50),
        ])?;
        let before = stats.query(Some("qwen"), base, base + 180, StatsStep::Minute)?;
        stats.maintain(base + 3_600 + 60)?;
        let after = stats.query(Some("qwen"), base, base + 180, StatsStep::Minute)?;
        assert_eq!(before, after);
        assert_eq!(after.series.requests, vec![1, 2, 0]);
        assert_eq!(after.series.errors, vec![0, 1, 0]);
        assert_eq!(after.series.completion_tokens, vec![20, 40, 0]);
        assert_eq!(after.series.ttft_p50_ms[2], None);
        assert_eq!(after.consumers.len(), 2);
        assert_eq!(after.consumers[0].consumer, placement);
        assert_eq!(after.consumers[0].errors, 1);
        after.validate()?;

        let hourly = stats.query(None, base, base + 3_600, StatsStep::Hour)?;
        assert_eq!(hourly.series.requests, vec![4]);
        Ok(())
    }

    #[test]
    fn retention_prunes_rows_and_rollups() -> Result<()> {
        let (_directory, stats) = stats();
        let old = 1_600_000_000 - 1_600_000_000 % 3_600;
        stats.write(&[request(old + 1, "qwen", ModelConsumer::Owner, 10)])?;
        stats.maintain(old + 3_700)?;
        let rolled = stats.query(None, old, old + 3_600, StatsStep::Hour)?;
        assert_eq!(rolled.series.requests, vec![1]);
        stats.maintain(old + HOUR_RETENTION_SECONDS + 7_200)?;
        let pruned = stats.query(None, old, old + 3_600, StatsStep::Hour)?;
        assert_eq!(pruned.series.requests, vec![0]);
        assert!(
            lock!(stats.db)
                .requests_between(0, i64::MAX, None)?
                .is_empty()
        );
        Ok(())
    }

    #[test]
    fn day_totals_count_tokens_and_errors() -> Result<()> {
        let (_directory, stats) = stats();
        let now = unix_time()?;
        let mut failed = request(now - 10, "qwen", ModelConsumer::Owner, 10);
        failed.status = 429;
        let mut closed = request(now - 5, "qwen", ModelConsumer::Owner, 10);
        closed.status = CLIENT_CLOSED;
        stats.write(&[
            request(now - 3_000, "qwen", ModelConsumer::Owner, 10),
            failed,
            closed,
        ])?;
        let totals = stats.day_totals(now)?;
        assert_eq!(
            totals,
            DayTotals {
                requests: 3,
                tokens: 90,
                errors: 1
            }
        );
        Ok(())
    }
}
