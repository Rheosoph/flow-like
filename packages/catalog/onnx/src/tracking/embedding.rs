/// L2-normalizes `values`. Returns `None` for an empty, zero or non-finite vector.
pub fn normalized(values: &[f32]) -> Option<Vec<f32>> {
    if values.is_empty() || values.iter().any(|v| !v.is_finite()) {
        return None;
    }
    let norm = values.iter().map(|v| v * v).sum::<f32>().sqrt();
    if !norm.is_finite() || norm <= f32::EPSILON {
        return None;
    }
    Some(values.iter().map(|v| v / norm).collect())
}

/// Cosine similarity of two L2-normalized vectors, clamped to [-1, 1]. Mismatched lengths are 0.
pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    a.iter()
        .zip(b)
        .map(|(x, y)| x * y)
        .sum::<f32>()
        .clamp(-1.0, 1.0)
}

/// Exponential moving average `alpha * current + (1 - alpha) * next`, re-normalized.
/// Falls back to `next` when the blend degenerates or the lengths differ.
pub fn blend(current: &[f32], next: &[f32], alpha: f32) -> Vec<f32> {
    if current.len() != next.len() {
        return next.to_vec();
    }
    let mixed: Vec<f32> = current
        .iter()
        .zip(next)
        .map(|(c, n)| alpha * c + (1.0 - alpha) * n)
        .collect();
    normalized(&mixed).unwrap_or_else(|| next.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_to_unit_length() {
        let v = normalized(&[3.0, 4.0]).unwrap();
        assert!((v[0] - 0.6).abs() < 1e-6);
        assert!((v[1] - 0.8).abs() < 1e-6);
    }

    #[test]
    fn rejects_degenerate_vectors() {
        assert!(normalized(&[]).is_none());
        assert!(normalized(&[0.0, 0.0]).is_none());
        assert!(normalized(&[f32::NAN, 1.0]).is_none());
        assert!(normalized(&[f32::INFINITY, 1.0]).is_none());
    }

    #[test]
    fn cosine_of_unit_vectors() {
        assert!((cosine(&[1.0, 0.0], &[1.0, 0.0]) - 1.0).abs() < 1e-6);
        assert!(cosine(&[1.0, 0.0], &[0.0, 1.0]).abs() < 1e-6);
        assert_eq!(cosine(&[1.0], &[1.0, 0.0]), 0.0);
    }

    #[test]
    fn blend_stays_normalized() {
        let v = blend(&[1.0, 0.0], &[0.0, 1.0], 0.9);
        let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-5);
        assert!(v[0] > v[1]);
        assert_eq!(blend(&[1.0, 0.0], &[-1.0, 0.0], 0.5), vec![-1.0, 0.0]);
    }
}
