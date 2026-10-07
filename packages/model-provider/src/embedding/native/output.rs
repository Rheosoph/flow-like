// Adapted from FastEmbed 5.17.2 (Apache-2.0), with projected-output priority and in-place normalization.
// Attribution: thirdparty/manual-notices/fastembed-compatibility.md.
use anyhow::{Result, anyhow, ensure};
use ndarray::{ArrayView2, ArrayViewD, Axis, Ix2, Ix3};
use ort::{session::SessionOutputs, value::DynValue};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Pooling {
    Mean,
    Cls,
    LastToken,
}

/// Normalize in place with the same epsilon and summation order as existing vectors.
pub fn normalize(vector: &mut [f32]) {
    let norm = vector.iter().map(|value| value * value).sum::<f32>().sqrt();
    for value in vector {
        *value /= norm + 1e-12;
    }
}

pub(crate) fn text_vectors(
    outputs: &SessionOutputs<'_>,
    mask: ArrayView2<'_, i64>,
    pooling: Pooling,
) -> Result<Vec<Vec<f32>>> {
    let tensor = select_text_output(
        outputs.len(),
        (outputs.len() > 0).then(|| &outputs[0]),
        |name| outputs.get(name),
    )?
    .try_extract_array::<f32>()?;
    pool(tensor, Some(mask), pooling)
}

fn select_text_output<'a>(
    length: usize,
    first: Option<&'a DynValue>,
    get: impl Fn(&str) -> Option<&'a DynValue>,
) -> Result<&'a DynValue> {
    if length == 1 {
        first.ok_or_else(|| anyhow!("Embedding graph returned no outputs"))
    } else if let Some(text) = get("text_embeds") {
        Ok(text)
    } else if let Some(sentence) = get("sentence_embedding")
        && sentence.try_extract_array::<f32>()?.ndim() == 2
    {
        Ok(sentence)
    } else if let Some(tokens) = get("last_hidden_state") {
        Ok(tokens)
    } else if let Some(sentence) = get("sentence_embedding") {
        Ok(sentence)
    } else {
        Err(anyhow!("Embedding graph has no recognized text output"))
    }
}

pub(crate) fn image_vectors(outputs: &SessionOutputs<'_>) -> Result<Vec<Vec<f32>>> {
    let names: Vec<&str> = if outputs.len() == 1 {
        outputs.keys().collect()
    } else {
        vec!["image_embeds", "last_hidden_state"]
    };
    let tensor = names
        .iter()
        .find_map(|name| {
            outputs
                .get(*name)
                .and_then(|value| value.try_extract_array::<f32>().ok())
        })
        .ok_or_else(|| anyhow!("Embedding graph has no recognized float image output"))?;
    pool(tensor, None, Pooling::Cls)
}

