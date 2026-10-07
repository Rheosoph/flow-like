use super::{Pooling, SessionOptions, TokenizerFiles, load_tokenizer, output::text_vectors};
use anyhow::{Result, anyhow, ensure};
use ndarray::Array2;
use ort::{session::Session, value::TensorRef};
use std::{path::Path, sync::Arc};
use tokenizers::{Encoding, Tokenizer, utils::padding::pad_encodings};

/// Token tensors before inference, available for preprocessing parity checks.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TokenizedBatch {
    pub input_ids: Array2<i64>,
    pub attention_mask: Array2<i64>,
    pub token_type_ids: Array2<i64>,
}

/// Immutable tokenization state shared by preprocessing workers and an inference session.
#[derive(Clone)]
pub struct NativeTextPreprocessor {
    inner: Arc<TextTokenizers>,
}

struct TextTokenizers {
    tokenizer: Tokenizer,
    untruncated: Tokenizer,
}

impl NativeTextPreprocessor {
    pub fn new(tokenizer: Tokenizer) -> Result<Self> {
        let mut untruncated = tokenizer.clone();
        untruncated
            .with_truncation(None)
            .map_err(|error| anyhow!(error.to_string()))?;
        untruncated.with_padding(None);
        Ok(Self {
            inner: Arc::new(TextTokenizers {
                tokenizer,
                untruncated,
            }),
        })
    }

    pub fn tokenizer(&self) -> &Tokenizer {
        &self.inner.tokenizer
    }

    /// Encode with the legacy truncation and padding settings.
    pub fn tokenize<S: AsRef<str> + Send + Sync>(&self, texts: &[S]) -> Result<TokenizedBatch> {
        let mut batch = TokenizedBatch::default();
        self.tokenize_into(texts, &mut batch)?;
        Ok(batch)
    }

    /// Reuse the tensor buffers from the previous batch, including when its shape changes.
    pub fn tokenize_into<S: AsRef<str> + Send + Sync>(
        &self,
        texts: &[S],
        batch: &mut TokenizedBatch,
    ) -> Result<()> {
        let encodings = self
            .inner
            .tokenizer
            .encode_batch(texts.iter().map(AsRef::as_ref).collect(), true)
            .map_err(|error| anyhow!("Could not tokenize embedding batch: {error}"))?;
        batch.fill(&encodings)
    }

    /// Encode one complete input without truncation, padding, or parallel batch dispatch.
    pub fn encode_untruncated_one(&self, text: &str) -> Result<Encoding> {
        self.inner
            .untruncated
            .encode(text, true)
            .map_err(|error| anyhow!("Could not tokenize embedding input: {error}"))
    }

    /// Encode complete inputs once, without truncation or padding, for context validation.
    pub fn encode_untruncated<S: AsRef<str> + Send + Sync>(
        &self,
        texts: &[S],
    ) -> Result<Vec<Encoding>> {
        self.inner
            .untruncated
            .encode_batch(texts.iter().map(AsRef::as_ref).collect(), true)
            .map_err(|error| anyhow!("Could not tokenize embedding inputs: {error}"))
    }

    pub fn pad_encodings(&self, encodings: Vec<Encoding>) -> Result<TokenizedBatch> {
        let mut batch = TokenizedBatch::default();
        self.pad_encodings_into(encodings, &mut batch)?;
        Ok(batch)
    }

    /// Pad validated full encodings using the original tokenizer's exact padding policy.
    pub fn pad_encodings_into(
        &self,
        mut encodings: Vec<Encoding>,
        batch: &mut TokenizedBatch,
    ) -> Result<()> {
        if let Some(truncation) = self.inner.tokenizer.get_truncation() {
            ensure!(
                encodings
                    .iter()
                    .all(|encoding| encoding.len() <= truncation.max_length),
                "Full embedding input exceeds the tokenizer context; split it before padding"
            );
        }
        // Truncating after special-token processing can discard a terminal token. The caller
        // validates full lengths instead; the legacy tokenize path still performs truncation.
        if let Some(padding) = self.inner.tokenizer.get_padding() {
            pad_encodings(&mut encodings, padding)
                .map_err(|error| anyhow!("Could not pad embedding inputs: {error}"))?;
        }
        batch.fill(&encodings)
    }
}

