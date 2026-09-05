use crate::bit::Bit;

pub mod embedding;
pub mod embedding_factory;
pub mod generation;
pub mod image_embedding;
pub mod llm;
pub mod local_utils;
#[cfg(feature = "local-stt")]
pub mod stt;
#[cfg(feature = "local-tts")]
pub mod tts;

pub trait ModelMeta: Send + Sync {
    fn get_bit(&self) -> Bit;
}
