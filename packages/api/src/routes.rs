use serde::{Deserialize, Serialize};

pub mod admin;
pub mod ai;
pub mod alias;
pub mod app;
pub mod audit;
pub mod auth;
pub mod bit;
pub mod channel;
pub mod chat;
pub mod course;
pub mod embeddings;
pub mod execution;
pub mod flowscript;
pub mod health;
pub mod inbound;
pub mod info;
pub mod maintenance;
pub mod oauth;
pub mod og;
pub mod profile;
pub mod registry;
pub mod sink;
pub mod solution;
pub mod store;
pub mod telemetry;
pub mod tmp;
pub mod usage;
pub mod user;
pub mod webhook;

#[derive(Clone, Deserialize, Serialize, Debug)]
pub struct LanguageParams {
    pub language: Option<String>,
    pub limit: Option<u64>,
    pub offset: Option<u64>,
}

#[derive(Clone, Deserialize, Serialize, Debug)]
pub struct PaginationParams {
    pub limit: Option<u64>,
    pub offset: Option<u64>,
}
