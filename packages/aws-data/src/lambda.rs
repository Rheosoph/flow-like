//! Per-invocation token upkeep for Lambda-hosted services.
//!
//! A Lambda's timers stop while the instance is frozen, so the DSQL token is
//! refreshed at the start of each request instead: a timestamp compare on the
//! hot path, a local SigV4 presign every ~12 minutes.

use crate::dsql::DsqlDatabase;
use std::{
    future::Future,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};
use tower::{Layer, Service};

#[derive(Clone)]
pub struct TokenRefreshLayer {
    database: Arc<DsqlDatabase>,
}

impl TokenRefreshLayer {
    pub fn new(database: Arc<DsqlDatabase>) -> Self {
        Self { database }
    }
}

impl<S> Layer<S> for TokenRefreshLayer {
    type Service = TokenRefresh<S>;

    fn layer(&self, inner: S) -> Self::Service {
        TokenRefresh {
            inner,
            database: self.database.clone(),
        }
    }
}

#[derive(Clone)]
pub struct TokenRefresh<S> {
    inner: S,
    database: Arc<DsqlDatabase>,
}

impl<S, Request> Service<Request> for TokenRefresh<S>
where
    S: Service<Request> + Clone + Send + 'static,
    S::Future: Send + 'static,
    Request: Send + 'static,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = Pin<Box<dyn Future<Output = Result<S::Response, S::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, request: Request) -> Self::Future {
        // The ready service is the one that was polled; the clone waits for
        // the next `poll_ready`.
        let ready = self.inner.clone();
        let mut inner = std::mem::replace(&mut self.inner, ready);
        let database = self.database.clone();
        Box::pin(async move {
            // A failed mint is not fatal here: the pool may still hold live
            // connections, and a truly dead token surfaces as a connection
            // error on the request itself.
            if let Err(error) = database.refresh_token_if_stale().await {
                tracing::warn!(%error, "Aurora DSQL token refresh failed before request");
            }
            inner.call(request).await
        })
    }
}