impl TokenizedBatch {
    fn fill(&mut self, encodings: &[Encoding]) -> Result<()> {
        let length = encodings.first().map_or(0, Encoding::len);
        ensure!(
            encodings.iter().all(|encoding| encoding.len() == length),
            "Tokenizer must pad to a common batch length"
        );
        let shape = (encodings.len(), length);
        refill_tensor(&mut self.input_ids, shape, encodings, Encoding::get_ids)?;
        refill_tensor(
            &mut self.attention_mask,
            shape,
            encodings,
            Encoding::get_attention_mask,
        )?;
        refill_tensor(
            &mut self.token_type_ids,
            shape,
            encodings,
            Encoding::get_type_ids,
        )
    }
}

fn refill_tensor(
    tensor: &mut Array2<i64>,
    shape: (usize, usize),
    encodings: &[Encoding],
    values: impl Fn(&Encoding) -> &[u32],
) -> Result<()> {
    let (mut buffer, _) = std::mem::take(tensor).into_raw_vec_and_offset();
    buffer.clear();
    buffer.reserve(shape.0 * shape.1);
    for encoding in encodings {
        buffer.extend(values(encoding).iter().map(|value| i64::from(*value)));
    }
    *tensor = Array2::from_shape_vec(shape, buffer)?;
    Ok(())
}

pub struct NativeTextEmbedding {
    preprocessor: NativeTextPreprocessor,
    session: Session,
    pooling: Pooling,
    need_token_type_ids: bool,
}

impl NativeTextEmbedding {
    pub fn new_from_file(
        path: impl AsRef<Path>,
        files: TokenizerFiles,
        max_length: usize,
        pooling: Pooling,
        options: SessionOptions,
    ) -> Result<Self> {
        Self::from_session(options.load_file(path)?, files, max_length, pooling)
    }

    pub fn new_from_memory(
        graph: &[u8],
        files: TokenizerFiles,
        max_length: usize,
        pooling: Pooling,
        options: SessionOptions,
    ) -> Result<Self> {
        Self::from_session(options.load_memory(graph)?, files, max_length, pooling)
    }

    pub fn from_session(
        session: Session,
        files: TokenizerFiles,
        max_length: usize,
        pooling: Pooling,
    ) -> Result<Self> {
        Self::with_tokenizer(session, load_tokenizer(&files, max_length)?, pooling)
    }

    pub fn with_tokenizer(
        session: Session,
        tokenizer: Tokenizer,
        pooling: Pooling,
    ) -> Result<Self> {
        let names = session
            .inputs()
            .iter()
            .map(|input| input.name())
            .collect::<Vec<_>>();
        ensure!(
            names.contains(&"input_ids"),
            "Text graph must accept input_ids"
        );
        ensure!(
            names.contains(&"attention_mask"),
            "Text graph must accept attention_mask"
        );
        ensure!(
            names
                .iter()
                .all(|name| matches!(*name, "input_ids" | "attention_mask" | "token_type_ids")),
            "This text adapter cannot supply graph inputs {names:?}; select the model's multimodal adapter"
        );
        let need_token_type_ids = names.contains(&"token_type_ids");
        Ok(Self {
            preprocessor: NativeTextPreprocessor::new(tokenizer)?,
            session,
            pooling,
            need_token_type_ids,
        })
    }

    pub fn tokenizer(&self) -> &Tokenizer {
        self.preprocessor.tokenizer()
    }

    pub fn preprocessor(&self) -> NativeTextPreprocessor {
        self.preprocessor.clone()
    }

    pub fn output_dimensions(&self) -> Option<usize> {
        let outputs = self.session.outputs();
        let output = if outputs.len() == 1 {
            outputs.first()
        } else {
            outputs
                .iter()
                .find(|output| output.name() == "text_embeds")
                .or_else(|| {
                    outputs.iter().find(|output| {
                        output.name() == "sentence_embedding"
                            && output
                                .dtype()
                                .tensor_shape()
                                .is_some_and(|shape| shape.len() == 2)
                    })
                })
                .or_else(|| {
                    outputs
                        .iter()
                        .find(|output| output.name() == "last_hidden_state")
                })
                .or_else(|| {
                    outputs
                        .iter()
                        .find(|output| output.name() == "sentence_embedding")
                })
        }?;
        let dimensions = *output.dtype().tensor_shape()?.last()?;
        usize::try_from(dimensions)
            .ok()
            .filter(|dimensions| *dimensions > 0)
    }

