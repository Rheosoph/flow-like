//! Scheduler backends for managing cron and scheduled event triggers
//!
//! This module provides a unified abstraction over different scheduling systems:
//! - AWS EventBridge Scheduler
//! - Kubernetes CronJobs
//! - In-memory scheduler (for Docker Compose / local development)

mod cron_translator;
mod traits;

pub mod aws;
pub mod kubernetes;
pub mod memory;

pub use cron_translator::standard_to_aws_cron;

pub use aws::{AwsEventBridgeConfig, AwsEventBridgeScheduler};
pub use kubernetes::{KubernetesConfig, KubernetesScheduler};
pub use memory::InMemoryScheduler;
pub use traits::{ScheduleInfo, SchedulerBackend, SchedulerError, SchedulerResult};

/// Scheduler provider type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchedulerProvider {
    /// AWS EventBridge Scheduler
    Aws,
    /// Kubernetes CronJobs
    Kubernetes,
    /// In-memory scheduler (for Docker Compose / local)
    Memory,
}

impl SchedulerProvider {
    /// Parse from string
    pub fn from_env_value(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "aws" | "eventbridge" => Self::Aws,
            "kubernetes" | "k8s" => Self::Kubernetes,
            _ => Self::Memory,
        }
    }
}

/// Create a scheduler backend based on the provider configuration
///
/// For AWS and Kubernetes providers with their features enabled,
/// this returns an in-memory scheduler as a fallback since they
/// require async initialization. Use the specific constructors instead:
/// - `AwsEventBridgeScheduler::from_env().await`
/// - `KubernetesScheduler::from_env().await`
pub fn create_scheduler(provider: SchedulerProvider) -> Box<dyn SchedulerBackend> {
    match provider {
        SchedulerProvider::Memory => Box::new(memory::InMemoryScheduler::new()),
        // AWS and K8s require async init, return memory as sync fallback
        _ => {
            tracing::warn!(
                provider = ?provider,
                "Scheduler requires async initialization, falling back to in-memory"
            );
            Box::new(memory::InMemoryScheduler::new())
        }
    }
}

/// Create a scheduler backend asynchronously
#[cfg(feature = "aws")]
pub async fn create_scheduler_async(
    provider: SchedulerProvider,
) -> Result<Box<dyn SchedulerBackend>, SchedulerError> {
    match provider {
        SchedulerProvider::Aws => Ok(Box::new(aws::AwsEventBridgeScheduler::from_env().await)),
        SchedulerProvider::Kubernetes => {
            #[cfg(feature = "kubernetes")]
            {
                Ok(Box::new(kubernetes::KubernetesScheduler::from_env().await?))
            }
            #[cfg(not(feature = "kubernetes"))]
            {
                tracing::warn!("Kubernetes feature not enabled, using in-memory scheduler");
                Ok(Box::new(memory::InMemoryScheduler::new()))
            }
        }
        SchedulerProvider::Memory => Ok(Box::new(memory::InMemoryScheduler::new())),
    }
}

#[cfg(all(feature = "kubernetes", not(feature = "aws")))]
pub async fn create_scheduler_async(
    provider: SchedulerProvider,
) -> Result<Box<dyn SchedulerBackend>, SchedulerError> {
    match provider {
        SchedulerProvider::Kubernetes => {
            Ok(Box::new(kubernetes::KubernetesScheduler::from_env().await?))
        }
        SchedulerProvider::Aws => {
            tracing::warn!("AWS feature not enabled, using in-memory scheduler");
            Ok(Box::new(memory::InMemoryScheduler::new()))
        }
        SchedulerProvider::Memory => Ok(Box::new(memory::InMemoryScheduler::new())),
    }
}

#[cfg(not(any(feature = "aws", feature = "kubernetes")))]
pub async fn create_scheduler_async(
    provider: SchedulerProvider,
) -> Result<Box<dyn SchedulerBackend>, SchedulerError> {
    if provider != SchedulerProvider::Memory {
        tracing::warn!(
            provider = ?provider,
            "Feature not enabled, using in-memory scheduler"
        );
    }
    Ok(Box::new(memory::InMemoryScheduler::new()))
}
