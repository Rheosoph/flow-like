use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
};

use axum::{Json, http::HeaderValue};

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReportPolicy {
    Ignore,
    Report,
}

#[derive(Debug, Clone)]
pub struct ErrorReport {
    pub id: String,
    pub status_code: u16,
    pub public_code: String,
    pub summary: String,
    pub details: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ApiError {
    status: StatusCode,
    public_code: String,
    public_message: Option<String>,
    report_policy: ReportPolicy,
    report_summary: Option<String>,
    report_details: Option<String>,
}

// Associated constants for enum-like usage without parentheses
impl ApiError {
    pub const NOT_FOUND: ApiError = ApiError {
        status: StatusCode::NOT_FOUND,
        public_code: String::new(),
        public_message: None,
        report_policy: ReportPolicy::Ignore,
        report_summary: None,
        report_details: None,
    };

    pub const FORBIDDEN: ApiError = ApiError {
        status: StatusCode::FORBIDDEN,
        public_code: String::new(),
        public_message: None,
        report_policy: ReportPolicy::Ignore,
        report_summary: None,
        report_details: None,
    };

    pub const UNAUTHORIZED: ApiError = ApiError {
        status: StatusCode::UNAUTHORIZED,
        public_code: String::new(),
        public_message: None,
        report_policy: ReportPolicy::Ignore,
        report_summary: None,
        report_details: None,
    };

    pub fn internal_error(err: flow_like_types::Error) -> Self {
        Self::internal(err.to_string())
    }

    /// The client-safe message, for callers that need to record the same
    /// reason they are about to return.
    pub fn public_message(&self) -> Option<&str> {
        self.public_message.as_deref()
    }

    pub fn status(&self) -> StatusCode {
        self.status
    }
}

impl ApiError {
    fn new(
        status: StatusCode,
        public_code: impl Into<String>,
        public_message: Option<String>,
        report_policy: ReportPolicy,
    ) -> Self {
        Self {
            status,
            public_code: public_code.into(),
            public_message,
            report_policy,
            report_summary: None,
            report_details: None,
        }
    }

    fn with_report(mut self, summary: impl Into<String>, details: Option<String>) -> Self {
        self.report_summary = Some(summary.into());
        self.report_details = details;
        self
    }

    pub fn internal(msg: impl Into<String>) -> Self {
        let msg = msg.into();
        tracing::error!("Internal error: {}", msg);
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "INTERNAL_ERROR",
            None,
            ReportPolicy::Report,
        )
        .with_report(msg, None)
    }

    pub fn not_found(msg: impl Into<String>) -> Self {
        let msg = msg.into();
        tracing::warn!("Not found: {}", msg);
        Self::new(
            StatusCode::NOT_FOUND,
            "NOT_FOUND",
            Some(msg),
            ReportPolicy::Ignore,
        )
    }

    pub fn bad_request(msg: impl Into<String>) -> Self {
        let msg = msg.into();
        tracing::warn!("Bad request: {}", msg);
        Self::new(
            StatusCode::BAD_REQUEST,
            "BAD_REQUEST",
            Some(msg),
            ReportPolicy::Ignore,
        )
    }

    pub fn unauthorized(msg: impl Into<String>) -> Self {
        let msg = msg.into();
        tracing::warn!("Unauthorized: {}", msg);
        Self::new(
            StatusCode::UNAUTHORIZED,
            "UNAUTHORIZED",
            Some(msg),
            ReportPolicy::Ignore,
        )
    }

    pub fn forbidden(msg: impl Into<String>) -> Self {
        let msg = msg.into();
        tracing::warn!("Forbidden: {}", msg);
        Self::new(
            StatusCode::FORBIDDEN,
            "FORBIDDEN",
            Some(msg),
            ReportPolicy::Ignore,
        )
    }

    pub fn conflict(msg: impl Into<String>) -> Self {
        let msg = msg.into();
        tracing::warn!("Conflict: {}", msg);
        Self::new(
            StatusCode::CONFLICT,
            "CONFLICT",
            Some(msg),
            ReportPolicy::Ignore,
        )
    }

    pub fn payment_required(msg: impl Into<String>) -> Self {
        let msg = msg.into();
        tracing::warn!("Payment required: {}", msg);
        Self::new(
            StatusCode::PAYMENT_REQUIRED,
            "PAYMENT_REQUIRED",
            Some(msg),
            ReportPolicy::Ignore,
        )
    }

