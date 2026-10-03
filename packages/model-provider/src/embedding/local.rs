use anyhow::Result;
use fastembed::{Embedding, OutputKey, Pooling, SingleBatchOutput, TextEmbedding};

/// Preserve a model's projected sentence vectors when it also returns raw token states.
pub fn embed<S: AsRef<str> + Send + Sync>(
    model: &mut TextEmbedding,
    texts: impl AsRef<[S]>,
    batch_size: Option<usize>,
    pooling: &Pooling,
) -> Result<Vec<Embedding>> {
    model
        .transform(texts, batch_size)?
        .export_with_transformer(|batches| embedding_output_vectors(batches, pooling))
}

fn embedding_output_vectors(
    batches: &[SingleBatchOutput],
    pooling: &Pooling,
) -> Result<Vec<Embedding>> {
    const DEFAULT_OUTPUTS: &[OutputKey] = &[
        OutputKey::OnlyOne,
        OutputKey::ByName("text_embeds"),
        OutputKey::ByName("last_hidden_state"),
        OutputKey::ByName("sentence_embedding"),
    ];
    const SENTENCE_OUTPUT: &[OutputKey] = &[OutputKey::ByName("sentence_embedding")];

    let mut vectors = Vec::new();
    for batch in batches {
        let mut precedence = DEFAULT_OUTPUTS;
        if batch.outputs.len() > 1
            && !batch.outputs.iter().any(|(name, _)| name == "text_embeds")
            && let Some((_, sentence)) = batch
                .outputs
                .iter()
                .find(|(name, _)| name == "sentence_embedding")
            && sentence.try_extract_array::<f32>()?.ndim() == 2
        {
            // A sentence output can include learned projection layers after pooling.
            // Re-pooling the raw hidden states would skip those layers.
            precedence = SENTENCE_OUTPUT;
        }

        let embeddings = batch.select_and_pool_output(&precedence, Some(pooling.clone()))?;
        for row in embeddings.rows() {
            let norm = row.iter().map(|value| value * value).sum::<f32>().sqrt();
            vectors.push(row.iter().map(|value| value / (norm + 1e-12)).collect());
        }
    }
    Ok(vectors)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ml::ort_runtime::ensure_ort_initialized;
    use ndarray::array;
    use ort::value::{Tensor, Value};

    fn tensor(shape: &[usize], values: &[f32]) -> Value {
        ensure_ort_initialized().unwrap();
        Tensor::from_array((shape.to_vec(), values.to_vec()))
            .unwrap()
            .into_dyn()
    }

    fn assert_vector(actual: &[f32], expected: &[f32]) {
        assert_eq!(actual.len(), expected.len());
        for (actual, expected) in actual.iter().zip(expected) {
            assert!((actual - expected).abs() < 1e-6, "{actual} != {expected}");
        }
    }

    #[test]
    fn embedding_output_uses_projected_sentence_vectors_before_token_states() {
        for pooling in [Pooling::Mean, Pooling::Cls] {
            let batch = SingleBatchOutput {
                outputs: vec![
                    (
                        "last_hidden_state".into(),
                        tensor(&[2, 2, 2], &[4., 0., 2., 0., 0., 1., 0., 2.]),
                    ),
                    (
                        "sentence_embedding".into(),
                        tensor(&[2, 2], &[0., 2., 3., 4.]),
                    ),
                ],
                attention_mask_array: array![[1, 1], [1, 1]],
            };
            let vectors = embedding_output_vectors(&[batch], &pooling).unwrap();
            assert_eq!(vectors.len(), 2);
            assert_vector(&vectors[0], &[0., 1.]);
            assert_vector(&vectors[1], &[0.6, 0.8]);
        }
    }

    #[test]
    fn embedding_output_pools_token_only_models_with_the_declared_strategy() {
        for name in ["last_hidden_state", "token_embeddings"] {
            let batch = SingleBatchOutput {
                outputs: vec![(
                    name.into(),
                    tensor(&[1, 3, 2], &[3., 0., 0., 4., 100., 100.]),
                )],
                attention_mask_array: array![[1, 1, 0]],
            };
            let batches = [batch];
            let mean = embedding_output_vectors(&batches, &Pooling::Mean).unwrap();
            let cls = embedding_output_vectors(&batches, &Pooling::Cls).unwrap();
            assert_vector(&mean[0], &[0.6, 0.8]);
            assert_vector(&cls[0], &[1., 0.]);
        }
    }

    #[test]
    fn embedding_output_preserves_text_embeds_precedence() {
        let batch = SingleBatchOutput {
            outputs: vec![
                ("sentence_embedding".into(), tensor(&[1, 2], &[0., 5.])),
                ("text_embeds".into(), tensor(&[1, 2], &[3., 4.])),
            ],
            attention_mask_array: array![[1]],
        };
        let vectors = embedding_output_vectors(&[batch], &Pooling::Mean).unwrap();
        assert_vector(&vectors[0], &[0.6, 0.8]);
    }

    #[test]
    fn embedding_output_only_promotes_sentence_vectors_that_are_already_pooled() {
        let batch = SingleBatchOutput {
            outputs: vec![
                (
                    "sentence_embedding".into(),
                    tensor(&[1, 2, 2], &[0., 1., 0., 1.]),
                ),
                (
                    "last_hidden_state".into(),
                    tensor(&[1, 2, 2], &[3., 0., 0., 4.]),
                ),
            ],
            attention_mask_array: array![[1, 1]],
        };
        let vectors = embedding_output_vectors(&[batch], &Pooling::Mean).unwrap();
        assert_vector(&vectors[0], &[0.6, 0.8]);
    }

    #[test]
    fn embedding_output_rejects_invalid_sentence_tensor_instead_of_using_token_states() {
        ensure_ort_initialized().unwrap();
        let invalid = Tensor::from_array(([1, 2], vec![0_i64, 1]))
            .unwrap()
            .into_dyn();
        let batch = SingleBatchOutput {
            outputs: vec![
                (
                    "last_hidden_state".into(),
                    tensor(&[1, 2, 2], &[3., 0., 0., 4.]),
                ),
                ("sentence_embedding".into(), invalid),
            ],
            attention_mask_array: array![[1, 1]],
        };
        assert!(embedding_output_vectors(&[batch], &Pooling::Mean).is_err());
    }
}
