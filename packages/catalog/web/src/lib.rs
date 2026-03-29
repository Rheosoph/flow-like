//! Web catalog for Flow-Like
//!
//! This crate contains web-related nodes:
//! - HTTP requests
//! - Web scraping
//! - Mail (IMAP/SMTP)
//! - Discord bot integration
//! - Telegram bot integration

use std::sync::Arc;

pub use flow_like_catalog_core::{NodeConstructor, NodeLogic, inventory, register_node};

pub mod discord;
pub mod http;
pub mod mail;
pub mod telegram;
pub mod web;

pub fn get_catalog() -> Vec<Arc<dyn NodeLogic>> {
    flow_like_catalog_core::get_catalog()
}
