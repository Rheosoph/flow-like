//! Object-storage probes: how much each bucket holds, read from the provider's own
//! metrics pipeline.
//!
//! Nothing here lists objects. A listing is O(objects) with one request per 1000 keys,
//! which would make the dashboard the most expensive request the deployment serves and
//! make it slowest exactly when the bucket is largest. Every provider already computes
//! the two numbers an operator wants — bytes stored and objects held — and publishes
//! them through a metrics API that answers in one call.
//!
//! The price of that is staleness. S3 rolls its storage metrics up once a day, Azure
//! recomputes blob capacity a handful of times a day, GCS measures once a day and then
//! re-reports the same value every five minutes, and Cloudflare samples on a cadence it
//! does not publish. So both numbers are always `MetricFreshness::Provider` and always
//! carry the datapoint's *own* timestamp — never `now`. An operator who reads a
//! 24-hour-old rollup as current will go hunting for a deletion that already happened.

use std::time::Instant;

use flow_like_storage::files::store::{FlowLikeStore, local_store::LocalObjectStore};

use super::types::{ResourceKind, ResourceMetric, ResourceStatus};
use crate::{state::AppState, storage_identity::BucketIdentity};

#[cfg(any(feature = "aws", feature = "azure", feature = "gcp", feature = "r2"))]
use super::types::{MetricFreshness, MetricUnit};
#[cfg(any(feature = "aws", feature = "azure", feature = "gcp", feature = "r2"))]
use crate::storage_identity::StorageProviderKind;

/// Ceiling on one provider metrics call.
///
/// Deliberately well under the per-probe budget in `super`: the three buckets are probed
/// concurrently but often hit the same provider, and a dashboard that hangs on a metrics
/// API is worse than one that says the metrics API is not answering.
#[cfg(any(feature = "aws", feature = "azure", feature = "gcp", feature = "r2"))]
const PROVIDER_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);
#[cfg(any(feature = "azure", feature = "gcp", feature = "r2"))]
const CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

#[cfg(any(feature = "aws", feature = "azure", feature = "gcp", feature = "r2"))]
const SIZE_KEY: &str = "size_bytes";
#[cfg(any(feature = "aws", feature = "azure", feature = "gcp", feature = "r2"))]
const COUNT_KEY: &str = "object_count";

/// Probe every bucket this deployment serves from.
pub async fn probe(state: &AppState) -> Vec<ResourceStatus> {
    let identities = &state.storage_identity;
    let targets = [
        (
            "storage:meta",
            "Meta bucket",
            &identities.meta,
            state.meta_bucket.as_ref(),
        ),
        (
            "storage:content",
            "Content bucket",
            &identities.content,
            state.content_bucket.as_ref(),
        ),
        (
            "storage:cdn",
            "CDN bucket",
            &identities.cdn,
            state.cdn_bucket.as_ref(),
        ),
    ];

    futures::future::join_all(
        targets
            .into_iter()
            .map(|(id, label, identity, store)| probe_bucket(state, id, label, identity, store)),
    )
    .await
}

/// What one bucket's probe concluded.
enum ProbeOutcome {
    /// Metrics, plus an optional caveat about the set as a whole.
    Metrics(Vec<ResourceMetric>, Option<String>),
    /// Configured and reachable, but no cheap statistics exist for it.
    Unsupported(String),
    Failed(String),
}

async fn probe_bucket(
    state: &AppState,
    id: &str,
    label: &str,
    identity: &BucketIdentity,
    store: &FlowLikeStore,
) -> ResourceStatus {
    let started = Instant::now();

    let outcome = match store {
        FlowLikeStore::Local(local) => local_capacity(local),
        FlowLikeStore::Memory(_) => ProbeOutcome::Unsupported(
            "In-memory object store: its contents live in this process only and are not \
             measured here"
                .to_string(),
        ),
        _ => provider_metrics(state, identity).await,
    };

    let status = ResourceStatus::new(
        id,
        ResourceKind::Storage,
        label,
        format!("{}/{}", identity.provider, store_kind(store)),
    )
    .detail(identity.describe())
    .latency_ms(started.elapsed().as_millis() as u64);

    match outcome {
        ProbeOutcome::Metrics(metrics, message) => {
            let status = status.metrics(metrics);
            match message {
                Some(message) => status.message(message),
                None => status,
            }
        }
        ProbeOutcome::Unsupported(reason) => status.unsupported(reason),
        ProbeOutcome::Failed(reason) => status.failed(reason),
    }
}

fn store_kind(store: &FlowLikeStore) -> &'static str {
    match store {
        FlowLikeStore::Local(_) => "filesystem",
        FlowLikeStore::AWS(_) => "s3",
        FlowLikeStore::Azure(_) => "azure-blob",
        FlowLikeStore::Google(_) => "gcs",
        FlowLikeStore::Memory(_) => "memory",
        FlowLikeStore::Other(_) => "other",
    }
}

