#![cfg(feature = "local-tts")]

pub mod local;

pub use local::{LocalTtsModel, LocalTtsModelInfo, LocalTtsSynthesisRequest, TtsSynthesisOutput};