    pub fn gone(msg: impl Into<String>) -> Self {
        let msg = msg.into();
        tracing::warn!("Gone: {}", msg);
        Self::new(StatusCode::GONE, "GONE", Some(msg), ReportPolicy::Ignore)
    }

    pub fn unprocessable(msg: impl Into<String>) -> Self {
        let msg = msg.into();
        tracing::warn!("Unprocessable entity: {}", msg);
        Self::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "UNPROCESSABLE_ENTITY",
            Some(msg),
            ReportPolicy::Ignore,
        )
    }

    pub fn too_many_requests(msg: impl Into<String>) -> Self {
        let msg = msg.into();
        tracing::warn!("Too many requests: {}", msg);
        Self::new(
            StatusCode::TOO_MANY_REQUESTS,
            "TOO_MANY_REQUESTS",
            Some(msg),
            ReportPolicy::Ignore,
        )
    }

    pub fn service_unavailable(msg: impl Into<String>) -> Self {
        let msg = msg.into();
        tracing::error!("Service unavailable: {}", msg);
        Self::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "SERVICE_UNAVAILABLE",
            Some("Service unavailable".to_string()),
            ReportPolicy::Report,
        )
        .with_report(msg, None)
    }

    pub fn bad_gateway(msg: impl Into<String>) -> Self {
        let msg = msg.into();
        tracing::warn!("Bad gateway: {}", msg);
        Self::new(
            StatusCode::BAD_GATEWAY,
            "BAD_GATEWAY",
            Some(msg),
            ReportPolicy::Ignore,
        )
    }

    pub fn gateway_timeout(msg: impl Into<String>) -> Self {
        let msg = msg.into();
        tracing::warn!("Gateway timeout: {}", msg);
        Self::new(
            StatusCode::GATEWAY_TIMEOUT,
            "GATEWAY_TIMEOUT",
            Some(msg),
            ReportPolicy::Ignore,
        )
    }

    pub fn not_implemented(msg: impl Into<String>) -> Self {
        let msg = msg.into();
        tracing::warn!("Not implemented: {}", msg);
        Self::new(
            StatusCode::NOT_IMPLEMENTED,
            "NOT_IMPLEMENTED",
            Some(msg),
            ReportPolicy::Ignore,
        )
    }

    // Legacy constructor removed; use `bad_request`/`internal_error`.
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        #[derive(Serialize)]
        struct ErrorEnvelope<'a> {
            error: ErrorBody<'a>,
        }

        #[derive(Serialize)]
        struct ErrorBody<'a> {
            code: &'a str,
            #[serde(skip_serializing_if = "Option::is_none")]
            id: Option<&'a str>,
            message: &'a str,
        }

        let code = if self.public_code.is_empty() {
            match self.status {
                StatusCode::NOT_FOUND => "NOT_FOUND",
                StatusCode::FORBIDDEN => "FORBIDDEN",
                StatusCode::UNAUTHORIZED => "UNAUTHORIZED",
                StatusCode::BAD_REQUEST => "BAD_REQUEST",
                _ => "ERROR",
            }
        } else {
            self.public_code.as_str()
        };

        let public_message = self
            .public_message
            .as_deref()
            .unwrap_or_else(|| self.status.canonical_reason().unwrap_or("Error"));

        let mut error_id: Option<String> = None;
        if self.report_policy == ReportPolicy::Report {
            error_id = Some(flow_like_types::create_id());
        }

        let mut response = (
            self.status,
            Json(ErrorEnvelope {
                error: ErrorBody {
                    code,
                    id: error_id.as_deref(),
                    message: public_message,
                },
            }),
        )
            .into_response();

        if let Some(id) = error_id.as_deref() {
            if let Ok(v) = HeaderValue::from_str(id) {
                response.headers_mut().insert("x-error-id", v);
            }

            let report = ErrorReport {
                id: id.to_string(),
                status_code: self.status.as_u16(),
                public_code: code.to_string(),
                summary: self
                    .report_summary
                    .clone()
                    .unwrap_or_else(|| public_message.to_string()),
                details: self.report_details.clone(),
            };
            response.extensions_mut().insert(report);
        }

        response
    }
}

// Implement From for flow_like_types::Error
impl From<flow_like_types::Error> for ApiError {
    fn from(err: flow_like_types::Error) -> Self {
        tracing::error!("Internal error: {:?}", err);
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "INTERNAL_ERROR",
            None,
            ReportPolicy::Report,
        )
        .with_report(format!("{:?}", err), Some(err.to_string()))
    }
}