#[cfg_attr(not(feature = "aws"), allow(unused_variables))]
async fn provider_metrics(state: &AppState, identity: &BucketIdentity) -> ProbeOutcome {
    if identity.name.is_empty() {
        return ProbeOutcome::Unsupported(
            "No bucket name is configured for this store, so no provider metric can be addressed"
                .to_string(),
        );
    }

    match identity.provider {
        #[cfg(feature = "aws")]
        StorageProviderKind::Aws => timed("CloudWatch", aws::metrics(state, identity)).await,
        #[cfg(feature = "azure")]
        StorageProviderKind::Azure => timed("Azure Monitor", azure::metrics(identity)).await,
        #[cfg(feature = "gcp")]
        StorageProviderKind::Gcp => timed("Cloud Monitoring", gcp::metrics(identity)).await,
        #[cfg(feature = "r2")]
        StorageProviderKind::R2 => timed("Cloudflare analytics", r2::metrics(identity)).await,
        other => ProbeOutcome::Unsupported(format!(
            "No cheap storage-metrics API is wired up for a '{other}' store, and this endpoint \
             will not list objects to derive one"
        )),
    }
}

#[cfg(any(feature = "aws", feature = "azure", feature = "gcp", feature = "r2"))]
async fn timed(
    provider: &str,
    probe: impl std::future::Future<Output = ProbeOutcome>,
) -> ProbeOutcome {
    match flow_like_types::tokio::time::timeout(PROVIDER_TIMEOUT, probe).await {
        Ok(outcome) => outcome,
        Err(_) => ProbeOutcome::Failed(format!(
            "{provider} did not answer within {}s",
            PROVIDER_TIMEOUT.as_secs()
        )),
    }
}

/// Filesystem capacity for a local store.
///
/// The one place a real number is cheap, because `statvfs` is O(1). It is also the one
/// place the number answers a different question than it does everywhere else — it
/// describes the volume, not this bucket — so every metric says so.
fn local_capacity(store: &LocalObjectStore) -> ProbeOutcome {
    let root =
        match store.path_to_filesystem(&flow_like_storage::object_store::path::Path::default()) {
            Ok(root) => root,
            Err(error) => {
                return ProbeOutcome::Failed(format!("Local store root is unreadable: {error}"));
            }
        };

    let (total, available) = match (fs4::total_space(&root), fs4::available_space(&root)) {
        (Ok(total), Ok(available)) => (total, available),
        (Err(error), _) | (_, Err(error)) => {
            return ProbeOutcome::Failed(format!(
                "Filesystem holding {} could not be measured: {error}",
                root.display()
            ));
        }
    };

    let note = format!(
        "Whole filesystem holding {}, not just this bucket's prefix",
        root.display()
    );
    let metrics = vec![
        ResourceMetric::bytes("disk_total_bytes", "Disk total", total as i64).note(note.clone()),
        ResourceMetric::bytes("disk_available_bytes", "Disk available", available as i64)
            .note(note.clone()),
        ResourceMetric::bytes(
            "disk_used_bytes",
            "Disk used",
            total.saturating_sub(available) as i64,
        )
        .note(format!(
            "{note}. Derived as total minus available, so it also counts other data on the \
             volume and the blocks reserved for root"
        )),
    ];

    ProbeOutcome::Metrics(
        metrics,
        Some(
            "Stored size and object count are not reported for a local store: totalling them \
             means walking every object, which this endpoint deliberately refuses to do."
                .to_string(),
        ),
    )
}

/// Build a metric from a provider rollup.
///
/// Every such value is `Provider` freshness and carries the datapoint's own timestamp.
/// When the provider hands back no timestamp the metric says its age is unknown rather
/// than borrowing the current time — a fabricated "as of now" is the exact error this
/// endpoint exists to avoid.
#[cfg(any(feature = "aws", feature = "azure", feature = "gcp", feature = "r2"))]
fn provider_metric(
    key: &str,
    label: &str,
    value: f64,
    unit: MetricUnit,
    observed_at: Option<String>,
    note: &str,
) -> ResourceMetric {
    const UNKNOWN_AGE: &str =
        "The provider returned no timestamp for this datapoint, so its age is unknown";

    let metric = ResourceMetric::new(key, label, value, unit, MetricFreshness::Provider);
    let (metric, note) = match observed_at {
        Some(observed_at) => (metric.observed_at(observed_at), note.to_string()),
        None if note.is_empty() => (metric, UNKNOWN_AGE.to_string()),
        None => (metric, format!("{note}. {UNKNOWN_AGE}")),
    };

    if note.is_empty() {
        metric
    } else {
        metric.note(note)
    }
}

#[cfg(any(feature = "azure", feature = "gcp", feature = "r2"))]
fn http_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .https_only(true)
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(PROVIDER_TIMEOUT)
        .build()
        .map_err(|error| format!("Could not build the metrics HTTP client: {error}"))
}

