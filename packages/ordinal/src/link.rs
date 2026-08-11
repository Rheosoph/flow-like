//! Cumulative link functions.
//!
//! A cumulative-link ordinal model reads `P(y <= k | x) = G(theta_k - x . beta)`, where `G` is a
//! CDF. Which CDF you pick *is* the modelling choice:
//!
//! - [`Link::Logit`] gives the proportional-odds model; coefficients are log odds ratios.
//! - [`Link::Probit`] gives the ordered probit model, the default in econometrics and the social
//!   sciences, where the latent variable is assumed normal.
//! - [`Link::CLogLog`] is asymmetric — it approaches the top level slowly and the bottom quickly —
//!   which is the right shape for "time until something escalates" style targets.
//! - [`Link::Cauchit`] has heavy tails, so extreme observations pull the fit far less than under
//!   logit or probit.
//!
//! Every function here is written to stay finite across the whole real line: a badly scaled feature
//! column should cost accuracy, not produce NaN.

use linfa::Float;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// The CDF used by a cumulative-link ordinal model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum Link {
    /// Logistic CDF. The proportional-odds model.
    #[default]
    Logit,
    /// Standard normal CDF. Ordered probit.
    Probit,
    /// Complementary log-log, `1 - exp(-exp(z))`. Asymmetric.
    CLogLog,
    /// Cauchy CDF. Heavy-tailed, so outliers have limited leverage.
    Cauchit,
}

impl Link {
    /// `G(z)`, the cumulative probability.
    pub fn cdf<F: Float>(&self, z: F) -> F {
        match self {
            Link::Logit => logistic_cdf(z),
            Link::Probit => normal_cdf(z),
            Link::CLogLog => {
                // 1 - exp(-exp(z)). Saturates rather than overflowing at either end.
                if z > F::cast(4.0) {
                    F::one()
                } else if z < F::cast(-40.0) {
                    F::zero()
                } else {
                    -(-z.exp()).exp_m1()
                }
            }
            Link::Cauchit => F::cast(0.5) + z.atan() / F::cast(std::f64::consts::PI),
        }
    }

    /// `g(z) = dG/dz`, the density. This is the factor every gradient in the model needs.
    pub fn pdf<F: Float>(&self, z: F) -> F {
        match self {
            Link::Logit => {
                let s = logistic_cdf(z);
                s * (F::one() - s)
            }
            Link::Probit => normal_pdf(z),
            Link::CLogLog => {
                // exp(z - exp(z)), evaluated in log space so neither factor overflows alone.
                if z > F::cast(4.0) || z < F::cast(-40.0) {
                    F::zero()
                } else {
                    (z - z.exp()).exp()
                }
            }
            Link::Cauchit => F::one() / (F::cast(std::f64::consts::PI) * (F::one() + z * z)),
        }
    }

    /// `G^-1(p)`, used to seed the thresholds from the empirical cumulative level frequencies.
    ///
    /// `p` must lie strictly inside `(0, 1)`; callers clamp it.
    pub fn inverse_cdf<F: Float>(&self, p: F) -> F {
        match self {
            Link::Logit => (p / (F::one() - p)).ln(),
            Link::Probit => normal_inverse_cdf(p),
            Link::CLogLog => (-(F::one() - p).ln()).ln(),
            Link::Cauchit => (F::cast(std::f64::consts::PI) * (p - F::cast(0.5))).tan(),
        }
    }
}

/// Logistic CDF, evaluated on whichever side of zero avoids overflowing `exp`.
fn logistic_cdf<F: Float>(z: F) -> F {
    if z >= F::zero() {
        F::one() / (F::one() + (-z).exp())
    } else {
        let e = z.exp();
        e / (F::one() + e)
    }
}

/// Standard normal density.
fn normal_pdf<F: Float>(z: F) -> F {
    let inv_sqrt_2pi = F::cast(0.398_942_280_401_432_7);
    inv_sqrt_2pi * (-F::cast(0.5) * z * z).exp()
}

/// Standard normal CDF via Zelen & Severo (Abramowitz & Stegun 26.2.17).
///
/// Absolute error below 7.5e-8, which is far tighter than the optimizer's tolerance. Rolling our
/// own avoids a dependency on an error-function crate for one call site.
fn normal_cdf<F: Float>(z: F) -> F {
    let abs_z = z.abs();
    if abs_z > F::cast(40.0) {
        return if z > F::zero() { F::one() } else { F::zero() };
    }

    let t = F::one() / (F::one() + F::cast(0.231_641_9) * abs_z);
    let b1 = F::cast(0.319_381_530);
    let b2 = F::cast(-0.356_563_782);
    let b3 = F::cast(1.781_477_937);
    let b4 = F::cast(-1.821_255_978);
    let b5 = F::cast(1.330_274_429);

    let poly = t * (b1 + t * (b2 + t * (b3 + t * (b4 + t * b5))));
    let upper_tail = normal_pdf(abs_z) * poly;

    if z >= F::zero() {
        F::one() - upper_tail
    } else {
        upper_tail
    }
}

