//! DEPRECATED: Use `crate::session::CreateSessionNode` instead.
//!
//! This module is deprecated and will be removed in a future version.
//! The unified `AutomationSession` type from `crate::types::handles` should be used
//! instead of the old `RpaSessionHandle`.
//!
//! Migration guide:
//! - Replace `RpaSessionHandle` with `AutomationSession`
//! - Use `CreateSessionNode` from `crate::session` instead of `CreateRpaSessionNode`
//! - The `AutomationSession` provides both RPA and browser automation capabilities
