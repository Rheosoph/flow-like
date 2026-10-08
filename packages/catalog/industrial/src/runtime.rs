use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct Session {
    pub ref_id: String,
    pub protocol: String,
}

#[cfg(feature = "execute")]
pub use execution::*;

#[cfg(feature = "execute")]
mod execution {
    use super::Session;
    use flow_like::flow::{
        execution::{context::ExecutionContext, internal_node::InternalNode},
        pin::PinType,
        variable::VariableType,
    };
    use flow_like_types::{Cacheable, Result, Value, anyhow};
    use std::{any::Any, sync::Arc};
    use tokio_util::sync::CancellationToken;

    struct CachedSession {
        protocol: String,
        value: Arc<dyn Any + Send + Sync>,
        cancellation: CancellationToken,
    }

    impl Cacheable for CachedSession {
        fn as_any(&self) -> &dyn Any {
            self
        }
        fn as_any_mut(&mut self) -> &mut dyn Any {
            self
        }
    }

    impl Drop for CachedSession {
        fn drop(&mut self) {
            self.cancellation.cancel();
        }
    }

    pub async fn store<T: Send + Sync + 'static>(
        context: &ExecutionContext,
        protocol: &str,
        value: T,
    ) -> Session {
        let session = Session {
            ref_id: format!("industrial:{}", uuid::Uuid::new_v4()),
            protocol: protocol.into(),
        };
        let cancellation = context
            .get_cancellation_token()
            .map(|token| token.child_token())
            .unwrap_or_default();
        let cleanup_token = cancellation.clone();
        let cache = Arc::downgrade(&context.cache);
        let key = session.ref_id.clone();
        context
            .set_cache(
                &session.ref_id,
                Arc::new(CachedSession {
                    protocol: protocol.into(),
                    value: Arc::new(value),
                    cancellation,
                }),
            )
            .await;
        tokio::spawn(async move {
            cleanup_token.cancelled().await;
            if let Some(cache) = cache.upgrade() {
                cache.write().await.remove(&key);
            }
        });
        session
    }

    pub async fn get<T: Send + Sync + 'static>(
        context: &ExecutionContext,
        session: &Session,
        protocol: &str,
    ) -> Result<Arc<T>> {
        if session.protocol != protocol {
            return Err(anyhow!("Expected a {protocol} session"));
        }
        let cached = context
            .get_cache(&session.ref_id)
            .await
            .ok_or_else(|| anyhow!("Connection is closed or belongs to another workflow run"))?;
        let cached = cached
            .as_any()
            .downcast_ref::<CachedSession>()
            .ok_or_else(|| anyhow!("Invalid industrial connection"))?;
        if cached.protocol != protocol || cached.cancellation.is_cancelled() {
            return Err(anyhow!("Connection is closed or has a different protocol"));
        }
        cached
            .value
            .clone()
            .downcast::<T>()
            .map_err(|_| anyhow!("Connection type does not match the operation"))
    }

    pub async fn cancellation(
        context: &ExecutionContext,
        session: &Session,
    ) -> Result<CancellationToken> {
        let cached = context
            .get_cache(&session.ref_id)
            .await
            .ok_or_else(|| anyhow!("Connection is closed"))?;
        let cached = cached
            .as_any()
            .downcast_ref::<CachedSession>()
            .ok_or_else(|| anyhow!("Invalid industrial connection"))?;
        if cached.protocol != session.protocol {
            return Err(anyhow!("Connection protocol mismatch"));
        }
        Ok(cached.cancellation.clone())
    }

    pub async fn remove(context: &ExecutionContext, session: &Session) -> Result<()> {
        let mut cache = context.cache.write().await;
        if let Some(cached) = cache.get(&session.ref_id) {
            let cached = cached
                .as_any()
                .downcast_ref::<CachedSession>()
                .ok_or_else(|| anyhow!("Invalid industrial connection"))?;
            if cached.protocol != session.protocol {
                return Err(anyhow!("Connection protocol mismatch"));
            }
            cached.cancellation.cancel();
            cache.remove(&session.ref_id);
        }
        Ok(())
    }

    pub async fn wait_for_cancel(token: Option<CancellationToken>) {
        if let Some(token) = token {
            token.cancelled().await;
        } else {
            std::future::pending::<()>().await;
        }
    }

    pub struct Handler {
        context: ExecutionContext,
        parent_id: String,
    }

    impl Handler {
        pub async fn new(context: &ExecutionContext) -> Result<Self> {
            let handlers = context.get_referenced_functions().await?;
            if handlers.len() != 1 {
                return Err(anyhow!("Reference exactly one event handler function"));
            }
            let mut child = context.create_sub_context(&handlers[0]).await;
            child.delegated = true;
            child.context_pin_overrides = Some(Default::default());
            let has_output = child.node.pins.iter().any(|pin| {
                pin.pin_type == PinType::Output && pin.data_type != VariableType::Execution
            });
            if !has_output {
                return Err(anyhow!(
                    "The handler needs an event output or outputs named after event fields"
                ));
            }
            Ok(Self {
                context: child,
                parent_id: context.node.node_id().to_string(),
            })
        }

        /// Wait for the whole handler flow before reporting successful processing to the broker.
        pub async fn dispatch(&mut self, event: Value) -> Result<()> {
            // Each delivery needs fresh dependency state, local results, and trace bookkeeping.
            let mut delivery = self.context.create_sub_context(&self.context.node).await;
            delivery.delegated = true;
            delivery.context_pin_overrides = Some(Default::default());
            let context = &mut delivery;
            if context.is_cancelled() {
                return Err(anyhow!("Industrial event handler was cancelled"));
            }
            let pins: Vec<_> = context
                .node
                .pins
                .iter()
                .filter(|pin| {
                    pin.pin_type == PinType::Output && pin.data_type != VariableType::Execution
                })
                .map(|pin| (*pin).clone())
                .collect();
            let mut matched = false;
            for pin in pins {
                let value = if pin.name.as_ref() == "event" {
                    Some(event.clone())
                } else {
                    event.get(pin.name.as_ref() as &str).cloned()
                };
                if let Some(value) = value {
                    context.set_pin_ref_value(&pin, value).await?;
                    matched = true;
                } else {
                    // A previous delivery must never supply a missing field in a later one.
                    context.override_pin_value_shared(pin.id(), Arc::new(Value::Null));
                    pin.set_value(Value::Null).await;
                }
            }
            if !matched {
                return Err(anyhow!(
                    "The handler outputs do not match the incoming event"
                ));
            }
            let mut guard = ahash::AHashSet::new();
            guard.insert(self.parent_id.clone());
            let mut guard = Some(guard);
            let cancellation = context.get_cancellation_token();
            let result = tokio::select! {
                _ = wait_for_cancel(cancellation) => Err(anyhow!("Industrial event handler was cancelled")),
                result = InternalNode::trigger(context, &mut guard, true) => result.map_err(|error| anyhow!("Industrial event handler failed: {error:?}")),
            };
            context.end_trace();
            let logs = if context.try_get_run().is_ok() {
                context.flush_logs().await
            } else {
                Ok(())
            };
            result?;
            if context.is_cancelled() {
                return Err(anyhow!("Industrial event handler was cancelled"));
            }
            logs?;
            Ok(())
        }
    }

    pub fn local_storage_directory(
        context: &ExecutionContext,
        suffix: &str,
    ) -> Result<std::path::PathBuf> {
        use flow_like_storage::{Path, files::store::FlowLikeStore};
        if suffix.is_empty() || suffix.contains(['/', '\\']) || suffix == "." || suffix == ".." {
            return Err(anyhow!("Invalid industrial storage directory"));
        }
        let cache = context
            .execution_cache
            .as_ref()
            .ok_or_else(|| anyhow!("Execution storage context is unavailable"))?;
        if cache.shadow {
            return Err(anyhow!("Industrial connections require a live workflow"));
        }
        let Some(FlowLikeStore::Local(local)) = &cache.stores.app_storage_store else {
            return Err(anyhow!(
                "Certificate storage requires local app storage on this executor"
            ));
        };
        let root = local
            .directory_to_filesystem(&Path::from(""))?
            .canonicalize()?;
        // Retain the existing certificate location when moving the inspection nodes.
        let path = local
            .directory_to_filesystem(&cache.get_storage(false)?.join("inspection").join(suffix))?;
        let mut ancestor = path.as_path();
        while !ancestor.try_exists()? {
            if std::fs::symlink_metadata(ancestor).is_ok() {
                return Err(anyhow!("Certificate path contains a dangling symlink"));
            }
            ancestor = ancestor
                .parent()
                .ok_or_else(|| anyhow!("Certificate path has no parent"))?;
        }
        if !ancestor.canonicalize()?.starts_with(root) {
            return Err(anyhow!("Certificate path escapes app storage"));
        }
        Ok(path)
    }
}