#[cfg(any(feature = "azure", feature = "gcp", feature = "r2"))]
fn env_non_empty(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

#[cfg(any(feature = "azure", feature = "gcp", feature = "r2"))]
fn rfc3339(at: chrono::DateTime<chrono::Utc>) -> String {
    at.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

#[cfg(any(feature = "azure", feature = "gcp", feature = "r2"))]
fn truncated_body(body: String) -> String {
    body.chars().take(300).collect()
}

#[cfg(feature = "aws")]
mod aws {
    //! S3 storage metrics from CloudWatch.
    //!
    //! Two `GetMetricStatistics` calls rather than one `GetMetricData`: the batched
    //! operation is one of the three AWS always charges and never puts in the free tier,
    //! while `GetMetricStatistics` falls under the million-request monthly allowance.

    use super::{COUNT_KEY, MetricUnit, ProbeOutcome, SIZE_KEY, provider_metric};
    use crate::{state::AppState, storage_identity::BucketIdentity};
    use aws_sdk_cloudwatch::{
        Client,
        error::DisplayErrorContext,
        primitives::DateTime,
        types::{Dimension, Statistic},
    };

    /// S3 publishes these once a day and AWS guarantees neither completeness nor
    /// timeliness, so a same-day window intermittently comes back empty. A week costs
    /// exactly the same single request.
    const LOOKBACK_DAYS: i64 = 7;
    const PERIOD_SECONDS: i32 = 86_400;

    /// `BucketSizeBytes` exists per storage class only — there is no `AllStorageTypes`
    /// aggregate for it — while `NumberOfObjects` exists *only* under `AllStorageTypes`.
    /// Swapping the two returns HTTP 200 with an empty datapoint list, which is
    /// indistinguishable from an empty bucket.
    const SIZE_STORAGE_TYPE: &str = "StandardStorage";
    const COUNT_STORAGE_TYPE: &str = "AllStorageTypes";

    pub(super) async fn metrics(state: &AppState, identity: &BucketIdentity) -> ProbeOutcome {
        if let Some(endpoint) = &identity.endpoint {
            return ProbeOutcome::Unsupported(format!(
                "AWS_ENDPOINT points at {endpoint}, so this is an S3-compatible store (MinIO, \
                 Ceph, R2 over the S3 API) rather than AWS S3. CloudWatch holds no AWS/S3 \
                 metrics for it, and asking anyway would return an empty result that reads \
                 exactly like an empty bucket."
            ));
        }

        let Some(region) = identity.region.clone() else {
            return ProbeOutcome::Unsupported(
                "No AWS region is configured. S3 publishes a bucket's storage metrics into \
                 CloudWatch in the bucket's own region, and a query aimed at another region \
                 returns an empty result rather than an error."
                    .to_string(),
            );
        };

        let config = aws_sdk_cloudwatch::config::Builder::from(state.aws_client.as_ref())
            .region(aws_sdk_cloudwatch::config::Region::new(region.clone()))
            .build();
        let client = Client::from_conf(config);

        let now = chrono::Utc::now().timestamp();
        let window = (now - LOOKBACK_DAYS * 86_400, now);

        let (size, count) = futures::future::join(
            latest(
                &client,
                &identity.name,
                "BucketSizeBytes",
                SIZE_STORAGE_TYPE,
                window,
            ),
            latest(
                &client,
                &identity.name,
                "NumberOfObjects",
                COUNT_STORAGE_TYPE,
                window,
            ),
        )
        .await;

        let mut metrics = Vec::new();
        let mut errors = Vec::new();
        let mut empty = Vec::new();

        match size {
            Ok(Some((value, at))) => metrics.push(provider_metric(
                SIZE_KEY,
                "Size",
                value,
                MetricUnit::Bytes,
                observed_at(at),
                "S3 Standard only — bytes tiered to Glacier, Intelligent-Tiering or \
                 Standard-IA are not counted",
            )),
            Ok(None) => empty.push("BucketSizeBytes"),
            Err(error) => errors.push(error),
        }

        match count {
            Ok(Some((value, at))) => metrics.push(provider_metric(
                COUNT_KEY,
                "Objects",
                value,
                MetricUnit::Count,
                observed_at(at),
                "All storage classes, including noncurrent versions, delete markers and every \
                 part of an incomplete multipart upload",
            )),
            Ok(None) => empty.push("NumberOfObjects"),
            Err(error) => errors.push(error),
        }

        if metrics.is_empty() {
            if !errors.is_empty() {
                return ProbeOutcome::Failed(errors.join("; "));
            }
            return ProbeOutcome::Unsupported(no_datapoints(&identity.name, &region));
        }

        let mut caveats = errors;
        if !empty.is_empty() {
            caveats.push(format!(
                "{} returned no datapoints. {}",
                empty.join(" and "),
                no_datapoints(&identity.name, &region)
            ));
        }

        ProbeOutcome::Metrics(metrics, (!caveats.is_empty()).then(|| caveats.join(" ")))
    }

    fn no_datapoints(bucket: &str, region: &str) -> String {
        format!(
            "CloudWatch holds no S3 storage datapoints for '{bucket}' in {region} over the past \
             {days} days. A bucket that is new, has always been empty, or lives in another \
             region publishes none — which is why this reports as unknown rather than as zero.",
            days = LOOKBACK_DAYS
        )
    }

    fn observed_at(at: i64) -> Option<String> {
        chrono::DateTime::from_timestamp(at, 0).map(|at| at.to_rfc3339())
    }

    /// Newest datapoint as `(average, epoch seconds)`.
    ///
    /// CloudWatch explicitly does not return datapoints in chronological order, so the
    /// newest has to be selected by timestamp rather than by position.
    async fn latest(
        client: &Client,
        bucket: &str,
        metric: &str,
        storage_type: &str,
        window: (i64, i64),
    ) -> Result<Option<(f64, i64)>, String> {
        // `unit` is deliberately unset: CloudWatch performs no unit conversion and nulls
        // the whole result when the requested unit does not match what was published.
        let output = client
            .get_metric_statistics()
            .namespace("AWS/S3")
            .metric_name(metric)
            .dimensions(
                Dimension::builder()
                    .name("BucketName")
                    .value(bucket)
                    .build(),
            )
            .dimensions(
                Dimension::builder()
                    .name("StorageType")
                    .value(storage_type)
                    .build(),
            )
            .start_time(DateTime::from_secs(window.0))
            .end_time(DateTime::from_secs(window.1))
            .period(PERIOD_SECONDS)
            .statistics(Statistic::Average)
            .send()
            .await
            .map_err(|error| {
                format!(
                    "CloudWatch {metric} query failed: {}",
                    DisplayErrorContext(&error)
                )
            })?;

        Ok(output
            .datapoints()
            .iter()
            .filter_map(|point| Some((point.average()?, point.timestamp()?.secs())))
            .max_by_key(|(_, at)| *at))
    }
}

#[cfg(feature = "azure")]
mod azure {
    //! Blob capacity and blob count from Azure Monitor's management plane.
    //!
    //! These metrics live on the `blobServices/default` sub-resource, not on the storage
    //! account itself, and they are **account-wide**: there is no per-container capacity
    //! on this path. Meta, content and CDN therefore report the same number whenever they
    //! share an account, and every metric carries a note saying so.

    use super::{
        COUNT_KEY, MetricUnit, ProbeOutcome, SIZE_KEY, env_non_empty, http_client, provider_metric,
        rfc3339, truncated_body,
    };
    use crate::storage_identity::BucketIdentity;
    use serde::Deserialize;

    const API_VERSION: &str = "2023-10-01";
    const NAMESPACE: &str = "Microsoft.Storage/storageAccounts/blobServices";
    const MANAGEMENT_SCOPE: &str = "https://management.azure.com/.default";
    /// Capacity is recomputed only a few times a day, so a short window frequently holds
    /// nothing at all.
    const LOOKBACK_HOURS: i64 = 24;
    /// `top` silently defaults to 10 whenever a `$filter` is present, and a full
    /// BlobType × Tier split can exceed that — which would truncate the sum with no error.
    const TOP: u32 = 200;

    const ACCOUNT_WIDE: &str = "Azure reports blob capacity per storage account, not per container: this covers every \
         container in the account";

    pub(super) async fn metrics(identity: &BucketIdentity) -> ProbeOutcome {
        let (Some(subscription), Some(resource_group), Some(account)) = (
            env_non_empty("AZURE_SUBSCRIPTION_ID"),
            env_non_empty("AZURE_RESOURCE_GROUP"),
            identity.account.clone(),
        ) else {
            return ProbeOutcome::Unsupported(
                "Azure Monitor needs the storage account's full ARM resource id. Set \
                 AZURE_SUBSCRIPTION_ID and AZURE_RESOURCE_GROUP (AZURE_STORAGE_ACCOUNT_NAME is \
                 already read) and grant the deployment's identity the Monitoring Reader role \
                 on that account."
                    .to_string(),
            );
        };

        let token = match management_token().await {
            Ok(token) => token,
            Err(error) => return ProbeOutcome::Failed(error),
        };
        let client = match http_client() {
            Ok(client) => client,
            Err(error) => return ProbeOutcome::Failed(error),
        };

        let end = chrono::Utc::now();
        let start = end - chrono::Duration::hours(LOOKBACK_HOURS);
        let encode = urlencoding::encode;
        let url = format!(
            "https://management.azure.com/subscriptions/{subscription}/resourceGroups/\
             {group}/providers/Microsoft.Storage/storageAccounts/{account}/blobServices/default/\
             providers/Microsoft.Insights/metrics?api-version={version}\
             &metricnamespace={namespace}&metricnames=BlobCapacity,BlobCount\
             &aggregation=Average&interval=PT1H&timespan={start}/{end}&$filter={filter}\
             &top={top}&AutoAdjustTimegrain=true&ValidateDimensions=false",
            subscription = encode(&subscription),
            group = encode(&resource_group),
            account = encode(&account),
            version = API_VERSION,
            namespace = encode(NAMESPACE),
            start = encode(&rfc3339(start)),
            end = encode(&rfc3339(end)),
            // Splitting on both dimensions and summing the raw per-series samples is the
            // only unambiguous total: an unfiltered `Average` rollup may be the mean
            // across tiers rather than their sum, and Microsoft documents neither.
            filter = encode("BlobType eq '*' and Tier eq '*'"),
            top = TOP,
        );

        let response = match client.get(url).bearer_auth(token).send().await {
            Ok(response) => response,
            Err(error) => {
                return ProbeOutcome::Failed(format!("Azure Monitor request failed: {error}"));
            }
        };

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return ProbeOutcome::Failed(format!(
                "Azure Monitor returned HTTP {status}: {}",
                truncated_body(body)
            ));
        }

        let payload: MetricsResponse = match response.json().await {
            Ok(payload) => payload,
            Err(error) => {
                return ProbeOutcome::Failed(format!(
                    "Azure Monitor response could not be parsed: {error}"
                ));
            }
        };

        let mut metrics = Vec::new();
        let mut problems = Vec::new();

        // Per-metric failures arrive in-band with HTTP 200, so `errorCode` has to be read
        // even on a successful response.
        for metric in &payload.value {
            if let Some(code) = metric.error_code.as_deref()
                && !code.eq_ignore_ascii_case("Success")
            {
                problems.push(format!(
                    "{}: {code}{}",
                    metric.name.value,
                    metric
                        .error_message
                        .as_deref()
                        .map(|message| format!(" ({message})"))
                        .unwrap_or_default()
                ));
                continue;
            }

            let Some((value, observed_at)) = newest_sum(metric) else {
                continue;
            };

            match metric.name.value.as_str() {
                "BlobCapacity" => metrics.push(provider_metric(
                    SIZE_KEY,
                    "Size",
                    value,
                    MetricUnit::Bytes,
                    Some(observed_at),
                    ACCOUNT_WIDE,
                )),
                "BlobCount" => metrics.push(provider_metric(
                    COUNT_KEY,
                    "Objects",
                    value,
                    MetricUnit::Count,
                    Some(observed_at),
                    ACCOUNT_WIDE,
                )),
                _ => {}
            }
        }

        if metrics.is_empty() {
            if !problems.is_empty() {
                return ProbeOutcome::Failed(problems.join("; "));
            }
            return ProbeOutcome::Unsupported(format!(
                "Azure Monitor returned no blob capacity datapoints for account '{account}' in \
                 the past {hours} hours. Capacity is recomputed only a few times a day, so this \
                 reports as unknown rather than as zero.",
                hours = LOOKBACK_HOURS
            ));
        }

        let mut caveats = vec![format!(
            "BlobCapacity and BlobCount cover the whole '{account}' storage account, so every \
             card sharing that account shows the same figure."
        )];
        caveats.extend(problems);

        ProbeOutcome::Metrics(metrics, Some(caveats.join(" ")))
    }

    /// Sum every time series at the newest timestamp that carries a value.
    ///
    /// Summing across dimensions at one instant is correct; summing across time buckets
    /// would double-count a gauge. Azure emits RFC 3339 in UTC with a fixed shape, so a
    /// lexicographic maximum is a chronological one.
    fn newest_sum(metric: &Metric) -> Option<(f64, String)> {
        let newest = metric
            .timeseries
            .iter()
            .flat_map(|series| series.data.iter())
            .filter(|point| point.average.is_some())
            .map(|point| point.time_stamp.as_str())
            .max()?
            .to_string();

        let total = metric
            .timeseries
            .iter()
            .flat_map(|series| series.data.iter())
            .filter(|point| point.time_stamp == newest)
            .filter_map(|point| point.average)
            .sum();

        Some((total, newest))
    }

    /// Mirrors the managed-identity handling in `credentials::azure_credentials`,
    /// including the user-assigned identity named by `AZURE_CLIENT_ID`, so the dashboard
    /// authenticates as the same principal the storage plane does.
    async fn management_token() -> Result<String, String> {
        use azure_core::credentials::TokenCredential;
        use azure_identity::{
            ManagedIdentityCredential, ManagedIdentityCredentialOptions, UserAssignedId,
        };

        let user_assigned_id = env_non_empty("AZURE_CLIENT_ID").map(UserAssignedId::ClientId);
        let credential = ManagedIdentityCredential::new(Some(ManagedIdentityCredentialOptions {
            user_assigned_id,
            ..Default::default()
        }))
        .map_err(|error| format!("Could not initialise the Azure managed identity: {error}"))?;

        let token = credential
            .get_token(&[MANAGEMENT_SCOPE], None)
            .await
            .map_err(|error| {
                format!(
                    "Could not acquire an Azure management token: {error}. The identity needs \
                     the Monitoring Reader role on the storage account."
                )
            })?;

        Ok(token.token.secret().to_string())
    }

    #[derive(Deserialize)]
    struct MetricsResponse {
        #[serde(default)]
        value: Vec<Metric>,
    }

    #[derive(Deserialize)]
    struct Metric {
        name: Localizable,
        #[serde(rename = "errorCode")]
        error_code: Option<String>,
        #[serde(rename = "errorMessage")]
        error_message: Option<String>,
        #[serde(default)]
        timeseries: Vec<TimeSeries>,
    }

    #[derive(Deserialize)]
    struct Localizable {
        value: String,
    }

    #[derive(Deserialize)]
    struct TimeSeries {
        #[serde(default)]
        data: Vec<MetricValue>,
    }

    /// Every field but the timestamp is genuinely absent from the JSON for a bucket with
    /// no data, so `average` must stay optional.
    #[derive(Deserialize)]
    struct MetricValue {
        #[serde(rename = "timeStamp")]
        time_stamp: String,
        average: Option<f64>,
    }
}

#[cfg(feature = "gcp")]
mod gcp {
    //! Bucket size and object count from Cloud Monitoring.
    //!
    //! One request per metric, because a monitoring filter names exactly one metric type.
    //! Both are reduced server-side to a single aligned point, so each call bills as one
    //! time series and no client-side summing across storage classes is needed.

    use super::{
        COUNT_KEY, MetricUnit, ProbeOutcome, SIZE_KEY, env_non_empty, http_client, provider_metric,
        rfc3339, truncated_body,
    };
    use crate::{credentials::gcp_credentials, storage_identity::BucketIdentity};
    use serde::Deserialize;

    const ENDPOINT: &str = "https://monitoring.googleapis.com/v3/projects";
    /// The value is measured once a day and re-reported every 300s; 26 hours reliably
    /// spans one measurement even when the daily job ran late.
    const LOOKBACK_SECONDS: i64 = 26 * 3_600;

    const DAILY: &str = "GCS measures bucket totals once a day and repeats that value at every reporting \
         interval, so the timestamp can be minutes old while the number is up to a day old";

    pub(super) async fn metrics(identity: &BucketIdentity) -> ProbeOutcome {
        let Some(project) =
            env_non_empty("GCP_PROJECT_ID").or_else(|| env_non_empty("GOOGLE_CLOUD_PROJECT"))
        else {
            return ProbeOutcome::Unsupported(
                "Cloud Monitoring is addressed per project: set GCP_PROJECT_ID (or \
                 GOOGLE_CLOUD_PROJECT) to the project that owns the bucket, and grant the \
                 workload roles/monitoring.viewer there."
                    .to_string(),
            );
        };

        let token = match access_token().await {
            Ok(token) => token,
            Err(error) => return ProbeOutcome::Failed(error),
        };
        let client = match http_client() {
            Ok(client) => client,
            Err(error) => return ProbeOutcome::Failed(error),
        };

        let end = chrono::Utc::now();
        let start = end - chrono::Duration::seconds(LOOKBACK_SECONDS);
        let window = (rfc3339(start), rfc3339(end));

        let (size, count) = futures::future::join(
            latest(
                &client,
                &token,
                &project,
                &identity.name,
                "storage.googleapis.com/storage/total_bytes",
                &window,
            ),
            latest(
                &client,
                &token,
                &project,
                &identity.name,
                "storage.googleapis.com/storage/object_count",
                &window,
            ),
        )
        .await;

        let mut metrics = Vec::new();
        let mut errors = Vec::new();
        let mut empty = false;

        for (result, key, label, unit) in [
            (size, SIZE_KEY, "Size", MetricUnit::Bytes),
            (count, COUNT_KEY, "Objects", MetricUnit::Count),
        ] {
            match result {
                Ok(Some((value, observed_at))) => metrics.push(provider_metric(
                    key,
                    label,
                    value,
                    unit,
                    Some(observed_at),
                    DAILY,
                )),
                Ok(None) => empty = true,
                Err(error) => errors.push(error),
            }
        }

        if metrics.is_empty() {
            if !errors.is_empty() {
                return ProbeOutcome::Failed(errors.join("; "));
            }
            return ProbeOutcome::Unsupported(no_data(&identity.name));
        }

        let mut caveats = errors;
        if empty {
            caveats.push(no_data(&identity.name));
        }

        ProbeOutcome::Metrics(metrics, (!caveats.is_empty()).then(|| caveats.join(" ")))
    }

    fn no_data(bucket: &str) -> String {
        format!(
            "Cloud Monitoring returned no time series for '{bucket}'. Google does not track \
             buckets holding no objects, and a bucket created today has not been measured yet — \
             so this reports as unknown rather than as zero."
        )
    }

    /// The workload's own token on Cloud Run and GKE carries `cloud-platform`, which
    /// covers `monitoring.read`. Off-GCP the service-account key has to be signed for the
    /// monitoring scope explicitly: the storage scope this module normally mints cannot
    /// read Cloud Monitoring at all.
    async fn access_token() -> Result<String, String> {
        let token = match gcp_credentials::service_account_key_from_env() {
            Some(key) => {
                gcp_credentials::generate_access_token_standalone(
                    &key,
                    gcp_credentials::MONITORING_READ_SCOPE,
                )
                .await
            }
            None => gcp_credentials::fetch_metadata_token().await,
        };

        token.map_err(|error| format!("Could not mint a Cloud Monitoring token: {error}"))
    }

    async fn latest(
        client: &reqwest::Client,
        token: &str,
        project: &str,
        bucket: &str,
        metric_type: &str,
        window: &(String, String),
    ) -> Result<Option<(f64, String)>, String> {
        let filter = format!(
            "metric.type=\"{metric_type}\" AND resource.type=\"gcs_bucket\" AND \
             resource.labels.bucket_name=\"{bucket}\""
        );
        // ALIGN_NEXT_OLDER and REDUCE_SUM both preserve the metric's value type, which is
        // what keeps object_count an int64 rather than silently becoming a double, and
        // the reducer is what folds the per-storage-class series into one total.
        let alignment = format!("{seconds}s", seconds = LOOKBACK_SECONDS);
        let query = [
            ("filter", filter.as_str()),
            ("interval.startTime", window.0.as_str()),
            ("interval.endTime", window.1.as_str()),
            ("aggregation.alignmentPeriod", alignment.as_str()),
            ("aggregation.perSeriesAligner", "ALIGN_NEXT_OLDER"),
            ("aggregation.crossSeriesReducer", "REDUCE_SUM"),
            ("aggregation.groupByFields", "resource.label.bucket_name"),
            ("view", "FULL"),
        ];

        let response = client
            .get(format!("{ENDPOINT}/{project}/timeSeries"))
            .bearer_auth(token)
            .query(&query)
            .send()
            .await
            .map_err(|error| format!("Cloud Monitoring request failed: {error}"))?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(format!(
                "Cloud Monitoring returned HTTP {status}: {}",
                truncated_body(body)
            ));
        }

        let payload: TimeSeriesResponse = response
            .json()
            .await
            .map_err(|error| format!("Cloud Monitoring response could not be parsed: {error}"))?;

        // Points come back newest first and the order cannot be overridden, so the head of
        // the single reduced series is the current value.
        Ok(payload
            .time_series
            .first()
            .and_then(|series| series.points.first())
            .and_then(|point| Some((point.value.as_f64()?, point.interval.end_time.clone()))))
    }

    #[derive(Deserialize)]
    struct TimeSeriesResponse {
        /// Absent entirely — the body is literally `{}` — for a bucket Google does not
        /// track, which is not the same as a bucket holding zero bytes.
        #[serde(default, rename = "timeSeries")]
        time_series: Vec<TimeSeries>,
    }

    #[derive(Deserialize)]
    struct TimeSeries {
        #[serde(default)]
        points: Vec<Point>,
    }

    #[derive(Deserialize)]
    struct Point {
        interval: Interval,
        value: TypedValue,
    }

    #[derive(Deserialize)]
    struct Interval {
        #[serde(rename = "endTime")]
        end_time: String,
    }

    /// `int64Value` is serialised as a JSON string, `doubleValue` as a number.
    #[derive(Deserialize)]
    struct TypedValue {
        #[serde(rename = "doubleValue")]
        double_value: Option<f64>,
        #[serde(rename = "int64Value")]
        int64_value: Option<String>,
    }

    impl TypedValue {
        fn as_f64(&self) -> Option<f64> {
            self.double_value
                .or_else(|| self.int64_value.as_ref()?.parse().ok())
        }
    }
}