/// Inverse standard normal CDF via Acklam's rational approximation.
///
/// Relative error below 1.15e-9 over the open unit interval, which is ample for seeding thresholds.
fn normal_inverse_cdf<F: Float>(p: F) -> F {
    let a = [
        F::cast(-3.969_683_028_665_376e1),
        F::cast(2.209_460_984_245_205e2),
        F::cast(-2.759_285_104_469_687e2),
        F::cast(1.383_577_518_672_69e2),
        F::cast(-3.066_479_806_614_716e1),
        F::cast(2.506_628_277_459_239),
    ];
    let b = [
        F::cast(-5.447_609_879_822_406e1),
        F::cast(1.615_858_368_580_409e2),
        F::cast(-1.556_989_798_598_866e2),
        F::cast(6.680_131_188_771_972e1),
        F::cast(-1.328_068_155_288_572e1),
    ];
    let c = [
        F::cast(-7.784_894_002_430_293e-3),
        F::cast(-3.223_964_580_411_365e-1),
        F::cast(-2.400_758_277_161_838),
        F::cast(-2.549_732_539_343_734),
        F::cast(4.374_664_141_464_968),
        F::cast(2.938_163_982_698_783),
    ];
    let d = [
        F::cast(7.784_695_709_041_462e-3),
        F::cast(3.224_671_290_700_398e-1),
        F::cast(2.445_134_137_142_996),
        F::cast(3.754_408_661_907_416),
    ];

    let low = F::cast(0.02425);
    let high = F::one() - low;

    if p < low {
        let q = (-F::cast(2.0) * p.ln()).sqrt();
        (((((c[0] * q + c[1]) * q + c[2]) * q + c[3]) * q + c[4]) * q + c[5])
            / ((((d[0] * q + d[1]) * q + d[2]) * q + d[3]) * q + F::one())
    } else if p <= high {
        let q = p - F::cast(0.5);
        let r = q * q;
        (((((a[0] * r + a[1]) * r + a[2]) * r + a[3]) * r + a[4]) * r + a[5]) * q
            / (((((b[0] * r + b[1]) * r + b[2]) * r + b[3]) * r + b[4]) * r + F::one())
    } else {
        let q = (-F::cast(2.0) * (F::one() - p).ln()).sqrt();
        -(((((c[0] * q + c[1]) * q + c[2]) * q + c[3]) * q + c[4]) * q + c[5])
            / ((((d[0] * q + d[1]) * q + d[2]) * q + d[3]) * q + F::one())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LINKS: [Link; 4] = [Link::Logit, Link::Probit, Link::CLogLog, Link::Cauchit];

    #[test]
    fn cdfs_are_monotone_and_bounded() {
        for link in LINKS {
            let mut previous = f64::NEG_INFINITY;
            for step in -100..=100 {
                let z = step as f64 / 10.0;
                let p = link.cdf(z);
                assert!((0.0..=1.0).contains(&p), "{link:?} cdf({z}) = {p}");
                assert!(p >= previous - 1e-12, "{link:?} not monotone at {z}");
                previous = p;
            }
        }
    }

    #[test]
    fn cdfs_stay_finite_at_extremes() {
        for link in LINKS {
            for z in [-1e6, -100.0, 100.0, 1e6] {
                let p: f64 = link.cdf(z);
                assert!(p.is_finite(), "{link:?} cdf({z}) = {p}");
                assert!(link.pdf::<f64>(z).is_finite(), "{link:?} pdf({z})");
                assert!(link.pdf::<f64>(z) >= 0.0, "{link:?} pdf negative at {z}");
            }
        }
    }

    /// The density must be the derivative of the CDF, or every gradient in the model is wrong.
    ///
    /// The step is deliberately coarse. `normal_cdf` is a rational approximation with ~7.5e-8
    /// absolute error, and a central difference divides that error by `2 * step` — at `step = 1e-6`
    /// the approximation noise alone would swamp the comparison at ~4e-2. A larger step trades a
    /// little truncation error for far less amplification.
    #[test]
    fn pdf_matches_the_derivative_of_the_cdf() {
        let step = 1e-3;
        for link in LINKS {
            for z in [-2.0, -0.5, 0.0, 0.5, 2.0] {
                let numeric = (link.cdf(z + step) - link.cdf(z - step)) / (2.0 * step);
                let analytic: f64 = link.pdf(z);
                assert!(
                    (numeric - analytic).abs() < 1e-3,
                    "{link:?} at {z}: analytic {analytic} vs numeric {numeric}"
                );
            }
        }
    }

    #[test]
    fn inverse_cdf_round_trips() {
        for link in LINKS {
            for p in [0.001, 0.05, 0.25, 0.5, 0.75, 0.95, 0.999] {
                let z: f64 = link.inverse_cdf(p);
                let recovered = link.cdf(z);
                assert!(
                    (recovered - p).abs() < 1e-6,
                    "{link:?}: cdf(inverse_cdf({p})) = {recovered}"
                );
            }
        }
    }

    /// Anchor the normal CDF against values that are known to several decimals, so a transcription
    /// slip in the rational coefficients cannot pass unnoticed.
    #[test]
    fn normal_cdf_matches_known_values() {
        let cases = [
            (0.0f64, 0.5),
            (1.0, 0.841_344_746),
            (-1.0, 0.158_655_254),
            (1.96, 0.975_002_105),
            (-2.58, 0.004_940_016),
        ];
        for (z, expected) in cases {
            let actual: f64 = normal_cdf(z);
            assert!(
                (actual - expected).abs() < 1e-7,
                "normal_cdf({z}) = {actual}, expected {expected}"
            );
        }
    }

    #[test]
    fn works_at_f32() {
        for link in LINKS {
            let p: f32 = link.cdf(0.5f32);
            assert!(p.is_finite() && (0.0..=1.0).contains(&p));
            assert!(link.pdf(0.5f32).is_finite());
        }
    }
}