    pub fn tokenize<S: AsRef<str> + Send + Sync>(&self, texts: &[S]) -> Result<TokenizedBatch> {
        self.preprocessor.tokenize(texts)
    }

    pub fn embed<S: AsRef<str> + Send + Sync>(
        &mut self,
        texts: impl AsRef<[S]>,
        batch_size: Option<usize>,
    ) -> Result<Vec<Vec<f32>>> {
        let batch_size = batch_size.unwrap_or(256);
        ensure!(batch_size > 0, "batch_size must be greater than zero");
        let texts = texts.as_ref();
        let mut embeddings = Vec::with_capacity(texts.len());
        let mut tokens = TokenizedBatch::default();
        for batch in texts.chunks(batch_size) {
            self.preprocessor.tokenize_into(batch, &mut tokens)?;
            embeddings.extend(self.embed_tokens(&tokens)?);
        }
        Ok(embeddings)
    }

    pub fn embed_tokens(&mut self, tokens: &TokenizedBatch) -> Result<Vec<Vec<f32>>> {
        ensure!(
            tokens.input_ids.dim() == tokens.attention_mask.dim()
                && tokens.input_ids.dim() == tokens.token_type_ids.dim(),
            "Token tensor shapes differ"
        );
        if tokens.input_ids.nrows() == 0 {
            return Ok(Vec::new());
        }
        ensure!(
            tokens.input_ids.ncols() > 0,
            "Cannot embed a batch with zero tokens"
        );
        let mut inputs = ort::inputs![
            "input_ids" => TensorRef::from_array_view(&tokens.input_ids)?,
            "attention_mask" => TensorRef::from_array_view(&tokens.attention_mask)?,
        ];
        if self.need_token_type_ids {
            inputs.push((
                "token_type_ids".into(),
                TensorRef::from_array_view(&tokens.token_type_ids)?.into(),
            ));
        }
        let outputs = self.session.run(inputs)?;
        let vectors = text_vectors(&outputs, tokens.attention_mask.view(), self.pooling)?;
        ensure!(
            vectors.len() == tokens.input_ids.nrows(),
            "Embedding graph changed batch size"
        );
        // Only vectors survive the batch. Token-level outputs can dominate memory.
        Ok(vectors)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokenizers::{PaddingDirection, PaddingParams, PaddingStrategy, TruncationParams};

    fn preprocessor(padding: PaddingParams) -> NativeTextPreprocessor {
        let mut tokenizer = Tokenizer::from_bytes(
            br#"{
                "version":"1.0","truncation":null,"padding":null,
                "added_tokens":[],"normalizer":null,
                "pre_tokenizer":{"type":"Whitespace"},"post_processor":null,
                "decoder":null,"model":{"type":"WordLevel","vocab":{
                    "[UNK]":0,"[PAD]":1,"hello":2,"world":3,"[CLS]":4,"[SEP]":5
                },"unk_token":"[UNK]"}
            }"#,
        )
        .unwrap();
        tokenizer.with_post_processor(Some(tokenizers::processors::bert::BertProcessing::new(
            ("[SEP]".into(), 5),
            ("[CLS]".into(), 4),
        )));
        tokenizer
            .with_truncation(Some(TruncationParams {
                max_length: 8,
                ..Default::default()
            }))
            .unwrap();
        tokenizer.with_padding(Some(padding));
        NativeTextPreprocessor::new(tokenizer).unwrap()
    }

    fn padding() -> PaddingParams {
        PaddingParams {
            pad_id: 1,
            pad_token: "[PAD]".into(),
            pad_type_id: 7,
            ..Default::default()
        }
    }