#[cfg(feature = "r2")]
mod r2 {
    //! Bucket storage from the Cloudflare GraphQL analytics API.
    //!
    //! `r2StorageAdaptiveGroups` is the only dataset carrying size and object count, and
    //! `max` is its only aggregation node. Pinning the newest sample with `limit: 1` plus
    //! `orderBy: [datetime_DESC]` is exactly what `wrangler r2 bucket info` does.

    use super::{
        COUNT_KEY, MetricUnit, ProbeOutcome, SIZE_KEY, env_non_empty, http_client, provider_metric,
        rfc3339, truncated_body,
    };
    use crate::storage_identity::BucketIdentity;
    use serde::Deserialize;

    const ENDPOINT: &str = "https://api.cloudflare.com/client/v4/graphql";
    /// Cloudflare publishes no sampling cadence; 24 hours is what wrangler and the R2
    /// dashboard use, and a shorter window legitimately comes back empty.
    const LOOKBACK_HOURS: i64 = 24;

    const QUERY: &str = "query R2Storage($accountTag: string!, $start: Time, $end: Time, $bucketName: string) { viewer { accounts(filter: {accountTag: $accountTag}) { r2StorageAdaptiveGroups(limit: 1, filter: {datetime_geq: $start, datetime_leq: $end, bucketName: $bucketName}, orderBy: [datetime_DESC]) { max { objectCount payloadSize metadataSize } dimensions { datetime } } } } }";

