//! Numerically stable scalar primitives, generic over [`linfa::Float`].
//!
//! The ordinal likelihoods work with differences of sigmoids, which lose precision quickly when a
//! naive `1/(1+exp(-z))` overflows. Every helper here is written to stay finite across the whole
//! real line so a badly scaled feature column degrades accuracy instead of producing NaN.

use linfa::Float;

/// Beyond this magnitude `exp` is either saturating or vanishing, so the linear/zero limits are
/// both exact enough and safe.
fn saturation<F: Float>() -> F {
    F::cast(30.0)
}

/// Logistic sigmoid, evaluated on whichever side of zero avoids overflowing `exp`.
pub fn sigmoid<F: Float>(z: F) -> F {
    if z >= F::zero() {
        F::one() / (F::one() + (-z).exp())
    } else {
        let e = z.exp();
        e / (F::one() + e)
    }
}

/// `ln(1 + e^z)`, linear for large `z` where `e^z` would overflow.
pub fn softplus<F: Float>(z: F) -> F {
    if z > saturation::<F>() {
        z
    } else if z < -saturation::<F>() {
        z.exp()
    } else {
        z.exp().ln_1p()
    }
}

/// Inverse of [`softplus`], i.e. `ln(e^d - 1)`. `d` must be strictly positive.
pub fn softplus_inv<F: Float>(d: F) -> F {
    if d > saturation::<F>() {
        d
    } else {
        // `exp_m1` keeps precision for the small-`d` case where `e^d - 1` cancels badly.
        d.exp_m1().max(F::cast(f64::MIN_POSITIVE)).ln()
    }
}

/// `ln(p / (1 - p))` for a probability strictly inside (0, 1).
pub fn logit<F: Float>(p: F) -> F {
    (p / (F::one() - p)).ln()
}

/// Smallest probability the likelihood is allowed to see.
///
/// A category probability can underflow to exactly zero once the thresholds separate widely, and
/// the log-likelihood would then be `-inf` with a non-finite gradient. Flooring it keeps the
/// optimizer moving in the right direction instead of stalling on NaN.
pub fn min_probability<F: Float>() -> F {
    F::cast(1e-12)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sigmoid_is_finite_at_extremes() {
        assert!(sigmoid(1000.0f64).is_finite());
        assert!(sigmoid(-1000.0f64).is_finite());
        assert_eq!(sigmoid(0.0f64), 0.5);
        assert!(sigmoid(-1000.0f64) >= 0.0);
        assert!(sigmoid(1000.0f64) <= 1.0);
        // The f32 instantiation must be just as safe, since `Float` covers both.
        assert!(sigmoid(1000.0f32).is_finite());
        assert!(sigmoid(-1000.0f32).is_finite());
    }

    #[test]
    fn softplus_round_trips() {
        for d in [1e-6f64, 0.01, 0.5, 1.0, 5.0, 25.0, 100.0] {
            let recovered = softplus(softplus_inv(d));
            assert!(
                (recovered - d).abs() <= 1e-6 * d.max(1.0),
                "softplus(softplus_inv({d})) = {recovered}"
            );
        }
    }

    #[test]
    fn softplus_stays_positive() {
        assert!(softplus(-1000.0f64) >= 0.0);
        assert!(softplus(1000.0f64).is_finite());
        assert!(softplus(-1000.0f32) >= 0.0);
    }
}
