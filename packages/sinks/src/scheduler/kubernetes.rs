//! Kubernetes CronJob scheduler implementation
//!
//! Creates and manages Kubernetes CronJob resources that trigger
//! the sink-trigger container to call the central API.

use super::{ScheduleInfo, SchedulerBackend, SchedulerError, SchedulerResult};
use crate::CronSinkConfig;

/// Kubernetes scheduler configuration
#[derive(Debug, Clone)]
pub struct KubernetesConfig {
    /// Namespace for CronJobs
    pub namespace: String,
    /// Image for the sink-trigger container
    pub trigger_image: String,
    /// Image pull policy
    pub image_pull_policy: String,
    /// Service account name
    pub service_account: Option<String>,
    /// ConfigMap name for API base URL
    pub config_map_name: String,
    /// Secret name for sink trigger JWT
    pub secret_name: String,
}

impl KubernetesConfig {
    /// Create configuration from environment variables
    pub fn from_env() -> Result<Self, SchedulerError> {
        Ok(Self {
            namespace: std::env::var("K8S_NAMESPACE").unwrap_or_else(|_| "flow-like".to_string()),
            trigger_image: std::env::var("SINK_TRIGGER_IMAGE")
                .unwrap_or_else(|_| "ghcr.io/rheosoph/flow-like/sink-trigger:latest".to_string()),
            image_pull_policy: std::env::var("IMAGE_PULL_POLICY")
                .unwrap_or_else(|_| "IfNotPresent".to_string()),
            service_account: std::env::var("K8S_SERVICE_ACCOUNT").ok(),
            config_map_name: std::env::var("K8S_CONFIGMAP_NAME")
                .unwrap_or_else(|_| "flow-like-sink-config".to_string()),
            secret_name: std::env::var("K8S_SECRET_NAME")
                .unwrap_or_else(|_| "flow-like-sink-secrets".to_string()),
        })
    }
}

/// Kubernetes CronJob scheduler implementation
#[cfg(feature = "kubernetes")]
pub struct KubernetesScheduler {
    config: KubernetesConfig,
    client: kube::Client,
}

#[cfg(not(feature = "kubernetes"))]
pub struct KubernetesScheduler {
    config: KubernetesConfig,
}

impl KubernetesScheduler {
    /// Generate CronJob name from event ID
    fn cronjob_name(&self, event_id: &str) -> String {
        // Kubernetes names must be lowercase and can't contain certain characters
        format!(
            "flow-like-cron-{}",
            event_id
                .to_lowercase()
                .replace(['/', ':', '_'], "-")
                .chars()
                .take(50) // K8s name limit
                .collect::<String>()
        )
    }
}

#[cfg(feature = "kubernetes")]
impl KubernetesScheduler {
    /// Create a new scheduler from environment variables
    pub async fn from_env() -> Result<Self, SchedulerError> {
        let config = KubernetesConfig::from_env()
            .expect("Failed to load Kubernetes config from environment");
        let client = kube::Client::try_default().await.map_err(|e| {
            SchedulerError::ConfigError(format!("Failed to create K8s client: {}", e))
        })?;
        Ok(Self { config, client })
    }

    /// Create a new scheduler with explicit configuration
    pub async fn new(config: KubernetesConfig) -> Result<Self, SchedulerError> {
        let client = kube::Client::try_default().await.map_err(|e| {
            SchedulerError::ConfigError(format!("Failed to create K8s client: {}", e))
        })?;
        Ok(Self { config, client })
    }