    pub(super) async fn metrics(identity: &BucketIdentity) -> ProbeOutcome {
        let Some(token) = env_non_empty("R2_API_TOKEN") else {
            return ProbeOutcome::Unsupported(
                "R2_API_TOKEN is not set. Cloudflare's GraphQL analytics API is the only cheap \
                 source of R2 bucket size, and the token also needs the Account Analytics: Read \
                 permission — the Workers R2 Storage permission used for signed URLs does not \
                 grant it."
                    .to_string(),
            );
        };

        let Some(account) = identity.account.clone() else {
            return ProbeOutcome::Unsupported(
                "R2_ACCOUNT_ID is not set; the Cloudflare analytics query is scoped by account \
                 tag."
                    .to_string(),
            );
        };

        let client = match http_client() {
            Ok(client) => client,
            Err(error) => return ProbeOutcome::Failed(error),
        };

        let end = chrono::Utc::now();
        let start = end - chrono::Duration::hours(LOOKBACK_HOURS);
        let body = serde_json::json!({
            "query": QUERY,
            "operationName": "R2Storage",
            "variables": {
                "accountTag": account,
                "start": rfc3339(start),
                "end": rfc3339(end),
                "bucketName": identity.name,
            },
        });

        let response = match client
            .post(ENDPOINT)
            .bearer_auth(token)
            .json(&body)
            .send()
            .await
        {
            Ok(response) => response,
            Err(error) => {
                return ProbeOutcome::Failed(format!(
                    "Cloudflare analytics request failed: {error}"
                ));
            }
        };

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return ProbeOutcome::Failed(format!(
                "Cloudflare analytics returned HTTP {status}: {}",
                truncated_body(body)
            ));
        }

