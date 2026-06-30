#![cfg(feature = "local-stt")]

pub mod local;

pub use local::{
    LocalSttModel, LocalSttModelInfo, LocalTranscriptionRequest, TranscriptionOutput,
    TranscriptionSegmentOutput,
};