    /// Build a CronJob resource
    fn build_cronjob(
        &self,
        event_id: &str,
        cron_expr: &str,
        config: &CronSinkConfig,
    ) -> k8s_openapi::api::batch::v1::CronJob {
        use k8s_openapi::api::batch::v1::{CronJob, CronJobSpec, JobSpec, JobTemplateSpec};
        use k8s_openapi::api::core::v1::{
            ConfigMapKeySelector, Container, EnvVar, EnvVarSource, PodSpec, PodTemplateSpec,
            ResourceRequirements, SecretKeySelector, SecurityContext,
        };
        use k8s_openapi::apimachinery::pkg::api::resource::Quantity;
        use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
        use std::collections::BTreeMap;

        let name = self.cronjob_name(event_id);

        let mut labels = BTreeMap::new();
        labels.insert(
            "app.kubernetes.io/name".to_string(),
            "flow-like-cron".to_string(),
        );
        labels.insert(
            "app.kubernetes.io/component".to_string(),
            "sink-trigger".to_string(),
        );
        labels.insert("flow-like.io/event-id".to_string(), event_id.to_string());

        let mut requests = BTreeMap::new();
        requests.insert("memory".to_string(), Quantity("16Mi".to_string()));
        requests.insert("cpu".to_string(), Quantity("5m".to_string()));

        let mut limits = BTreeMap::new();
        limits.insert("memory".to_string(), Quantity("32Mi".to_string()));
        limits.insert("cpu".to_string(), Quantity("50m".to_string()));

        // For one-time schedules, convert to a one-shot cron expression (MM HH DD MM *)
        let effective_cron = if let Some(ref scheduled) = config.scheduled_for {
            let time_parts: Vec<&str> = scheduled.time.split(':').collect();
            let date_parts: Vec<&str> = scheduled.date.split('-').collect();
            if time_parts.len() >= 2 && date_parts.len() >= 3 {
                format!(
                    "{} {} {} {} *",
                    time_parts[1], time_parts[0], date_parts[2], date_parts[1]
                )
            } else {
                cron_expr.to_string()
            }
        } else {
            cron_expr.to_string()
        };

        let container = Container {
            name: "trigger".to_string(),
            image: Some(self.config.trigger_image.clone()),
            image_pull_policy: Some(self.config.image_pull_policy.clone()),
            env: Some(vec![
                EnvVar {
                    name: "EVENT_ID".to_string(),
                    value: Some(event_id.to_string()),
                    ..Default::default()
                },
                EnvVar {
                    name: "SINK_TYPE".to_string(),
                    value: Some("cron".to_string()),
                    ..Default::default()
                },
                EnvVar {
                    name: "API_BASE_URL".to_string(),
                    value_from: Some(EnvVarSource {
                        config_map_key_ref: Some(ConfigMapKeySelector {
                            name: self.config.config_map_name.clone(),
                            key: "base-url".to_string(),
                            optional: Some(false),
                        }),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                EnvVar {
                    name: "SINK_TRIGGER_JWT".to_string(),
                    value_from: Some(EnvVarSource {
                        secret_key_ref: Some(SecretKeySelector {
                            name: self.config.secret_name.clone(),
                            key: "trigger-jwt".to_string(),
                            optional: Some(false),
                        }),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            ]),
            resources: Some(ResourceRequirements {
                requests: Some(requests),
                limits: Some(limits),
                ..Default::default()
            }),
            security_context: Some(SecurityContext {
                run_as_non_root: Some(true),
                run_as_user: Some(1000),
                read_only_root_filesystem: Some(true),
                allow_privilege_escalation: Some(false),
                ..Default::default()
            }),
            ..Default::default()
        };

        CronJob {
            metadata: ObjectMeta {
                name: Some(name),
                namespace: Some(self.config.namespace.clone()),
                labels: Some(labels.clone()),
                ..Default::default()
            },
            spec: Some(CronJobSpec {
                schedule: effective_cron,
                time_zone: if config.timezone != "UTC" {
                    Some(config.timezone.clone())
                } else {
                    None
                },
                concurrency_policy: Some("Forbid".to_string()),
                successful_jobs_history_limit: Some(3),
                failed_jobs_history_limit: Some(1),
                suspend: Some(false),
                job_template: JobTemplateSpec {
                    metadata: Some(ObjectMeta {
                        labels: Some(labels.clone()),
                        ..Default::default()
                    }),
                    spec: Some(JobSpec {
                        backoff_limit: Some(2),
                        ttl_seconds_after_finished: Some(86400),
                        template: PodTemplateSpec {
                            metadata: Some(ObjectMeta {
                                labels: Some(labels),
                                ..Default::default()
                            }),
                            spec: Some(PodSpec {
                                service_account_name: self.config.service_account.clone(),
                                restart_policy: Some("Never".to_string()),
                                containers: vec![container],
                                ..Default::default()
                            }),
                        },
                        ..Default::default()
                    }),
                },
                ..Default::default()
            }),
            ..Default::default()
        }
    }
}

#[cfg(not(feature = "kubernetes"))]
impl KubernetesScheduler {
    /// Create a new scheduler (stub without kube-rs)
    pub fn from_env() -> Self {
        let config = KubernetesConfig::from_env()
            .expect("Failed to load Kubernetes config from environment");
        Self { config }
    }

    /// Create a new scheduler with explicit configuration
    pub fn new(config: KubernetesConfig) -> Self {
        Self { config }
    }
}

#[cfg(feature = "kubernetes")]
#[async_trait::async_trait]
impl SchedulerBackend for KubernetesScheduler {
    async fn create_schedule(
        &self,
        event_id: &str,
        cron_expr: &str,
        config: &CronSinkConfig,
    ) -> SchedulerResult<()> {
        use k8s_openapi::api::batch::v1::CronJob;
        use kube::api::{Api, PostParams};

        let cronjob = self.build_cronjob(event_id, cron_expr, config);
        let cronjobs: Api<CronJob> = Api::namespaced(self.client.clone(), &self.config.namespace);

        cronjobs
            .create(&PostParams::default(), &cronjob)
            .await
            .map_err(|e| SchedulerError::ProviderError(format!("K8s API error: {}", e)))?;

        tracing::info!(
            event_id = %event_id,
            cron = %cron_expr,
            namespace = %self.config.namespace,
            "Created Kubernetes CronJob"
        );

        Ok(())
    }

    async fn update_schedule(
        &self,
        event_id: &str,
        cron_expr: &str,
        config: &CronSinkConfig,
    ) -> SchedulerResult<()> {
        use k8s_openapi::api::batch::v1::CronJob;
        use kube::api::{Api, Patch, PatchParams};

        let name = self.cronjob_name(event_id);
        let cronjob = self.build_cronjob(event_id, cron_expr, config);
        let cronjobs: Api<CronJob> = Api::namespaced(self.client.clone(), &self.config.namespace);

        cronjobs
            .patch(
                &name,
                &PatchParams::apply("flow-like"),
                &Patch::Apply(&cronjob),
            )
            .await
            .map_err(|e| SchedulerError::ProviderError(format!("K8s API error: {}", e)))?;

        tracing::info!(
            event_id = %event_id,
            cron = %cron_expr,
            "Updated Kubernetes CronJob"
        );

        Ok(())
    }

    async fn delete_schedule(&self, event_id: &str) -> SchedulerResult<()> {
        use k8s_openapi::api::batch::v1::CronJob;
        use kube::api::{Api, DeleteParams};

        let name = self.cronjob_name(event_id);
        let cronjobs: Api<CronJob> = Api::namespaced(self.client.clone(), &self.config.namespace);

        match cronjobs.delete(&name, &DeleteParams::default()).await {
            Ok(_) => {
                tracing::info!(event_id = %event_id, "Deleted Kubernetes CronJob");
                Ok(())
            }
            Err(kube::Error::Api(e)) if e.code == 404 => {
                tracing::debug!(event_id = %event_id, "CronJob already deleted");
                Ok(())
            }
            Err(e) => Err(SchedulerError::ProviderError(format!(
                "K8s API error: {}",
                e
            ))),
        }
    }

    async fn enable_schedule(&self, event_id: &str) -> SchedulerResult<()> {
        use k8s_openapi::api::batch::v1::CronJob;
        use kube::api::{Api, Patch, PatchParams};

        let name = self.cronjob_name(event_id);
        let cronjobs: Api<CronJob> = Api::namespaced(self.client.clone(), &self.config.namespace);

        let patch = serde_json::json!({
            "spec": {
                "suspend": false
            }
        });

        cronjobs
            .patch(&name, &PatchParams::default(), &Patch::Merge(&patch))
            .await
            .map_err(|e| SchedulerError::ProviderError(format!("K8s API error: {}", e)))?;

        tracing::info!(event_id = %event_id, "Enabled Kubernetes CronJob");

        Ok(())
    }

    async fn disable_schedule(&self, event_id: &str) -> SchedulerResult<()> {
        use k8s_openapi::api::batch::v1::CronJob;
        use kube::api::{Api, Patch, PatchParams};

        let name = self.cronjob_name(event_id);
        let cronjobs: Api<CronJob> = Api::namespaced(self.client.clone(), &self.config.namespace);

        let patch = serde_json::json!({
            "spec": {
                "suspend": true
            }
        });

        cronjobs
            .patch(&name, &PatchParams::default(), &Patch::Merge(&patch))
            .await
            .map_err(|e| SchedulerError::ProviderError(format!("K8s API error: {}", e)))?;

        tracing::info!(event_id = %event_id, "Disabled Kubernetes CronJob");

        Ok(())
    }

    async fn schedule_exists(&self, event_id: &str) -> SchedulerResult<bool> {
        use k8s_openapi::api::batch::v1::CronJob;
        use kube::api::Api;

        let name = self.cronjob_name(event_id);
        let cronjobs: Api<CronJob> = Api::namespaced(self.client.clone(), &self.config.namespace);

        match cronjobs.get(&name).await {
            Ok(_) => Ok(true),
            Err(kube::Error::Api(e)) if e.code == 404 => Ok(false),
            Err(e) => Err(SchedulerError::ProviderError(format!(
                "K8s API error: {}",
                e
            ))),
        }
    }

    async fn get_schedule(&self, event_id: &str) -> SchedulerResult<Option<ScheduleInfo>> {
        use k8s_openapi::api::batch::v1::CronJob;
        use kube::api::Api;

        let name = self.cronjob_name(event_id);
        let cronjobs: Api<CronJob> = Api::namespaced(self.client.clone(), &self.config.namespace);

        match cronjobs.get(&name).await {
            Ok(cj) => {
                let spec = cj.spec.as_ref();
                let cron_expr = spec.map(|s| s.schedule.clone()).unwrap_or_default();
                let enabled = spec.map(|s| !s.suspend.unwrap_or(false)).unwrap_or(true);

                let last_triggered = cj
                    .status
                    .as_ref()
                    .and_then(|s| s.last_schedule_time.as_ref())
                    .map(|t| t.0);

                Ok(Some(ScheduleInfo {
                    event_id: event_id.to_string(),
                    cron_expression: cron_expr,
                    active: enabled,
                    last_triggered,
                    next_trigger: None,
                }))
            }
            Err(kube::Error::Api(e)) if e.code == 404 => Ok(None),
            Err(e) => Err(SchedulerError::ProviderError(format!(
                "K8s API error: {}",
                e
            ))),
        }
    }

    async fn list_schedules(
        &self,
        limit: Option<usize>,
        _offset: Option<usize>,
    ) -> SchedulerResult<Vec<ScheduleInfo>> {
        use k8s_openapi::api::batch::v1::CronJob;
        use kube::api::{Api, ListParams};

        let cronjobs: Api<CronJob> = Api::namespaced(self.client.clone(), &self.config.namespace);

        let lp = ListParams::default().labels("app.kubernetes.io/name=flow-like-cron");

        let list = cronjobs
            .list(&lp)
            .await
            .map_err(|e| SchedulerError::ProviderError(format!("K8s API error: {}", e)))?;

        let mut schedules: Vec<ScheduleInfo> = list
            .items
            .into_iter()
            .filter_map(|cj| {
                let event_id = cj
                    .metadata
                    .labels
                    .as_ref()
                    .and_then(|l| l.get("flow-like.io/event-id"))
                    .cloned()?;

                let spec = cj.spec.as_ref()?;
                let cron_expr = spec.schedule.clone();
                let enabled = !spec.suspend.unwrap_or(false);

                let last_triggered = cj
                    .status
                    .as_ref()
                    .and_then(|s| s.last_schedule_time.as_ref())
                    .map(|t| t.0);

                Some(ScheduleInfo {
                    event_id,
                    cron_expression: cron_expr,
                    active: enabled,
                    last_triggered,
                    next_trigger: None,
                })
            })
            .collect();

        if let Some(l) = limit {
            schedules.truncate(l);
        }

        Ok(schedules)
    }
}

// Stub implementation when kubernetes feature is disabled
#[cfg(not(feature = "kubernetes"))]
#[async_trait::async_trait]
impl SchedulerBackend for KubernetesScheduler {
    async fn create_schedule(
        &self,
        event_id: &str,
        cron_expr: &str,
        _config: &CronSinkConfig,
    ) -> SchedulerResult<()> {
        tracing::warn!(
            event_id = %event_id,
            cron = %cron_expr,
            "Kubernetes feature not enabled - CronJob not created"
        );
        Err(SchedulerError::ConfigError(
            "Kubernetes feature not enabled. Compile with --features kubernetes".into(),
        ))
    }

    async fn update_schedule(
        &self,
        event_id: &str,
        cron_expr: &str,
        _config: &CronSinkConfig,
    ) -> SchedulerResult<()> {
        tracing::warn!(event_id = %event_id, cron = %cron_expr, "Kubernetes feature not enabled");
        Err(SchedulerError::ConfigError(
            "Kubernetes feature not enabled".into(),
        ))
    }

    async fn delete_schedule(&self, event_id: &str) -> SchedulerResult<()> {
        tracing::warn!(event_id = %event_id, "Kubernetes feature not enabled");
        Err(SchedulerError::ConfigError(
            "Kubernetes feature not enabled".into(),
        ))
    }

    async fn enable_schedule(&self, event_id: &str) -> SchedulerResult<()> {
        tracing::warn!(event_id = %event_id, "Kubernetes feature not enabled");
        Err(SchedulerError::ConfigError(
            "Kubernetes feature not enabled".into(),
        ))
    }

    async fn disable_schedule(&self, event_id: &str) -> SchedulerResult<()> {
        tracing::warn!(event_id = %event_id, "Kubernetes feature not enabled");
        Err(SchedulerError::ConfigError(
            "Kubernetes feature not enabled".into(),
        ))
    }

    async fn schedule_exists(&self, _event_id: &str) -> SchedulerResult<bool> {
        Err(SchedulerError::ConfigError(
            "Kubernetes feature not enabled".into(),
        ))
    }

    async fn get_schedule(&self, _event_id: &str) -> SchedulerResult<Option<ScheduleInfo>> {
        Err(SchedulerError::ConfigError(
            "Kubernetes feature not enabled".into(),
        ))
    }

    async fn list_schedules(
        &self,
        _limit: Option<usize>,
        _offset: Option<usize>,
    ) -> SchedulerResult<Vec<ScheduleInfo>> {
        Err(SchedulerError::ConfigError(
            "Kubernetes feature not enabled".into(),
        ))
    }
}
