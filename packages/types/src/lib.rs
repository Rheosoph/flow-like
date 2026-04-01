pub mod proto {
    include!(concat!(env!("OUT_DIR"), "/flow_like_types.rs"));
}

pub trait ToProto<T> {
    fn to_proto(&self) -> T;
}

pub trait FromProto<T> {
    fn from_proto(proto: T) -> Self;
}

pub trait Cacheable: Any + Send + Sync {
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

impl dyn Cacheable {
    pub fn downcast_ref<T: Cacheable>(&self) -> Option<&T> {
        self.as_any().downcast_ref::<T>()
    }

    pub fn downcast_mut<T: Cacheable>(&mut self) -> Option<&mut T> {
        self.as_any_mut().downcast_mut::<T>()
    }
}

pub type Timestamp = prost_types::Timestamp;

use std::any::Any;

pub use anyhow::{Context, Error, Ok, Result, anyhow, bail};
pub use async_trait::async_trait;
pub use base64;
pub use cuid2::create_id;
pub use mime_guess;
pub use prost::Message;
pub use reqwest;
pub use reqwest_eventsource;
pub use schemars::JsonSchema;
pub use serde;
pub use serde_json::Value;
pub use tokio_util;
pub mod images;
pub mod json {
    pub use serde::{Deserialize, Serialize, de::DeserializeOwned};
    pub use serde_json::{
        Map, Number, from_reader, from_slice, from_str, from_value, json, to_string,
        to_string_pretty, to_value, to_vec, to_vec_pretty,
    };
}

pub mod dispatch;

pub use bytes::Bytes;
pub use tokio;
pub mod sync {
    pub use dashmap::DashMap;
    pub use tokio::sync::Mutex;
    pub use tokio::sync::RwLock;
    pub use tokio::sync::mpsc;
}

pub use rand;
pub mod futures {
    pub use futures::StreamExt;
}
pub use async_stream;
pub mod interaction;
pub mod intercom;
pub mod utils;

/// OAuth token input for execution requests
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct OAuthTokenInput {
    pub access_token: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<i64>,
}

pub use ab_glyph;
pub use image;
pub use imageproc;
pub use jsonschema;
pub use minijinja;
pub use regex;
pub use rxing;