pub(crate) fn pool(
    tensor: ArrayViewD<'_, f32>,
    mask: Option<ArrayView2<'_, i64>>,
    pooling: Pooling,
) -> Result<Vec<Vec<f32>>> {
    let mut vectors = match tensor.ndim() {
        2 => tensor
            .into_dimensionality::<Ix2>()?
            .rows()
            .into_iter()
            .map(|row| row.to_vec())
            .collect(),
        3 => {
            let tensor = tensor.into_dimensionality::<Ix3>()?;
            let [batch, sequence, dimensions] =
                [tensor.shape()[0], tensor.shape()[1], tensor.shape()[2]];
            ensure!(sequence > 0, "Embedding graph returned an empty token axis");
            if let Some(mask) = mask {
                ensure!(
                    mask.shape() == [batch, sequence],
                    "Attention mask and embedding tensor shapes differ"
                );
            }
            let mut vectors = Vec::with_capacity(batch);
            for (index, tokens) in tensor.axis_iter(Axis(0)).enumerate() {
                let vector = match pooling {
                    Pooling::Cls => tokens.row(0).to_vec(),
                    Pooling::LastToken => {
                        let row = mask
                            .and_then(|mask| mask.row(index).iter().rposition(|value| *value != 0))
                            .ok_or_else(|| {
                                anyhow!("Last-token pooling needs a nonempty attention mask")
                            })?;
                        tokens.row(row).to_vec()
                    }
                    Pooling::Mean => {
                        let mask =
                            mask.ok_or_else(|| anyhow!("Mean pooling requires an attention mask"))?;
                        let mut sum = vec![0.0; dimensions];
                        let mut weight = 0.0f32;
                        for (token, masked) in tokens.rows().into_iter().zip(mask.row(index)) {
                            let masked = *masked as f32;
                            weight += masked;
                            for (sum, value) in sum.iter_mut().zip(token) {
                                *sum += masked * value;
                            }
                        }
                        let denominator = if weight == 0.0 { 1.0 } else { weight };
                        for value in &mut sum {
                            *value /= denominator;
                        }
                        sum
                    }
                };
                vectors.push(vector);
            }
            vectors
        }
        _ => {
            return Err(anyhow!(
                "Expected a 2D or 3D embedding tensor, got {:?}",
                tensor.shape()
            ));
        }
    };
    for vector in &mut vectors {
        ensure!(
            !vector.is_empty(),
            "Embedding graph returned zero dimensions"
        );
        ensure!(
            vector.iter().all(|value| value.is_finite()),
            "Embedding graph returned nonfinite values"
        );
        normalize(vector);
    }
    Ok(vectors)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ndarray::array;
    use ort::value::Tensor;

    fn tensor(shape: &[usize], values: &[f32]) -> DynValue {
        crate::ml::ort_runtime::ensure_ort_initialized().unwrap();
        Tensor::from_array((shape.to_vec(), values.to_vec()))
            .unwrap()
            .into_dyn()
    }

    fn named_vectors(outputs: &[(&str, DynValue)], pooling: Pooling) -> Result<Vec<Vec<f32>>> {
        let tensor = select_text_output(
            outputs.len(),
            outputs.first().map(|(_, value)| value),
            |name| {
                outputs
                    .iter()
                    .find(|(key, _)| *key == name)
                    .map(|(_, value)| value)
            },
        )?
        .try_extract_array::<f32>()?;
        pool(tensor, Some(array![[1, 1]].view()), pooling)
    }

    #[test]
    fn output_uses_projected_sentence_vectors_before_token_states() {
        let outputs = [
            ("last_hidden_state", tensor(&[1, 2, 2], &[4., 0., 2., 0.])),
            ("sentence_embedding", tensor(&[1, 2], &[3., 4.])),
        ];
        for strategy in [Pooling::Mean, Pooling::Cls] {
            assert_eq!(
                named_vectors(&outputs, strategy).unwrap(),
                vec![vec![0.6, 0.8]]
            );
        }
    }

    #[test]
    fn output_preserves_text_embeds_precedence() {
        let outputs = [
            ("sentence_embedding", tensor(&[1, 2], &[0., 5.])),
            ("text_embeds", tensor(&[1, 2], &[3., 4.])),
        ];
        assert_eq!(
            named_vectors(&outputs, Pooling::Mean).unwrap(),
            vec![vec![0.6, 0.8]]
        );
    }

    #[test]
    fn output_only_promotes_already_pooled_sentence_vectors() {
        let outputs = [
            ("sentence_embedding", tensor(&[1, 2, 2], &[0., 1., 0., 1.])),
            ("last_hidden_state", tensor(&[1, 2, 2], &[3., 0., 0., 4.])),
        ];
        assert_eq!(
            named_vectors(&outputs, Pooling::Mean).unwrap(),
            vec![vec![0.6, 0.8]]
        );
    }

    #[test]
    fn output_rejects_invalid_sentence_tensor_instead_of_using_token_states() {
        crate::ml::ort_runtime::ensure_ort_initialized().unwrap();
        let outputs = [
            ("last_hidden_state", tensor(&[1, 2, 2], &[3., 0., 0., 4.])),
            (
                "sentence_embedding",
                Tensor::from_array(([1, 2], vec![0_i64, 1]))
                    .unwrap()
                    .into_dyn(),
            ),
        ];
        assert!(named_vectors(&outputs, Pooling::Mean).is_err());
    }

    #[test]
    fn output_accepts_one_arbitrary_name_and_rejects_unknown_multiple_names() {
        let outputs = [("token_embeddings", tensor(&[1, 2, 2], &[3., 0., 0., 4.]))];
        assert_eq!(
            named_vectors(&outputs, Pooling::Mean).unwrap(),
            vec![vec![0.6, 0.8]]
        );
        assert_eq!(
            named_vectors(&outputs, Pooling::Cls).unwrap(),
            vec![vec![1., 0.]]
        );
        let unknown = [
            ("a", tensor(&[1, 2], &[1., 0.])),
            ("b", tensor(&[1, 2], &[0., 1.])),
        ];
        assert!(named_vectors(&unknown, Pooling::Mean).is_err());
    }

    #[test]
    fn masked_mean_excludes_padding_and_cls_preserves_first_token() {
        let tokens = array![[[3., 0.], [0., 4.], [100., 100.]]];
        let mask = array![[1, 1, 0]];
        let mean = pool(tokens.view().into_dyn(), Some(mask.view()), Pooling::Mean).unwrap();
        let cls = pool(tokens.view().into_dyn(), Some(mask.view()), Pooling::Cls).unwrap();
        assert_eq!(mean[0], vec![0.6, 0.8]);
        assert_eq!(cls[0], vec![1., 0.]);
    }

    #[test]
    fn last_token_respects_left_and_right_padding() {
        let tokens = array![[[100., 100.], [3., 4.], [100., 100.]]];
        let mask = array![[0, 1, 0]];
        assert_eq!(
            pool(
                tokens.view().into_dyn(),
                Some(mask.view()),
                Pooling::LastToken
            )
            .unwrap()[0],
            vec![0.6, 0.8]
        );
    }

    #[test]
    fn pooled_projection_is_preserved_for_every_strategy() {
        let values = array![[0., 2.], [3., 4.]];
        for strategy in [Pooling::Mean, Pooling::Cls, Pooling::LastToken] {
            assert_eq!(
                pool(values.view().into_dyn(), None, strategy).unwrap(),
                vec![vec![0., 1.], vec![0.6, 0.8]]
            );
        }
    }

    #[test]
    fn invalid_shapes_and_nonfinite_values_are_errors() {
        assert!(pool(array![[f32::NAN]].view().into_dyn(), None, Pooling::Cls).is_err());
        assert!(pool(array![1., 2.].view().into_dyn(), None, Pooling::Cls).is_err());
        assert!(
            pool(
                array![[[1.]]].view().into_dyn(),
                Some(array![[1, 1]].view()),
                Pooling::Mean
            )
            .is_err()
        );
    }
}