// Implement From for sea_orm::DbErr
impl From<sea_orm::DbErr> for ApiError {
    fn from(err: sea_orm::DbErr) -> Self {
        tracing::error!("Database error: {:?}", err);
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "DATABASE_ERROR",
            None,
            ReportPolicy::Report,
        )
        .with_report(format!("{:?}", err), Some(err.to_string()))
    }
}

// Implement From for common error types
impl From<std::io::Error> for ApiError {
    fn from(err: std::io::Error) -> Self {
        tracing::error!("IO error: {:?}", err);
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "IO_ERROR",
            None,
            ReportPolicy::Report,
        )
        .with_report(format!("{:?}", err), Some(err.to_string()))
    }
}

impl From<serde_json::Error> for ApiError {
    fn from(err: serde_json::Error) -> Self {
        tracing::error!("JSON error: {:?}", err);
        // Parsing errors are typically user-caused. Keep message, do not persist.
        Self::bad_request(format!("JSON error: {}", err))
    }
}

impl From<std::num::ParseIntError> for ApiError {
    fn from(err: std::num::ParseIntError) -> Self {
        tracing::warn!("Parse error: {:?}", err);
        Self::bad_request(format!("Invalid number format: {}", err))
    }
}

impl From<flow_like_storage::object_store::Error> for ApiError {
    fn from(err: flow_like_storage::object_store::Error) -> Self {
        tracing::error!("Object store error: {:?}", err);
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "STORAGE_ERROR",
            None,
            ReportPolicy::Report,
        )
        .with_report(format!("{:?}", err), Some(err.to_string()))
    }
}

impl From<jsonwebtoken::errors::Error> for ApiError {
    fn from(err: jsonwebtoken::errors::Error) -> Self {
        tracing::warn!("JWT error: {:?}", err);
        Self::unauthorized(format!("JWT error: {}", err))
    }
}

impl From<flow_like_storage::lancedb::Error> for ApiError {
    fn from(err: flow_like_storage::lancedb::Error) -> Self {
        tracing::error!("LanceDB error: {:?}", err);
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "LANCEDB_ERROR",
            None,
            ReportPolicy::Report,
        )
        .with_report(format!("{:?}", err), Some(err.to_string()))
    }
}

impl From<sea_orm::TransactionError<ApiError>> for ApiError {
    fn from(err: sea_orm::TransactionError<ApiError>) -> Self {
        match err {
            sea_orm::TransactionError::Connection(db_err) => db_err.into(),
            sea_orm::TransactionError::Transaction(api_err) => api_err,
        }
    }
}

impl From<stripe::StripeError> for ApiError {
    fn from(err: stripe::StripeError) -> Self {
        tracing::error!("Stripe error: {:?}", err);
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "STRIPE_ERROR",
            None,
            ReportPolicy::Report,
        )
        .with_report(format!("{:?}", err), Some(err.to_string()))
    }
}

impl From<flow_like_storage::datafusion::error::DataFusionError> for ApiError {
    fn from(err: flow_like_storage::datafusion::error::DataFusionError) -> Self {
        tracing::error!("DataFusion error: {:?}", err);
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "DATAFUSION_ERROR",
            None,
            ReportPolicy::Report,
        )
        .with_report(format!("{:?}", err), Some(err.to_string()))
    }
}

impl std::error::Error for ApiError {}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.status, self.public_code.as_str())
    }
}

// Convenience macros for quick error creation
#[macro_export]
macro_rules! internal {
    ($($arg:tt)*) => { $crate::error::ApiError::internal(format!($($arg)*)) };
}

#[macro_export]
macro_rules! not_found {
    ($($arg:tt)*) => { $crate::error::ApiError::not_found(format!($($arg)*)) };
}

#[macro_export]
macro_rules! bad_request {
    ($($arg:tt)*) => { $crate::error::ApiError::bad_request(format!($($arg)*)) };
}

#[macro_export]
macro_rules! unauthorized {
    ($($arg:tt)*) => { $crate::error::ApiError::unauthorized(format!($($arg)*)) };
}

#[macro_export]
macro_rules! forbidden {
    ($($arg:tt)*) => { $crate::error::ApiError::forbidden(format!($($arg)*)) };
}

// Legacy type alias for backward compatibility during migration
pub type InternalError = ApiError;
pub type AuthorizationError = ApiError;
