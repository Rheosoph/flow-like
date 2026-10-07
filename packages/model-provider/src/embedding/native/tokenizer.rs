// Adapted from FastEmbed 5.17.2 (Apache-2.0), with optional metadata and validation.
// Attribution: thirdparty/manual-notices/fastembed-compatibility.md.
use anyhow::{Context, Result, anyhow, ensure};
use tokenizers::{AddedToken, PaddingParams, PaddingStrategy, Tokenizer, TruncationParams};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenizerFiles {
    pub tokenizer_file: Vec<u8>,
    pub config_file: Vec<u8>,
    pub special_tokens_map_file: Vec<u8>,
    pub tokenizer_config_file: Vec<u8>,
}

/// Load the tokenizer contract used by existing embedding Bits.
pub fn load_tokenizer(files: &TokenizerFiles, requested_max_length: usize) -> Result<Tokenizer> {
    ensure!(
        requested_max_length > 0,
        "max_length must be greater than zero"
    );
    let config: serde_json::Value =
        serde_json::from_slice(&files.config_file).context("Invalid config.json")?;
    let special_tokens: serde_json::Value = if files.special_tokens_map_file.is_empty() {
        serde_json::json!({})
    } else {
        serde_json::from_slice(&files.special_tokens_map_file)
            .context("Invalid special_tokens_map.json")?
    };
    let tokenizer_config: serde_json::Value = serde_json::from_slice(&files.tokenizer_config_file)
        .context("Invalid tokenizer_config.json")?;
    let mut tokenizer = Tokenizer::from_bytes(&files.tokenizer_file)
        .map_err(|error| anyhow!("Invalid tokenizer.json: {error}"))?;

    // Keep the f32 conversion used by the legacy loader, including huge HF sentinels.
    let declared_max_length = tokenizer_config["model_max_length"]
        .as_f64()
        .ok_or_else(|| anyhow!("tokenizer_config.json is missing numeric model_max_length"))?
        as f32 as usize;
    let max_length = requested_max_length.min(declared_max_length);
    ensure!(
        max_length > 0,
        "Tokenizer model_max_length must be greater than zero"
    );
    let pad_id = config["pad_token_id"].as_u64().unwrap_or(0) as u32;
    let pad_token = tokenizer_config["pad_token"]
        .as_str()
        .or_else(|| tokenizer_config["pad_token"]["content"].as_str())
        .ok_or_else(|| anyhow!("tokenizer_config.json is missing pad_token"))?
        .to_owned();

    // Existing vectors use right padding and right truncation even when metadata
    // advertises another direction. New model-family adapters configure their own tokenizer.
    tokenizer.with_padding(Some(PaddingParams {
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
        .map_err(|error| anyhow!(error.to_string()))?;

    if let Some(tokens) = special_tokens.as_object() {
        for value in tokens.values() {
            let token = if let Some(content) = value.as_str() {
                Some(AddedToken {
                    content: content.to_owned(),
                    special: true,
                    ..Default::default()
                })
            } else if let (
                Some(content),
                Some(single_word),
                Some(lstrip),
                Some(rstrip),
                Some(normalized),
            ) = (
                value["content"].as_str(),
                value["single_word"].as_bool(),
                value["lstrip"].as_bool(),
                value["rstrip"].as_bool(),
                value["normalized"].as_bool(),
            ) {
                Some(AddedToken {
                    content: content.to_owned(),
                    special: true,
                    single_word,
                    lstrip,
                    rstrip,
                    normalized,
                })
            } else {
                None
            };
            if let Some(token) = token {
                tokenizer.add_special_tokens(&[token]);
            }
        }
    }
    Ok(tokenizer)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files() -> TokenizerFiles {
        TokenizerFiles {
            tokenizer_file: br#"{
                "version":"1.0","truncation":null,"padding":null,
                "added_tokens":[],"normalizer":null,
                "pre_tokenizer":{"type":"Whitespace"},"post_processor":null,
                "decoder":null,"model":{"type":"WordLevel","vocab":{
                    "[UNK]":0,"[PAD]":1,"hello":2,"world":3
                },"unk_token":"[UNK]"}
            }"#
            .to_vec(),
            config_file: br#"{"pad_token_id":1}"#.to_vec(),
            special_tokens_map_file: br#"{"pad_token":"[PAD]","unk_token":"[UNK]"}"#.to_vec(),
            tokenizer_config_file: br#"{
                "model_max_length":3,"pad_token":"[PAD]","padding_side":"left"
            }"#
            .to_vec(),
        }
    }

    #[test]
    fn legacy_tokenizer_keeps_right_padding_masks_and_truncation() {
        let tokenizer = load_tokenizer(&files(), 2).unwrap();
        let encodings = tokenizer
            .encode_batch(vec!["hello", "hello world hello"], true)
            .unwrap();
        assert_eq!(encodings[0].get_ids(), &[2, 1]);
        assert_eq!(encodings[0].get_attention_mask(), &[1, 0]);
        assert_eq!(encodings[1].get_ids(), &[2, 3]);
        assert_eq!(encodings[1].get_attention_mask(), &[1, 1]);
        assert_eq!(encodings[1].get_type_ids(), &[0, 0]);
    }

    #[test]
    fn tokenizer_rejects_zero_context_and_accepts_missing_optional_special_map() {
        assert!(load_tokenizer(&files(), 0).is_err());
        let mut files = files();
        files.special_tokens_map_file.clear();
        assert!(load_tokenizer(&files, 3).is_ok());
    }
}