        let payload: GraphQlResponse = match response.json().await {
            Ok(payload) => payload,
            Err(error) => {
                return ProbeOutcome::Failed(format!(
                    "Cloudflare analytics response could not be parsed: {error}"
                ));
            }
        };

        // The endpoint answers 200 for authentication and query errors alike, so the
        // errors array has to be read before the data.
        if !payload.errors.is_empty() {
            return ProbeOutcome::Failed(format!(
                "Cloudflare analytics rejected the query: {}. A permission error here usually \
                 means R2_API_TOKEN lacks Account Analytics: Read.",
                payload
                    .errors
                    .iter()
                    .map(|error| error.message.as_str())
                    .collect::<Vec<_>>()
                    .join("; ")
            ));
        }

        let sample = payload
            .data
            .and_then(|data| data.viewer)
            .and_then(|viewer| viewer.accounts.into_iter().next())
            .and_then(|account| account.groups.into_iter().next());

        let Some(sample) = sample else {
            return ProbeOutcome::Unsupported(format!(
                "Cloudflare returned no storage sample for '{bucket}' in account '{account}' \
                 over the past {hours} hours. An empty result also covers a wrong bucket name \
                 and a token that cannot see the account, so this reports as unknown rather \
                 than as zero.",
                bucket = identity.name,
                hours = LOOKBACK_HOURS
            ));
        };