    #[test]
    fn full_encodings_match_original_padding_and_special_tokens() {
        for direction in [PaddingDirection::Left, PaddingDirection::Right] {
            for strategy in [PaddingStrategy::BatchLongest, PaddingStrategy::Fixed(8)] {
                let preprocessor = preprocessor(PaddingParams {
                    direction,
                    strategy,
                    pad_to_multiple_of: Some(4),
                    ..padding()
                });
                let texts = ["", "hello", "hello world hello world hello world"];
                let encodings = preprocessor.encode_untruncated(&texts).unwrap();
                for (text, expected) in texts.iter().zip(&encodings) {
                    assert_eq!(
                        preprocessor.encode_untruncated_one(text).unwrap(),
                        *expected
                    );
                }
                assert_eq!(
                    encodings.iter().map(Encoding::len).collect::<Vec<_>>(),
                    [2, 3, 8]
                );
                assert_eq!(encodings[0].get_ids(), [4, 5]);
                assert_eq!(
                    preprocessor.pad_encodings(encodings).unwrap(),
                    preprocessor.tokenize(&texts).unwrap()
                );
                let short = ["", "hello"];
                assert_eq!(
                    preprocessor
                        .pad_encodings(preprocessor.encode_untruncated(&short).unwrap())
                        .unwrap(),
                    preprocessor.tokenize(&short).unwrap()
                );
            }
        }
    }

    #[test]
    fn full_context_validation_never_discards_terminal_special_token() {
        let preprocessor = preprocessor(padding());
        let texts = ["hello world hello world hello world hello"];
        let encodings = preprocessor.encode_untruncated(&texts).unwrap();
        assert_eq!(encodings[0].len(), 9);
        assert_eq!(encodings[0].get_ids().last(), Some(&5));
        assert!(preprocessor.pad_encodings(encodings).is_err());
        let legacy = preprocessor.tokenize(&texts).unwrap();
        assert_eq!(legacy.input_ids.row(0).to_vec(), [4, 2, 3, 2, 3, 2, 3, 5]);
    }

    #[test]
    fn reusable_tensors_keep_capacity_and_reset_empty_batches() {
        let preprocessor = preprocessor(padding());
        let mut batch = preprocessor
            .tokenize(&["hello world hello", "world hello world"])
            .unwrap();
        let pointers = (
            batch.input_ids.as_ptr(),
            batch.attention_mask.as_ptr(),
            batch.token_type_ids.as_ptr(),
        );
        let cloned = preprocessor.clone();
        assert!(Arc::ptr_eq(&preprocessor.inner, &cloned.inner));
        let texts = ["hello"];
        cloned.tokenize_into(&texts, &mut batch).unwrap();
        assert_eq!(batch, preprocessor.tokenize(&texts).unwrap());
        assert_eq!(batch.input_ids.as_ptr(), pointers.0);
        assert_eq!(batch.attention_mask.as_ptr(), pointers.1);
        assert_eq!(batch.token_type_ids.as_ptr(), pointers.2);

        preprocessor
            .pad_encodings_into(Vec::new(), &mut batch)
            .unwrap();
        assert_eq!(batch.input_ids.dim(), (0, 0));
        preprocessor
            .pad_encodings_into(preprocessor.encode_untruncated(&texts).unwrap(), &mut batch)
            .unwrap();
        assert_eq!(batch, preprocessor.tokenize(&texts).unwrap());
        assert_eq!(batch.input_ids.as_ptr(), pointers.0);
        assert_eq!(batch.attention_mask.as_ptr(), pointers.1);
        assert_eq!(batch.token_type_ids.as_ptr(), pointers.2);
        preprocessor.tokenize_into::<&str>(&[], &mut batch).unwrap();
        assert_eq!(batch, TokenizedBatch::default());
    }

    #[test]
    fn missing_padding_preserves_equal_length_requirement() {
        let mut tokenizer = preprocessor(padding()).tokenizer().clone();
        tokenizer.with_padding(None);
        let preprocessor = NativeTextPreprocessor::new(tokenizer).unwrap();
        let texts = ["hello", "hello world"];
        assert!(preprocessor.tokenize(&texts).is_err());
        assert!(
            preprocessor
                .pad_encodings(preprocessor.encode_untruncated(&texts).unwrap())
                .is_err()
        );
    }
}
