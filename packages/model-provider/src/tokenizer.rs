use std::sync::Arc;

#[cfg(feature = "local-ml")]
use fastembed::TokenizerFiles;
use text_splitter::ChunkSizer;
use tokenizers::{Encoding, Tokenizer};
#[cfg(feature = "local-ml")]
use tokenizers::{AddedToken, PaddingParams, PaddingStrategy, TruncationParams};

#[derive(Clone)]
pub struct TokenizerSizer(Arc<Tokenizer>);

impl TokenizerSizer {
    pub fn new(tokenizer: Tokenizer) -> Self {
        Self(Arc::new(tokenizer))
    }
}

fn num_tokens_with_overflow(encoding: &Encoding, pad_id: Option<u32>) -> usize {
    let base = encoding
        .get_ids()
        .iter()
        .skip_while(|&id| pad_id.is_some_and(|pad_id| id == &pad_id))
        .take_while(|&id| pad_id.is_none_or(|pad_id| id != &pad_id))
        .count();

    let overflow: usize = encoding
        .get_overflowing()
        .iter()
        .map(|enc| num_tokens_with_overflow(enc, pad_id))
        .sum();

    base + overflow
}

impl ChunkSizer for TokenizerSizer {
    fn size(&self, chunk: &str) -> usize {
        let encoding = self
            .0
            .encode_fast(chunk, false)
            .expect("Unable to tokenize the following string {chunk}");

        let pad_id = self.0.get_padding().map(|params| params.pad_id);
        num_tokens_with_overflow(&encoding, pad_id)
    }
}

impl From<Tokenizer> for TokenizerSizer {
    fn from(tokenizer: Tokenizer) -> Self {
        Self::new(tokenizer)
    }
}

#[cfg(feature = "local-ml")]
pub fn load_tokenizer_from_file(
    tokenizer_files: Arc<TokenizerFiles>,
    max_length: usize,
) -> flow_like_types::Result<TokenizerSizer> {
    let base_error_message =
        "Error building TokenizerFiles for UserDefinedEmbeddingModel. Could not read {} file.";

    // Serialize each tokenizer file
    let config: flow_like_types::Value =
        flow_like_types::json::from_slice(&tokenizer_files.config_file).map_err(|_| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                base_error_message.replace("{}", "config.json"),
            )
        })?;
    let special_tokens_map: flow_like_types::Value = flow_like_types::json::from_slice(
        &tokenizer_files.special_tokens_map_file,
    )
    .map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            base_error_message.replace("{}", "special_tokens_map.json"),
        )
    })?;
    let tokenizer_config: flow_like_types::Value = flow_like_types::json::from_slice(
        &tokenizer_files.tokenizer_config_file,
    )
    .map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            base_error_message.replace("{}", "tokenizer_config.json"),
        )
    })?;
    let mut tokenizer: tokenizers::Tokenizer = tokenizers::Tokenizer::from_bytes(
        tokenizer_files.tokenizer_file.clone(),
    )
    .map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            base_error_message.replace("{}", "tokenizer.json"),
        )
    })?;

    //For BGEBaseSmall, the model_max_length value is set to 1000000000000000019884624838656. Which fits in a f64
    let model_max_length = tokenizer_config["model_max_length"]
        .as_f64()
        .expect("Error reading model_max_length from tokenizer_config.json")
        as f32;
    let max_length = max_length.min(model_max_length as usize);
    let pad_id = config["pad_token_id"].as_u64().unwrap_or(0) as u32;
    let pad_token = tokenizer_config["pad_token"]
        .as_str()
        .expect("Error reading pad_token from tokenier_config.json")
        .into();

    tokenizer.with_padding(Some(PaddingParams {
        // TODO: the user should able to choose the padding strategy
        strategy: PaddingStrategy::BatchLongest,
        pad_token,
        pad_id,
        ..Default::default()
    }));
    tokenizer
        .with_truncation(Some(TruncationParams {
            max_length,
            ..Default::default()
        }))
        .map_err(flow_like_types::Error::msg)?;
    if let flow_like_types::Value::Object(root_object) = special_tokens_map {
        for (_, value) in root_object.iter() {
            if value.is_string() {
                tokenizer.add_special_tokens(&[AddedToken {
                    content: value.as_str().unwrap().into(),
                    special: true,
                    ..Default::default()
                }]);
            } else if value.is_object() {
                tokenizer.add_special_tokens(&[AddedToken {
                    content: value["content"].as_str().unwrap().into(),
                    special: true,
                    single_word: value["single_word"].as_bool().unwrap(),
                    lstrip: value["lstrip"].as_bool().unwrap(),
                    rstrip: value["rstrip"].as_bool().unwrap(),
                    normalized: value["normalized"].as_bool().unwrap(),
                }]);
            }
        }
    }
    let tokenizer: Tokenizer = tokenizer;
    Ok(TokenizerSizer::new(tokenizer))
}