        let observed_at = sample.dimensions.and_then(|dimensions| dimensions.datetime);
        let Some(max) = sample.max else {
            return ProbeOutcome::Unsupported(
                "Cloudflare returned a storage sample carrying no values".to_string(),
            );
        };

        let stored = max.payload_size.unwrap_or_default() + max.metadata_size.unwrap_or_default();
        let mut metrics = vec![provider_metric(
            SIZE_KEY,
            "Size",
            stored as f64,
            MetricUnit::Bytes,
            observed_at.clone(),
            "Object payload plus metadata, as `wrangler r2 bucket info` reports it; grouped by \
             sample time alone, so a bucket mixing storage classes may under-report",
        )];

        if let Some(object_count) = max.object_count {
            metrics.push(provider_metric(
                COUNT_KEY,
                "Objects",
                object_count as f64,
                MetricUnit::Count,
                observed_at,
                "",
            ));
        }

        ProbeOutcome::Metrics(metrics, None)
    }

    #[derive(Deserialize)]
    struct GraphQlResponse {
        data: Option<Data>,
        #[serde(default)]
        errors: Vec<GraphQlError>,
    }

    #[derive(Deserialize)]
    struct GraphQlError {
        message: String,
    }

    #[derive(Deserialize)]
    struct Data {
        viewer: Option<Viewer>,
    }

    #[derive(Deserialize)]
    struct Viewer {
        #[serde(default)]
        accounts: Vec<Account>,
    }

    #[derive(Deserialize)]
    struct Account {
        #[serde(rename = "r2StorageAdaptiveGroups", default)]
        groups: Vec<StorageGroup>,
    }

    #[derive(Deserialize)]
    struct StorageGroup {
        max: Option<StorageMax>,
        dimensions: Option<StorageDimensions>,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct StorageMax {
        payload_size: Option<u64>,
        metadata_size: Option<u64>,
        object_count: Option<u64>,
    }

    #[derive(Deserialize)]
    struct StorageDimensions {
        datetime: Option<String>,
    }
}
