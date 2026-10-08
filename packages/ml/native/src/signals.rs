use flow_like_ml_core::{Result, require};
use rustfft::{FftPlanner, num_complex::Complex};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct SpectrumBin {
    pub frequency_hz: f64,
    pub amplitude: f64,
    pub power: f64,
}

pub fn real_spectrum(values: &[f64], sample_rate_hz: f64) -> Result<Vec<SpectrumBin>> {
    require(
        values.len() >= 2 && values.iter().all(|v| v.is_finite()),
        "FFT needs at least two finite samples",
    )?;
    require(
        sample_rate_hz.is_finite() && sample_rate_hz > 0.0,
        "Sample rate must be finite and positive",
    )?;
    let n = values.len();
    let mut buffer: Vec<_> = values.iter().map(|v| Complex::new(*v, 0.0)).collect();
    FftPlanner::new().plan_fft_forward(n).process(&mut buffer);
    Ok(buffer[..=n / 2]
        .iter()
        .enumerate()
        .map(|(k, v)| {
            let factor = if k == 0 || n.is_multiple_of(2) && k == n / 2 {
                1.0
            } else {
                2.0
            };
            SpectrumBin {
                frequency_hz: k as f64 * sample_rate_hz / n as f64,
                amplitude: v.norm() * factor / n as f64,
                power: v.norm_sqr() * factor / (n as f64).powi(2),
            }
        })
        .collect())
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct WindowFeatures {
    pub mean: f64,
    pub standard_deviation: f64,
    pub rms: f64,
    pub peak_to_peak: f64,
    pub crest_factor: f64,
    pub skewness: f64,
    pub excess_kurtosis: f64,
    pub dominant_frequency_hz: f64,
    pub spectral_centroid_hz: f64,
}

impl WindowFeatures {
    pub fn as_vector(&self) -> Vec<f64> {
        vec![
            self.mean,
            self.standard_deviation,
            self.rms,
            self.peak_to_peak,
            self.crest_factor,
            self.skewness,
            self.excess_kurtosis,
            self.dominant_frequency_hz,
            self.spectral_centroid_hz,
        ]
    }
}

pub fn window_features(values: &[f64], sample_rate_hz: f64) -> Result<WindowFeatures> {
    let spectrum = real_spectrum(values, sample_rate_hz)?;
    let count = values.len() as f64;
    let mean = values.iter().sum::<f64>() / count;
    let variance = values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / count;
    let std = variance.sqrt();
    let rms = (values.iter().map(|v| v * v).sum::<f64>() / count).sqrt();
    let min = values.iter().copied().fold(f64::INFINITY, f64::min);
    let max = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let peak = min.abs().max(max.abs());
    let skewness = if std > 0.0 {
        values
            .iter()
            .map(|v| ((v - mean) / std).powi(3))
            .sum::<f64>()
            / count
    } else {
        0.0
    };
    let kurtosis = if std > 0.0 {
        values
            .iter()
            .map(|v| ((v - mean) / std).powi(4))
            .sum::<f64>()
            / count
            - 3.0
    } else {
        0.0
    };
    let total_power = spectrum.iter().skip(1).map(|v| v.power).sum::<f64>();
    let dominant = spectrum
        .iter()
        .skip(1)
        .max_by(|a, b| a.power.total_cmp(&b.power))
        .unwrap();
    Ok(WindowFeatures {
        mean,
        standard_deviation: std,
        rms,
        peak_to_peak: max - min,
        crest_factor: if rms > 0.0 { peak / rms } else { 0.0 },
        skewness,
        excess_kurtosis: kurtosis,
        dominant_frequency_hz: if total_power > f64::EPSILON {
            dominant.frequency_hz
        } else {
            0.0
        },
        spectral_centroid_hz: if total_power > 0.0 {
            spectrum
                .iter()
                .skip(1)
                .map(|v| v.frequency_hz * v.power)
                .sum::<f64>()
                / total_power
        } else {
            0.0
        },
    })
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct TimedValue {
    pub timestamp_ms: i64,
    pub value: f64,
}

pub fn resample_linear(
    values: &[TimedValue],
    start_ms: i64,
    step_ms: u64,
    count: usize,
    max_gap_ms: u64,
) -> Result<Vec<f64>> {
    require(
        values.len() >= 2 && step_ms > 0 && count > 0,
        "Resampling requires two values, positive step and count",
    )?;
    require(
        values.iter().all(|v| v.value.is_finite())
            && values
                .windows(2)
                .all(|w| w[0].timestamp_ms < w[1].timestamp_ms),
        "Timestamps must strictly increase and values must be finite",
    )?;
    let mut result = Vec::with_capacity(count);
    let mut right = 1;
    for i in 0..count {
        let timestamp = (start_ms as i128) + (step_ms as i128) * (i as i128);
        require(
            timestamp >= values[0].timestamp_ms as i128
                && timestamp <= values.last().unwrap().timestamp_ms as i128,
            "Cannot extrapolate outside the observed sensor interval",
        )?;
        while right < values.len() - 1 && (values[right].timestamp_ms as i128) < timestamp {
            right += 1;
        }
        let left = &values[right - 1];
        let right = &values[right];
        let gap = right.timestamp_ms as i128 - left.timestamp_ms as i128;
        require(
            gap <= max_gap_ms as i128,
            "Sensor gap exceeds interpolation limit",
        )?;
        let t = (timestamp - left.timestamp_ms as i128) as f64 / gap as f64;
        result.push(left.value + t * (right.value - left.value));
    }
    Ok(result)
}

pub fn sliding_windows(values: &[f64], width: usize, stride: usize) -> Result<Vec<Vec<f64>>> {
    require(
        width > 0 && stride > 0 && values.iter().all(|v| v.is_finite()),
        "Window width and stride must be positive and values finite",
    )?;
    Ok(values
        .windows(width)
        .step_by(stride)
        .map(<[f64]>::to_vec)
        .collect())
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WindowFunction {
    Rectangular,
    Hann,
    Hamming,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct Spectrogram {
    pub sample_rate_hz: f64,
    pub window_samples: usize,
    pub hop_samples: usize,
    pub time_seconds: Vec<f64>,
    pub frequency_hz: Vec<f64>,
    pub power: Vec<Vec<f64>>,
}

pub fn stft(
    values: &[f64],
    sample_rate_hz: f64,
    window_samples: usize,
    hop_samples: usize,
    window: WindowFunction,
) -> Result<Spectrogram> {
    require(
        window_samples >= 2 && hop_samples > 0 && values.len() >= window_samples,
        "STFT requires a complete window of at least two samples and a positive hop",
    )?;
    require(
        values.iter().all(|v| v.is_finite()),
        "STFT samples must be finite",
    )?;
    let weights: Vec<_> = (0..window_samples)
        .map(|i| {
            let phase = 2.0 * std::f64::consts::PI * i as f64 / (window_samples - 1) as f64;
            match window {
                WindowFunction::Rectangular => 1.0,
                WindowFunction::Hann => 0.5 - 0.5 * phase.cos(),
                WindowFunction::Hamming => 0.54 - 0.46 * phase.cos(),
            }
        })
        .collect();
    let mean_square = weights.iter().map(|w| w * w).sum::<f64>() / window_samples as f64;
    require(
        mean_square > 0.0,
        "Window has no energy; increase the STFT window size",
    )?;
    let mut power = Vec::new();
    let mut time_seconds = Vec::new();
    let mut frequency_hz = Vec::new();
    for (index, frame) in values
        .windows(window_samples)
        .step_by(hop_samples)
        .enumerate()
    {
        let weighted: Vec<_> = frame.iter().zip(&weights).map(|(v, w)| v * w).collect();
        let bins = real_spectrum(&weighted, sample_rate_hz)?;
        if frequency_hz.is_empty() {
            frequency_hz = bins.iter().map(|b| b.frequency_hz).collect();
        }
        power.push(bins.iter().map(|b| b.power / mean_square).collect());
        time_seconds.push(
            (index * hop_samples) as f64 / sample_rate_hz
                + (window_samples - 1) as f64 / (2.0 * sample_rate_hz),
        );
    }
    Ok(Spectrogram {
        sample_rate_hz,
        window_samples,
        hop_samples,
        time_seconds,
        frequency_hz,
        power,
    })
}

pub fn band_energy(values: &[f64], sample_rate_hz: f64, bands: &[(f64, f64)]) -> Result<Vec<f64>> {
    let bins = real_spectrum(values, sample_rate_hz)?;
    require(
        bands.iter().all(|(lo, hi)| {
            lo.is_finite() && hi.is_finite() && *lo >= 0.0 && lo < hi && *hi <= sample_rate_hz / 2.0
        }),
        "Frequency bands must increase within [0, Nyquist]",
    )?;
    Ok(bands
        .iter()
        .map(|(lo, hi)| {
            bins.iter()
                .filter(|bin| {
                    bin.frequency_hz >= *lo
                        && (bin.frequency_hz < *hi
                            || *hi == sample_rate_hz / 2.0 && bin.frequency_hz == *hi)
                })
                .map(|bin| bin.power)
                .sum()
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sine_fft_has_correct_peak_amplitude_and_energy() {
        let values: Vec<_> = (0..1024)
            .map(|i| (2.0 * std::f64::consts::PI * 64.0 * i as f64 / 1024.0).sin() * 2.0)
            .collect();
        let bins = real_spectrum(&values, 1024.0).unwrap();
        assert!((bins[64].amplitude - 2.0).abs() < 1e-10);
        assert!((bins.iter().map(|b| b.power).sum::<f64>() - 2.0).abs() < 1e-10);
        let features = window_features(&values, 1024.0).unwrap();
        assert_eq!(features.dominant_frequency_hz, 64.0);
        assert!((features.rms - 2.0f64.sqrt()).abs() < 1e-10);
    }
    #[test]
    fn resampling_rejects_gaps_and_extrapolation() {
        let values = [
            TimedValue {
                timestamp_ms: 0,
                value: 0.0,
            },
            TimedValue {
                timestamp_ms: 10,
                value: 10.0,
            },
        ];
        assert_eq!(
            resample_linear(&values, 0, 5, 3, 10).unwrap(),
            vec![0.0, 5.0, 10.0]
        );
        assert!(resample_linear(&values, 0, 5, 3, 9).is_err());
        assert!(resample_linear(&values, 0, 5, 4, 10).is_err());
    }

    #[test]
    fn stft_and_bands_separate_two_tones() {
        let values: Vec<_> = (0..2048)
            .map(|i| {
                let phase = 2.0 * std::f64::consts::PI * i as f64 / 1024.0;
                (phase * 64.0).sin() * 2.0 + (phase * 192.0).sin()
            })
            .collect();
        let energies = band_energy(&values, 1024.0, &[(32.0, 96.0), (128.0, 256.0)]).unwrap();
        assert!((energies[0] - 2.0).abs() < 1e-10);
        assert!((energies[1] - 0.5).abs() < 1e-10);
        let spectrum = stft(&values, 1024.0, 256, 128, WindowFunction::Hann).unwrap();
        assert_eq!(spectrum.power.len(), 15);
        for frame in &spectrum.power {
            assert!((frame.iter().sum::<f64>() - 2.5).abs() < 1e-4);
        }
    }
}
